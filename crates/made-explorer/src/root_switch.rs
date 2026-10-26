//! Root generation tracking and stale load/event rejection (Phase 2, tasks
//! 2.15-2.16).
//!
//! Every tree load and filesystem event carries the generation of the root it
//! belongs to. Events from a previous root cannot mutate the new tree.
//!
//! A generation is an opaque monotonic token. `RootGeneration::next` produces a
//! token that compares greater than its predecessor; only an exact match with
//! the current token is accepted. Tokens are deliberately independent of app
//! types and can be created by `made-app` when it switches workspace roots.

use std::error::Error;
use std::fmt;

/// Opaque monotonic token identifying one active workspace root.
///
/// The initial token is available as [`RootGeneration::INITIAL`]. Tokens are
/// ordered, so a consumer can also detect a stale token with [`RootGeneration::is_stale`]
/// or reject it with [`GenerationGate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct RootGeneration(u64);

impl RootGeneration {
    /// Generation used before any root switch.
    pub const INITIAL: Self = Self(0);

    /// Creates a generation from a raw value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw value, useful for diagnostics and logs.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the next, strictly greater token.
    ///
    /// # Panics
    ///
    /// Panics only if the token counter is exhausted (`u64::MAX`), which cannot
    /// happen through normal root switching.
    #[must_use]
    pub fn next(self) -> Self {
        Self(
            self.0
                .checked_add(1)
                .expect("root generation counter exhausted"),
        )
    }

    /// Returns `true` when this token is older than `current`.
    #[must_use]
    pub const fn is_stale(self, current: Self) -> bool {
        self.0 < current.0
    }

    /// Returns `true` when this token exactly matches `current`.
    #[must_use]
    pub const fn is_current(self, current: Self) -> bool {
        self.0 == current.0
    }
}

impl From<u64> for RootGeneration {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

impl From<RootGeneration> for u64 {
    fn from(value: RootGeneration) -> Self {
        value.get()
    }
}

impl fmt::Display for RootGeneration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Error returned when work for an older root generation is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleGeneration {
    /// Generation currently owned by the tree or preview.
    pub expected: RootGeneration,
    /// Generation carried by the rejected load or event.
    pub received: RootGeneration,
}

impl StaleGeneration {
    /// Creates a stale-generation error.
    #[must_use]
    pub const fn new(expected: RootGeneration, received: RootGeneration) -> Self {
        Self { expected, received }
    }
}

impl fmt::Display for StaleGeneration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "stale root generation {} (current generation is {})",
            self.received, self.expected
        )
    }
}

impl Error for StaleGeneration {}

/// Small helper that owns the current root generation.
///
/// The app can keep one gate for the active workspace, advance it on a root
/// switch, and check background results before applying them. The gate accepts
/// only the exact current token; both older and not-yet-issued tokens are
/// rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationGate {
    current: RootGeneration,
}

impl GenerationGate {
    /// Creates a gate at `current`.
    #[must_use]
    pub const fn new(current: RootGeneration) -> Self {
        Self { current }
    }

    /// Returns the current generation.
    #[must_use]
    pub const fn current(self) -> RootGeneration {
        self.current
    }

    /// Advances to a new token and returns it.
    pub fn advance(&mut self) -> RootGeneration {
        self.current = self.current.next();
        self.current
    }

    /// Returns `true` when `generation` is the exact current token.
    #[must_use]
    pub const fn accepts(self, generation: RootGeneration) -> bool {
        self.current.is_current(generation)
    }

    /// Rejects every token except the current one.
    pub fn check(self, generation: RootGeneration) -> Result<(), StaleGeneration> {
        if self.accepts(generation) {
            Ok(())
        } else {
            Err(StaleGeneration::new(self.current, generation))
        }
    }

    /// Accepts `value` only when it belongs to the current generation.
    pub fn accept<T>(self, generation: RootGeneration, value: T) -> Result<T, StaleGeneration> {
        self.check(generation)?;
        Ok(value)
    }

    /// Moves the gate forward to `generation`.
    ///
    /// A token equal to the current one is allowed; moving backwards is
    /// rejected so a stale root cannot become current again.
    pub fn set_current(&mut self, generation: RootGeneration) -> Result<(), StaleGeneration> {
        if generation.is_stale(self.current) {
            return Err(StaleGeneration::new(self.current, generation));
        }
        self.current = generation;
        Ok(())
    }
}

impl Default for GenerationGate {
    fn default() -> Self {
        Self::new(RootGeneration::INITIAL)
    }
}
