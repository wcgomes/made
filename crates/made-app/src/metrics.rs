//! Performance measurements and budget checks.
//!
//! Budgets are product targets from the change design: startup to interactive
//! terminal and explorer within 2 seconds, active-session switch within 500 ms,
//! terminal input-to-render within 50 ms, and sustained rendering at 60 frames
//! per second. Measurements are seeded by the app (or by tests); this module
//! performs no timing itself, so classifications are deterministic.

use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

/// Startup budget: interactive terminal and browsable explorer.
pub const STARTUP_BUDGET: Duration = Duration::from_secs(2);

/// Active-session change to explorer update budget.
pub const SESSION_SWITCH_BUDGET: Duration = Duration::from_millis(500);

/// Terminal input-to-render budget.
pub const INPUT_LATENCY_BUDGET: Duration = Duration::from_millis(50);

/// Target sustained frame rate.
pub const TARGET_FPS: u32 = 60;

/// Frame budget for 60 fps.
pub const FRAME_BUDGET: Duration = Duration::from_nanos(16_666_667);

/// Repaint request interval while the selected workspace is active.
///
/// The product target is 16 ms per frame (60 fps), but a request for exactly
/// 16 ms lands one event-loop/compositor scheduling hop late. Measured
/// release builds under Xvfb/llvmpipe showed a 0.4-1.2 ms wake-to-tick
/// overshoot in a single tab and up to ~1.8 ms with two hidden output tabs
/// sharing the tick. A 15 ms request still missed the 16.667 ms budget in the
/// hidden-three-tab case (p95 16.85 ms), so the deadline is calibrated to
/// 14 ms: 16.667 ms minus the observed worst overshoot (about 1.8 ms) and a
/// small scheduling margin.
///
/// This is still bounded and cheap: 71.4 wakeups/s instead of 62.5 at 16 ms,
/// with the UI thread idle between deadlines. Terminal input is written
/// immediately; the PTY pump and input cadence are untouched.
pub const ACTIVE_REPAINT_INTERVAL: Duration = Duration::from_millis(14);

/// Maximum retained frame samples.
pub const MAX_FRAME_SAMPLES: usize = 240;

/// Shell cwd report to explorer root update budget. The product target for a
/// reported cwd or active-session change is the same 500 ms window.
pub const CWD_ROOT_BUDGET: Duration = SESSION_SWITCH_BUDGET;

/// Classification against a budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetStatus {
    /// No measurement was recorded.
    NotMeasured,
    /// The measurement is within budget.
    WithinBudget,
    /// The measurement exceeds budget.
    OverBudget,
}

impl BudgetStatus {
    /// Whether the measurement is within budget.
    pub const fn is_within(self) -> bool {
        matches!(self, Self::WithinBudget)
    }

    /// Whether the measurement is over budget.
    pub const fn is_over(self) -> bool {
        matches!(self, Self::OverBudget)
    }

    /// Stable label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::NotMeasured => "not-measured",
            Self::WithinBudget => "within-budget",
            Self::OverBudget => "over-budget",
        }
    }

    fn classify(measured: Option<Duration>, budget: Duration) -> Self {
        match measured {
            None => Self::NotMeasured,
            Some(measured) if measured <= budget => Self::WithinBudget,
            Some(_) => Self::OverBudget,
        }
    }
}

impl fmt::Display for BudgetStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// One scalar budget check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricCheck {
    /// Metric name.
    pub name: &'static str,
    /// Measured value.
    pub measured: Option<Duration>,
    /// Budget.
    pub budget: Duration,
    /// Classification.
    pub status: BudgetStatus,
}

impl MetricCheck {
    /// Build a check from an optional measurement.
    pub fn new(name: &'static str, measured: Option<Duration>, budget: Duration) -> Self {
        Self {
            name,
            measured,
            budget,
            status: BudgetStatus::classify(measured, budget),
        }
    }
}

/// Frame-time statistics and classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameTimeReport {
    /// Number of retained samples.
    pub samples: usize,
    /// Mean frame time.
    pub average: Option<Duration>,
    /// 95th percentile frame time.
    pub p95: Option<Duration>,
    /// Worst frame time.
    pub worst: Option<Duration>,
    /// Frame budget.
    pub budget: Duration,
    /// Classification based on the 95th percentile.
    pub status: BudgetStatus,
}

impl FrameTimeReport {
    fn from_samples(samples: &VecDeque<Duration>) -> Self {
        if samples.is_empty() {
            return Self {
                samples: 0,
                average: None,
                p95: None,
                worst: None,
                budget: FRAME_BUDGET,
                status: BudgetStatus::NotMeasured,
            };
        }

        let mut sorted: Vec<Duration> = samples.iter().copied().collect();
        sorted.sort_unstable();
        let total: Duration = sorted.iter().copied().sum();
        let average = total / u32::try_from(sorted.len()).unwrap_or(u32::MAX);
        let index = ((95 * sorted.len()).div_ceil(100))
            .saturating_sub(1)
            .min(sorted.len() - 1);
        let p95 = sorted[index];
        let worst = *sorted
            .last()
            .expect("a nonempty sample set has a worst value");
        let status = BudgetStatus::classify(Some(p95), FRAME_BUDGET);

        Self {
            samples: sorted.len(),
            average: Some(average),
            p95: Some(p95),
            worst: Some(worst),
            budget: FRAME_BUDGET,
            status,
        }
    }
}

/// Bounded runtime counters for the revised terminal-first scenarios.
///
/// Every field is a scalar updated in place. Hidden output and dropped/coalesced
/// events are accumulated without retaining per-frame samples or allocating on
/// the input path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeCounters {
    /// Output bytes consumed while the owning workspace/pane was not visible.
    pub hidden_output_bytes: u64,
    /// Grid paints observed while the terminal was expected to stay hidden.
    pub hidden_grid_paints: u64,
    /// Bridge events dropped under bounded-queue pressure.
    pub bridge_dropped_events: u64,
    /// State snapshots coalesced before the app consumed them.
    pub bridge_coalesced_states: u64,
    /// Workspace activations measured to first paint.
    pub activations: u64,
    /// Completed preview roundtrips measured to terminal paint.
    pub preview_roundtrips: u64,
    /// Explorer hide/resize layout changes measured to the next paint.
    pub explorer_layout_changes: u64,
}

/// Full performance report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerformanceReport {
    /// Startup check.
    pub startup: MetricCheck,
    /// Session-switch check.
    pub session_switch: MetricCheck,
    /// Input-latency check.
    pub input_latency: MetricCheck,
    /// Frame-time report.
    pub frame_time: FrameTimeReport,
    /// Shell cwd report to explorer root update check.
    pub cwd_to_root: MetricCheck,
    /// Latest workspace activation to first paint, when measured.
    pub activation: Option<Duration>,
    /// Latest preview roundtrip (open plus return to terminal paint).
    pub preview_roundtrip: Option<Duration>,
    /// Latest explorer hide/resize to next paint.
    pub explorer_layout: Option<Duration>,
    /// Bounded runtime counters.
    pub counters: RuntimeCounters,
}

impl PerformanceReport {
    /// Overall classification: over budget if any tracked budget is over, not
    /// measured if a core metric is missing, otherwise within budget. The
    /// revised cwd-to-root metric only affects the result once measured.
    pub fn status(&self) -> BudgetStatus {
        let checks = [
            self.startup.status,
            self.session_switch.status,
            self.input_latency.status,
            self.frame_time.status,
        ];
        if checks.iter().any(|status| status.is_over()) || self.cwd_to_root.status.is_over() {
            BudgetStatus::OverBudget
        } else if checks.iter().any(|status| !status.is_within()) {
            BudgetStatus::NotMeasured
        } else {
            BudgetStatus::WithinBudget
        }
    }

    /// Whether every budget is measured and within budget.
    pub fn all_within_budget(&self) -> bool {
        self.status().is_within()
    }

    /// Names of metrics that exceed budget.
    pub fn over_budget(&self) -> Vec<&'static str> {
        let mut over = Vec::new();
        for check in [&self.startup, &self.session_switch, &self.input_latency] {
            if check.status.is_over() {
                over.push(check.name);
            }
        }
        if self.frame_time.status.is_over() {
            over.push("frame_time");
        }
        if self.cwd_to_root.status.is_over() {
            over.push("cwd_to_root");
        }
        over
    }

    /// Human-readable summary for logs.
    pub fn to_text(&self) -> String {
        let mut text = String::new();
        for check in [&self.startup, &self.session_switch, &self.input_latency] {
            text.push_str(&format!(
                "{}: {} {} (budget {})\n",
                check.name,
                check
                    .measured
                    .map(|value| format!("{:.3}ms", value.as_secs_f64() * 1000.0))
                    .unwrap_or_else(|| "not measured".to_owned()),
                check.status.label(),
                check.budget.as_millis()
            ));
        }
        text.push_str(&format!(
            "frame_time: p95 {} (budget {:.3}ms, {} samples, {})\n",
            self.frame_time
                .p95
                .map(|value| format!("{:.3}ms", value.as_secs_f64() * 1000.0))
                .unwrap_or_else(|| "not measured".to_owned()),
            self.frame_time.budget.as_secs_f64() * 1000.0,
            self.frame_time.samples,
            self.frame_time.status.label()
        ));
        text.push_str(&format!(
            "cwd_to_root: {} {} (budget {})\n",
            display_duration(self.cwd_to_root.measured),
            self.cwd_to_root.status.label(),
            self.cwd_to_root.budget.as_millis()
        ));
        text.push_str(&format!(
            "activation: {}\n",
            display_duration(self.activation)
        ));
        text.push_str(&format!(
            "preview_roundtrip: {}\n",
            display_duration(self.preview_roundtrip)
        ));
        text.push_str(&format!(
            "explorer_layout: {}\n",
            display_duration(self.explorer_layout)
        ));
        text.push_str(&format!(
            "counters: hidden_output_bytes={} hidden_grid_paints={} bridge_dropped_events={} bridge_coalesced_states={} activations={} preview_roundtrips={} explorer_layout_changes={}\n",
            self.counters.hidden_output_bytes,
            self.counters.hidden_grid_paints,
            self.counters.bridge_dropped_events,
            self.counters.bridge_coalesced_states,
            self.counters.activations,
            self.counters.preview_roundtrips,
            self.counters.explorer_layout_changes
        ));
        text
    }
}

fn display_duration(value: Option<Duration>) -> String {
    value
        .map(|value| format!("{:.3}ms", value.as_secs_f64() * 1000.0))
        .unwrap_or_else(|| "not measured".to_owned())
}

/// Accumulates measurements for one app session.
#[derive(Debug, Clone)]
pub struct Metrics {
    startup: Option<Duration>,
    session_switch: Option<Duration>,
    input_latency: Option<Duration>,
    cwd_to_root: Option<Duration>,
    activation: Option<Duration>,
    preview_roundtrip: Option<Duration>,
    explorer_layout: Option<Duration>,
    counters: RuntimeCounters,
    frames: VecDeque<Duration>,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    /// Empty metrics.
    pub fn new() -> Self {
        Self {
            startup: None,
            session_switch: None,
            input_latency: None,
            cwd_to_root: None,
            activation: None,
            preview_roundtrip: None,
            explorer_layout: None,
            counters: RuntimeCounters::default(),
            frames: VecDeque::with_capacity(MAX_FRAME_SAMPLES),
        }
    }

    /// Record startup-to-interactive time.
    pub fn record_startup(&mut self, elapsed: Duration) {
        self.startup = Some(elapsed);
    }

    /// Record active-session change to explorer update time.
    pub fn record_session_switch(&mut self, elapsed: Duration) {
        self.session_switch = Some(elapsed);
    }

    /// Record a shell cwd report to explorer root update time.
    pub fn record_cwd_to_root(&mut self, elapsed: Duration) {
        self.cwd_to_root = Some(elapsed);
    }

    /// Record workspace activation to the first visible paint.
    pub fn record_activation(&mut self, elapsed: Duration) {
        self.activation = Some(elapsed);
        self.counters.activations = self.counters.activations.saturating_add(1);
    }

    /// Record a preview open plus return-to-terminal paint roundtrip.
    pub fn record_preview_roundtrip(&mut self, elapsed: Duration) {
        self.preview_roundtrip = Some(elapsed);
        self.counters.preview_roundtrips = self.counters.preview_roundtrips.saturating_add(1);
    }

    /// Record an explorer hide/resize action to the next visible paint.
    pub fn record_explorer_layout(&mut self, elapsed: Duration) {
        self.explorer_layout = Some(elapsed);
        self.counters.explorer_layout_changes =
            self.counters.explorer_layout_changes.saturating_add(1);
    }

    /// Record terminal input-to-render time.
    pub fn record_input_latency(&mut self, elapsed: Duration) {
        self.input_latency = Some(elapsed);
    }

    /// Add output consumed while the terminal was hidden. Scalar, bounded.
    pub fn record_hidden_output(&mut self, bytes: u64) {
        self.counters.hidden_output_bytes = self.counters.hidden_output_bytes.saturating_add(bytes);
    }

    /// Add grid paints observed while the terminal should have stayed hidden.
    pub fn record_hidden_grid_paints(&mut self, paints: u64) {
        self.counters.hidden_grid_paints = self.counters.hidden_grid_paints.saturating_add(paints);
    }

    /// Store the latest bounded bridge queue counters.
    pub fn set_bridge_counters(&mut self, dropped_events: u64, coalesced_states: u64) {
        self.counters.bridge_dropped_events = dropped_events;
        self.counters.bridge_coalesced_states = coalesced_states;
    }

    /// Record one rendered frame interval.
    pub fn record_frame(&mut self, elapsed: Duration) {
        if self.frames.len() == MAX_FRAME_SAMPLES {
            self.frames.pop_front();
        }
        self.frames.push_back(elapsed);
    }

    /// Latest startup measurement.
    pub const fn startup(&self) -> Option<Duration> {
        self.startup
    }

    /// Latest session-switch measurement.
    pub const fn session_switch(&self) -> Option<Duration> {
        self.session_switch
    }

    /// Latest shell-cwd-to-root measurement.
    pub const fn cwd_to_root(&self) -> Option<Duration> {
        self.cwd_to_root
    }

    /// Latest workspace activation measurement.
    pub const fn activation(&self) -> Option<Duration> {
        self.activation
    }

    /// Latest preview roundtrip measurement.
    pub const fn preview_roundtrip(&self) -> Option<Duration> {
        self.preview_roundtrip
    }

    /// Latest explorer hide/resize measurement.
    pub const fn explorer_layout(&self) -> Option<Duration> {
        self.explorer_layout
    }

    /// Bounded runtime counters.
    pub const fn counters(&self) -> &RuntimeCounters {
        &self.counters
    }

    /// Latest input-latency measurement.
    pub const fn input_latency(&self) -> Option<Duration> {
        self.input_latency
    }

    /// Number of retained frame samples.
    pub fn frame_samples(&self) -> usize {
        self.frames.len()
    }

    /// Frame samples, oldest to newest.
    pub fn frames(&self) -> impl DoubleEndedIterator<Item = Duration> + ExactSizeIterator + '_ {
        self.frames.iter().copied()
    }

    /// Build the report without changing the samples.
    pub fn report(&self) -> PerformanceReport {
        PerformanceReport {
            startup: MetricCheck::new("startup", self.startup, STARTUP_BUDGET),
            session_switch: MetricCheck::new(
                "session_switch",
                self.session_switch,
                SESSION_SWITCH_BUDGET,
            ),
            input_latency: MetricCheck::new(
                "input_latency",
                self.input_latency,
                INPUT_LATENCY_BUDGET,
            ),
            frame_time: FrameTimeReport::from_samples(&self.frames),
            cwd_to_root: MetricCheck::new("cwd_to_root", self.cwd_to_root, CWD_ROOT_BUDGET),
            activation: self.activation,
            preview_roundtrip: self.preview_roundtrip,
            explorer_layout: self.explorer_layout,
            counters: self.counters,
        }
    }

    /// Drop every measurement.
    pub fn reset(&mut self) {
        self.startup = None;
        self.session_switch = None;
        self.input_latency = None;
        self.cwd_to_root = None;
        self.activation = None;
        self.preview_roundtrip = None;
        self.explorer_layout = None;
        self.counters = RuntimeCounters::default();
        self.frames.clear();
    }
}
