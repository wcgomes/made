//! Performance budget tests with seeded measurements (no real timing).

use std::time::Duration;

use made_app::metrics::{
    BudgetStatus, Metrics, ACTIVE_REPAINT_INTERVAL, CWD_ROOT_BUDGET, FRAME_BUDGET,
    INPUT_LATENCY_BUDGET, MAX_FRAME_SAMPLES, SESSION_SWITCH_BUDGET, STARTUP_BUDGET, TARGET_FPS,
};

#[test]
fn budgets_match_the_product_targets() {
    assert_eq!(STARTUP_BUDGET, Duration::from_secs(2));
    assert_eq!(SESSION_SWITCH_BUDGET, Duration::from_millis(500));
    assert_eq!(INPUT_LATENCY_BUDGET, Duration::from_millis(50));
    assert_eq!(TARGET_FPS, 60);
    assert!(FRAME_BUDGET <= Duration::from_millis(17));
    assert!(FRAME_BUDGET.as_nanos() >= 16_666_666);
}

#[test]
fn active_repaint_interval_leaves_measured_overshoot_headroom_without_spinning() {
    // The request must be shorter than the 60 fps budget so the measured
    // wake-to-tick overshoot still lands inside 16.667 ms.
    assert!(ACTIVE_REPAINT_INTERVAL < FRAME_BUDGET);
    assert!(ACTIVE_REPAINT_INTERVAL >= Duration::from_millis(14));
    assert!(
        ACTIVE_REPAINT_INTERVAL >= Duration::from_millis(10),
        "a sub-10ms request would trade CPU for frame pacing with no user-visible gain"
    );
}

#[test]
fn empty_metrics_are_not_measured() {
    let report = Metrics::new().report();
    assert_eq!(report.status(), BudgetStatus::NotMeasured);
    assert!(!report.all_within_budget());
    assert_eq!(report.startup.status, BudgetStatus::NotMeasured);
    assert_eq!(report.session_switch.status, BudgetStatus::NotMeasured);
    assert_eq!(report.input_latency.status, BudgetStatus::NotMeasured);
    assert_eq!(report.frame_time.status, BudgetStatus::NotMeasured);
    assert!(report.over_budget().is_empty());
}

#[test]
fn seeded_measurements_within_budget_pass() {
    let mut metrics = Metrics::new();
    metrics.record_startup(Duration::from_millis(1_500));
    metrics.record_session_switch(Duration::from_millis(400));
    metrics.record_input_latency(Duration::from_millis(30));
    for frame in [10, 12, 14, 16] {
        metrics.record_frame(Duration::from_millis(frame));
    }

    let report = metrics.report();
    assert_eq!(report.startup.status, BudgetStatus::WithinBudget);
    assert_eq!(report.session_switch.status, BudgetStatus::WithinBudget);
    assert_eq!(report.input_latency.status, BudgetStatus::WithinBudget);
    assert_eq!(report.frame_time.status, BudgetStatus::WithinBudget);
    assert_eq!(report.status(), BudgetStatus::WithinBudget);
    assert!(report.all_within_budget());
    assert!(report.over_budget().is_empty());
    assert_eq!(report.frame_time.samples, 4);
    assert_eq!(report.frame_time.worst, Some(Duration::from_millis(16)));
    assert!(report.to_text().contains("within-budget"));
}

#[test]
fn seeded_measurements_over_budget_fail_with_names() {
    let mut metrics = Metrics::new();
    metrics.record_startup(Duration::from_millis(2_500));
    metrics.record_session_switch(Duration::from_millis(600));
    metrics.record_input_latency(Duration::from_millis(60));
    for _ in 0..10 {
        metrics.record_frame(Duration::from_millis(30));
    }

    let report = metrics.report();
    assert_eq!(report.status(), BudgetStatus::OverBudget);
    assert!(!report.all_within_budget());
    assert_eq!(
        report.over_budget(),
        vec!["startup", "session_switch", "input_latency", "frame_time"]
    );
    assert_eq!(report.startup.status, BudgetStatus::OverBudget);
    assert_eq!(report.frame_time.status, BudgetStatus::OverBudget);
    assert!(report.to_text().contains("over-budget"));
}

#[test]
fn exact_budget_boundaries_are_within_budget() {
    let mut metrics = Metrics::new();
    metrics.record_startup(STARTUP_BUDGET);
    metrics.record_session_switch(SESSION_SWITCH_BUDGET);
    metrics.record_input_latency(INPUT_LATENCY_BUDGET);
    metrics.record_frame(FRAME_BUDGET);
    let report = metrics.report();
    assert_eq!(report.status(), BudgetStatus::WithinBudget);
}

#[test]
fn a_single_over_budget_metric_makes_the_report_over_budget() {
    let mut metrics = Metrics::new();
    metrics.record_startup(Duration::from_millis(1_000));
    metrics.record_session_switch(Duration::from_millis(100));
    metrics.record_input_latency(Duration::from_millis(10));
    metrics.record_frame(Duration::from_millis(10));
    assert_eq!(metrics.report().status(), BudgetStatus::WithinBudget);

    metrics.record_input_latency(Duration::from_millis(51));
    assert_eq!(metrics.report().status(), BudgetStatus::OverBudget);
    assert_eq!(metrics.report().over_budget(), vec!["input_latency"]);
}

#[test]
fn frame_samples_are_bounded_and_keep_the_latest_window() {
    let mut metrics = Metrics::new();
    for index in 0..(MAX_FRAME_SAMPLES + 20) {
        let millis = if index >= 20 { 16 } else { 100 };
        metrics.record_frame(Duration::from_millis(millis as u64));
    }
    assert_eq!(metrics.frame_samples(), MAX_FRAME_SAMPLES);
    let report = metrics.report();
    assert_eq!(report.frame_time.samples, MAX_FRAME_SAMPLES);
    assert_eq!(report.frame_time.worst, Some(Duration::from_millis(16)));
    assert_eq!(report.frame_time.status, BudgetStatus::WithinBudget);
}

#[test]
fn reset_clears_every_measurement() {
    let mut metrics = Metrics::new();
    metrics.record_startup(Duration::from_millis(100));
    metrics.record_session_switch(Duration::from_millis(100));
    metrics.record_input_latency(Duration::from_millis(10));
    metrics.record_frame(Duration::from_millis(10));
    metrics.reset();

    assert!(metrics.startup().is_none());
    assert!(metrics.session_switch().is_none());
    assert!(metrics.input_latency().is_none());
    assert_eq!(metrics.frame_samples(), 0);
    assert_eq!(metrics.report().status(), BudgetStatus::NotMeasured);
}

#[test]
fn revised_scenario_instrumentation_is_exposed_and_scalar() {
    let mut metrics = Metrics::new();
    metrics.record_startup(Duration::from_millis(1_000));
    metrics.record_session_switch(Duration::from_millis(200));
    metrics.record_input_latency(Duration::from_millis(10));
    metrics.record_frame(Duration::from_millis(10));
    metrics.record_cwd_to_root(Duration::from_millis(300));
    metrics.record_activation(Duration::from_millis(120));
    metrics.record_preview_roundtrip(Duration::from_millis(80));
    metrics.record_explorer_layout(Duration::from_millis(5));
    for _ in 0..10_000 {
        metrics.record_hidden_output(64);
    }
    metrics.record_hidden_grid_paints(0);
    metrics.set_bridge_counters(3, 7);

    assert_eq!(metrics.cwd_to_root(), Some(Duration::from_millis(300)));
    assert_eq!(metrics.activation(), Some(Duration::from_millis(120)));
    assert_eq!(metrics.preview_roundtrip(), Some(Duration::from_millis(80)));
    assert_eq!(metrics.explorer_layout(), Some(Duration::from_millis(5)));
    assert_eq!(metrics.counters().hidden_output_bytes, 640_000);
    assert_eq!(metrics.counters().hidden_grid_paints, 0);
    assert_eq!(metrics.counters().bridge_dropped_events, 3);
    assert_eq!(metrics.counters().bridge_coalesced_states, 7);
    assert_eq!(metrics.counters().activations, 1);
    assert_eq!(metrics.counters().preview_roundtrips, 1);
    assert_eq!(metrics.counters().explorer_layout_changes, 1);
    assert_eq!(metrics.frame_samples(), 1, "counters do not add samples");

    let report = metrics.report();
    assert_eq!(report.status(), BudgetStatus::WithinBudget);
    assert_eq!(report.cwd_to_root.status, BudgetStatus::WithinBudget);
    assert_eq!(report.counters.hidden_output_bytes, 640_000);
    let text = report.to_text();
    for expected in [
        "cwd_to_root:",
        "activation:",
        "preview_roundtrip:",
        "explorer_layout:",
        "hidden_output_bytes=640000",
        "bridge_dropped_events=3",
        "bridge_coalesced_states=7",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
}

#[test]
fn cwd_to_root_over_budget_is_reported_without_masking_core_metrics() {
    assert_eq!(CWD_ROOT_BUDGET, SESSION_SWITCH_BUDGET);
    let mut metrics = Metrics::new();
    metrics.record_startup(Duration::from_millis(1_000));
    metrics.record_session_switch(Duration::from_millis(200));
    metrics.record_input_latency(Duration::from_millis(10));
    metrics.record_frame(Duration::from_millis(10));
    metrics.record_cwd_to_root(CWD_ROOT_BUDGET + Duration::from_millis(1));

    let report = metrics.report();
    assert_eq!(report.cwd_to_root.status, BudgetStatus::OverBudget);
    assert_eq!(report.status(), BudgetStatus::OverBudget);
    assert_eq!(report.over_budget(), vec!["cwd_to_root"]);
    assert!(report
        .to_text()
        .contains("cwd_to_root: 501.000ms over-budget"));
}
