//! Focus controller tests: correlation, in-flight state, failures, timeouts,
//! and state-refresh triggers. Timeouts are deterministic and short.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use made_app::session_focus::{
    FocusErrorKind, FocusOutcome, FocusSink, SessionFocusController, StateRefresh,
};
use made_bridge::server::{BridgeHandle, FocusResult};

#[derive(Default)]
struct FakeSink {
    focus: Mutex<VecDeque<Result<String, String>>>,
    state: Mutex<VecDeque<Result<String, String>>>,
    focus_calls: Mutex<Vec<String>>,
    state_calls: AtomicUsize,
}

impl FakeSink {
    fn new(focus: Vec<Result<String, String>>, state: Vec<Result<String, String>>) -> Self {
        Self {
            focus: Mutex::new(focus.into()),
            state: Mutex::new(state.into()),
            focus_calls: Mutex::new(Vec::new()),
            state_calls: AtomicUsize::new(0),
        }
    }

    fn focus_calls(&self) -> Vec<String> {
        self.focus_calls.lock().unwrap().clone()
    }

    fn state_calls(&self) -> usize {
        self.state_calls.load(Ordering::SeqCst)
    }
}

impl FocusSink for FakeSink {
    fn send_focus(&self, session_id: &str) -> Result<String, String> {
        self.focus_calls.lock().unwrap().push(session_id.to_owned());
        self.focus
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok("focus-default".to_owned()))
    }

    fn request_state(&self) -> Result<String, String> {
        self.state_calls.fetch_add(1, Ordering::SeqCst);
        self.state
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok("state-default".to_owned()))
    }
}

fn result(request_id: &str, session_id: &str, ok: bool, error: Option<&str>) -> FocusResult {
    FocusResult {
        request_id: request_id.to_owned(),
        session_id: Some(session_id.to_owned()),
        ok,
        error: error.map(str::to_owned),
    }
}

#[test]
fn request_correlates_by_id_and_success_clears_the_pending_state() {
    let sink = FakeSink::new(
        vec![Ok("focus-1".to_owned())],
        vec![Ok("state-1".to_owned())],
    );
    let mut controller = SessionFocusController::with_timeout(Duration::from_secs(1));

    let started = controller.request(&sink, "ses_a");
    assert_eq!(started.outcome, FocusOutcome::Started);
    assert_eq!(started.request_id.as_deref(), Some("focus-1"));
    assert_eq!(started.state_refresh, StateRefresh::NotNeeded);
    assert_eq!(controller.in_flight().unwrap().session_id, "ses_a");
    assert_eq!(sink.focus_calls(), vec!["ses_a"]);

    let finished = controller.handle_result(&sink, result("focus-1", "ses_a", true, None));
    assert_eq!(finished.outcome, FocusOutcome::Succeeded);
    assert!(finished.in_flight.is_none());
    assert!(controller.in_flight().is_none());
    assert!(controller.last_error().is_none());
    assert_eq!(sink.state_calls(), 0);
}

#[test]
fn a_second_request_while_pending_is_busy_and_does_not_send() {
    let sink = FakeSink::new(vec![Ok("focus-1".to_owned())], vec![]);
    let mut controller = SessionFocusController::new();

    controller.request(&sink, "ses_a");
    let second = controller.request(&sink, "ses_b");
    assert_eq!(second.outcome, FocusOutcome::Busy);
    assert_eq!(second.request_id.as_deref(), Some("focus-1"));
    assert_eq!(sink.focus_calls(), vec!["ses_a"]);
}

#[test]
fn failure_reports_a_non_blocking_error_and_triggers_a_state_refresh() {
    let sink = FakeSink::new(
        vec![Ok("focus-1".to_owned())],
        vec![Ok("state-refresh".to_owned())],
    );
    let mut controller = SessionFocusController::new();

    controller.request(&sink, "ses_missing");
    let failed = controller.handle_result(
        &sink,
        result(
            "focus-1",
            "ses_missing",
            false,
            Some("session no longer exists"),
        ),
    );

    assert_eq!(failed.outcome, FocusOutcome::Failed);
    assert_eq!(
        failed.state_refresh,
        StateRefresh::Requested("state-refresh".to_owned())
    );
    assert_eq!(sink.state_calls(), 1);
    assert!(failed.in_flight.is_none());
    let error = controller.last_error().expect("failure is retained");
    assert_eq!(error.kind, FocusErrorKind::Failed);
    assert_eq!(error.session_id.as_deref(), Some("ses_missing"));
    assert!(error.message.contains("no longer exists"));
}

#[test]
fn timeout_is_checked_against_an_explicit_instant() {
    let sink = FakeSink::new(
        vec![Ok("focus-timeout".to_owned())],
        vec![Ok("state-timeout".to_owned())],
    );
    let mut controller = SessionFocusController::with_timeout(Duration::from_millis(20));
    let start = Instant::now();

    controller.request_at(&sink, "ses_slow", start);
    let early = controller.poll_timeout(&sink, start + Duration::from_millis(10));
    assert_eq!(early.outcome, FocusOutcome::Idle);
    assert!(early.is_pending());
    assert_eq!(sink.state_calls(), 0);

    let timed_out = controller.poll_timeout(&sink, start + Duration::from_millis(30));
    assert_eq!(timed_out.outcome, FocusOutcome::TimedOut);
    assert_eq!(
        timed_out.state_refresh,
        StateRefresh::Requested("state-timeout".to_owned())
    );
    assert_eq!(
        controller.last_error().unwrap().kind,
        FocusErrorKind::TimedOut
    );
    assert!(controller.in_flight().is_none());
    assert_eq!(sink.state_calls(), 1);
}

#[test]
fn a_short_real_timeout_is_non_blocking_and_under_the_test_budget() {
    let sink = FakeSink::new(
        vec![Ok("focus-short".to_owned())],
        vec![Ok("state-short".to_owned())],
    );
    let mut controller = SessionFocusController::with_timeout(Duration::from_millis(10));
    controller.request(&sink, "ses_short");
    std::thread::sleep(Duration::from_millis(20));
    let timed_out = controller.tick(&sink);
    assert_eq!(timed_out.outcome, FocusOutcome::TimedOut);
    assert_eq!(sink.state_calls(), 1);
}

#[test]
fn an_uncorrelated_result_leaves_the_request_pending() {
    let sink = FakeSink::new(vec![Ok("focus-1".to_owned())], vec![]);
    let mut controller = SessionFocusController::new();
    controller.request(&sink, "ses_a");

    let other = controller.handle_result(&sink, result("focus-other", "ses_a", true, None));
    assert_eq!(other.outcome, FocusOutcome::Uncorrelated);
    assert!(other.is_pending());
    assert!(controller.in_flight().is_some());
    assert_eq!(sink.state_calls(), 0);

    let matching = controller.handle_result(&sink, result("focus-1", "ses_a", true, None));
    assert_eq!(matching.outcome, FocusOutcome::Succeeded);
}

#[test]
fn a_sink_failure_triggers_a_state_refresh() {
    let sink = FakeSink::new(
        vec![Err("bridge is closed".to_owned())],
        vec![Ok("state-after-send-failure".to_owned())],
    );
    let mut controller = SessionFocusController::new();

    let update = controller.request(&sink, "ses_a");
    assert_eq!(update.outcome, FocusOutcome::SendFailed);
    assert_eq!(
        update.state_refresh,
        StateRefresh::Requested("state-after-send-failure".to_owned())
    );
    assert_eq!(sink.state_calls(), 1);
    assert_eq!(
        controller.last_error().unwrap().kind,
        FocusErrorKind::SendFailed
    );
}

#[test]
fn a_failed_refresh_is_reported_without_hiding_the_focus_error() {
    let sink = FakeSink::new(
        vec![Ok("focus-1".to_owned())],
        vec![Err("still closed".to_owned())],
    );
    let mut controller = SessionFocusController::new();
    controller.request(&sink, "ses_a");
    let failed = controller.handle_result(&sink, result("focus-1", "ses_a", false, None));

    assert_eq!(failed.outcome, FocusOutcome::Failed);
    assert_eq!(
        failed.state_refresh,
        StateRefresh::Failed("still closed".to_owned())
    );
    assert!(failed.error.is_some());
    assert!(controller.last_error().is_some());
}

#[test]
fn success_after_a_failure_clears_the_stored_error() {
    let sink = FakeSink::new(
        vec![Ok("focus-1".to_owned()), Ok("focus-2".to_owned())],
        vec![Ok("state-1".to_owned())],
    );
    let mut controller = SessionFocusController::new();

    controller.request(&sink, "ses_a");
    controller.handle_result(&sink, result("focus-1", "ses_a", false, Some("gone")));
    assert!(controller.last_error().is_some());

    controller.request(&sink, "ses_b");
    let succeeded = controller.handle_result(&sink, result("focus-2", "ses_b", true, None));
    assert_eq!(succeeded.outcome, FocusOutcome::Succeeded);
    assert!(controller.last_error().is_none());
}

#[test]
fn cancelling_a_pending_request_does_not_touch_the_bridge() {
    let sink = FakeSink::new(vec![Ok("focus-1".to_owned())], vec![]);
    let mut controller = SessionFocusController::new();
    controller.request(&sink, "ses_a");

    let error = controller.cancel().expect("pending request cancels");
    assert_eq!(error.kind, FocusErrorKind::Cancelled);
    assert!(controller.in_flight().is_none());
    assert_eq!(sink.state_calls(), 0);
    assert!(controller.cancel().is_none());
}

#[test]
fn the_production_bridge_handle_implements_the_focus_sink() {
    fn assert_sink<T: FocusSink>() {}
    assert_sink::<BridgeHandle>();
}
