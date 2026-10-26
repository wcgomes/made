//! Bounded, non-blocking delivery of bridge events.
//!
//! Socket, accept, and connection threads must never block on a stalled
//! application event loop. This queue is therefore push-only from those
//! threads: it never waits for capacity. Instead it applies a fixed retention
//! policy:
//!
//! 1. Lifecycle events (`Connected`, authenticated `Disconnected`,
//!    `HeartbeatTimeout`, and `Goodbye`) are retained ahead of other events.
//! 2. A single pending state snapshot is kept per authenticated connection
//!    epoch and replaced in place by newer snapshots.
//! 3. Authenticated protocol errors are retained ahead of state and focus
//!    results, but repeated errors may displace the oldest error under floods.
//! 4. Unauthenticated notices have a small budget of their own, so unauthenticated
//!    traffic cannot displace authenticated lifecycle events or state.
//! 5. Anything dropped because the budget is exhausted increments the dropped
//!    counter; replaced state snapshots increment the coalesced counter.
//!
//! The queue is bounded at [`EVENT_QUEUE_CAPACITY`] entries at all times.

use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use crate::{
    security::{EVENT_QUEUE_CAPACITY, EVENT_QUEUE_RESERVED_SLOTS, MAX_UNAUTH_EVENT_NOTICES},
    server::BridgeEvent,
    state::SessionSnapshot,
};

/// Retention priority of a queued event.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EventClass {
    /// Lifecycle events that identify or terminate a connection.
    Lifecycle,
    /// One coalesced snapshot per connection epoch.
    State,
    /// An authenticated protocol error.
    ProtocolError,
    /// A focus result or similar best-effort notification.
    Droppable,
    /// A pre-authentication notice with its own budget.
    Unauth,
}

struct QueueEntry {
    class: EventClass,
    epoch: Option<u64>,
    event: BridgeEvent,
}

#[derive(Default)]
struct QueueState {
    entries: VecDeque<QueueEntry>,
    epoch: u64,
    closed: bool,
    dropped: u64,
    coalesced: u64,
    droppable: usize,
    unauth: usize,
}

impl QueueState {
    fn push_entry(&mut self, entry: QueueEntry) {
        match entry.class {
            EventClass::State | EventClass::Droppable => self.droppable += 1,
            EventClass::Unauth => {
                self.droppable += 1;
                self.unauth += 1;
            }
            EventClass::Lifecycle | EventClass::ProtocolError => {}
        }
        self.entries.push_back(entry);
    }

    fn decrement(&mut self, entry: &QueueEntry) {
        match entry.class {
            EventClass::State | EventClass::Droppable => {
                self.droppable = self.droppable.saturating_sub(1);
            }
            EventClass::Unauth => {
                self.droppable = self.droppable.saturating_sub(1);
                self.unauth = self.unauth.saturating_sub(1);
            }
            EventClass::Lifecycle | EventClass::ProtocolError => {}
        }
    }

    fn pop_front(&mut self) -> Option<BridgeEvent> {
        let entry = self.entries.pop_front()?;
        self.decrement(&entry);
        Some(entry.event)
    }

    fn remove(&mut self, index: usize) -> Option<QueueEntry> {
        let entry = self.entries.remove(index)?;
        self.decrement(&entry);
        Some(entry)
    }

    /// Remove the oldest event matching `predicate` and count it as dropped.
    fn evict_oldest(&mut self, predicate: impl Fn(EventClass) -> bool) -> bool {
        let Some(index) = self.entries.iter().position(|entry| predicate(entry.class)) else {
            return false;
        };
        self.remove(index);
        self.dropped += 1;
        true
    }

    fn evict_state_from_other_epoch(&mut self) -> bool {
        let epoch = self.epoch;
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.class == EventClass::State && entry.epoch != Some(epoch))
        else {
            return false;
        };
        self.remove(index);
        self.dropped += 1;
        true
    }
}

/// A bounded event queue with a waitable consumer side.
///
/// The producer methods never block. The consumer methods wait for events or a
/// timeout and are used by [`BridgeHandle`](crate::server::BridgeHandle).
pub(crate) struct EventQueue {
    capacity: usize,
    droppable_budget: usize,
    state: Mutex<QueueState>,
    wake: Condvar,
}

impl EventQueue {
    pub(crate) fn new() -> Self {
        let capacity = EVENT_QUEUE_CAPACITY;
        let reserved = EVENT_QUEUE_RESERVED_SLOTS.min(capacity.saturating_sub(1));
        Self {
            capacity,
            droppable_budget: capacity.saturating_sub(reserved),
            state: Mutex::new(QueueState::default()),
            wake: Condvar::new(),
        }
    }

    /// Begin a new authenticated connection epoch.
    ///
    /// Queued state snapshots from older epochs are discarded so a replacement
    /// connection can never observe a stale snapshot after its `Connected`
    /// event.
    pub(crate) fn begin_epoch(&self, epoch: u64) {
        let mut state = lock(&self.state);
        state.epoch = epoch;
        let mut removed = 0_usize;
        state.entries.retain(|entry| {
            let keep = !(entry.class == EventClass::State && entry.epoch != Some(epoch));
            if !keep {
                removed += 1;
            }
            keep
        });
        state.droppable = state.droppable.saturating_sub(removed);
        state.dropped += removed as u64;
    }

    /// Queue a lifecycle event. Lifecycle events are retained ahead of state,
    /// protocol errors, and unauth notices.
    pub(crate) fn push_lifecycle(&self, event: BridgeEvent) {
        let mut state = lock(&self.state);
        if state.entries.len() >= self.capacity
            && !(state
                .evict_oldest(|class| matches!(class, EventClass::Droppable | EventClass::Unauth))
                || state.evict_state_from_other_epoch()
                || state.evict_oldest(|class| class == EventClass::ProtocolError)
                || state.evict_oldest(|class| class == EventClass::Lifecycle))
        {
            state.dropped += 1;
            return;
        }
        state.push_entry(QueueEntry {
            class: EventClass::Lifecycle,
            epoch: None,
            event,
        });
        drop(state);
        self.wake.notify_one();
    }

    /// Queue an authenticated protocol error.
    ///
    /// Droppable notices are displaced first, then the oldest protocol error.
    /// Connected and disconnected lifecycle events are preserved.
    pub(crate) fn push_protocol_error(&self, event: BridgeEvent) {
        let mut state = lock(&self.state);
        if state.entries.len() >= self.capacity
            && !(state
                .evict_oldest(|class| matches!(class, EventClass::Droppable | EventClass::Unauth))
                || state.evict_oldest(|class| class == EventClass::ProtocolError))
        {
            state.dropped += 1;
            return;
        }
        state.push_entry(QueueEntry {
            class: EventClass::ProtocolError,
            epoch: None,
            event,
        });
        drop(state);
        self.wake.notify_one();
    }

    /// Queue a state snapshot, replacing any pending snapshot for `epoch`.
    ///
    /// Snapshots from an older epoch are discarded. This is the mechanism that
    /// keeps the queue bounded during rapid tab changes and makes replacement
    /// connections supersede stale state.
    pub(crate) fn push_state(&self, epoch: u64, snapshot: SessionSnapshot) {
        let mut state = lock(&self.state);
        if epoch != state.epoch {
            state.dropped += 1;
            return;
        }

        if let Some(entry) = state
            .entries
            .iter_mut()
            .find(|entry| entry.class == EventClass::State && entry.epoch == Some(epoch))
        {
            entry.event = BridgeEvent::State(snapshot);
            state.coalesced += 1;
            drop(state);
            self.wake.notify_one();
            return;
        }

        if state.droppable >= self.droppable_budget
            && !state
                .evict_oldest(|class| matches!(class, EventClass::Droppable | EventClass::Unauth))
        {
            state.dropped += 1;
            return;
        }
        if state.entries.len() >= self.capacity
            && !(state
                .evict_oldest(|class| matches!(class, EventClass::Droppable | EventClass::Unauth))
                || state.evict_oldest(|class| class == EventClass::ProtocolError))
        {
            state.dropped += 1;
            return;
        }

        state.push_entry(QueueEntry {
            class: EventClass::State,
            epoch: Some(epoch),
            event: BridgeEvent::State(snapshot),
        });
        drop(state);
        self.wake.notify_one();
    }

    /// Queue a best-effort event such as a focus result.
    pub(crate) fn push_droppable(&self, event: BridgeEvent) {
        let mut state = lock(&self.state);
        if state.droppable >= self.droppable_budget || state.entries.len() >= self.capacity {
            state.dropped += 1;
            return;
        }
        state.push_entry(QueueEntry {
            class: EventClass::Droppable,
            epoch: None,
            event,
        });
        drop(state);
        self.wake.notify_one();
    }

    /// Queue an unauthenticated notice. Unauthenticated traffic has a small
    /// dedicated budget and can never fill the authenticated queue.
    pub(crate) fn push_unauth(&self, event: BridgeEvent) {
        let mut state = lock(&self.state);
        if state.unauth >= MAX_UNAUTH_EVENT_NOTICES
            || state.droppable >= self.droppable_budget
            || state.entries.len() >= self.capacity
        {
            state.dropped += 1;
            return;
        }
        state.push_entry(QueueEntry {
            class: EventClass::Unauth,
            epoch: None,
            event,
        });
        drop(state);
        self.wake.notify_one();
    }

    pub(crate) fn try_recv(&self) -> Result<BridgeEvent, std::sync::mpsc::TryRecvError> {
        let mut state = lock(&self.state);
        match state.pop_front() {
            Some(event) => Ok(event),
            None if state.closed => Err(std::sync::mpsc::TryRecvError::Disconnected),
            None => Err(std::sync::mpsc::TryRecvError::Empty),
        }
    }

    pub(crate) fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<BridgeEvent, std::sync::mpsc::RecvTimeoutError> {
        let deadline = Instant::now().checked_add(timeout);
        let mut state = lock(&self.state);
        loop {
            if let Some(event) = state.pop_front() {
                return Ok(event);
            }
            if state.closed {
                return Err(std::sync::mpsc::RecvTimeoutError::Disconnected);
            }
            let wait = match deadline {
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(std::sync::mpsc::RecvTimeoutError::Timeout);
                    }
                    deadline - now
                }
                None => Duration::from_secs(60 * 60),
            };
            let (next, _) = self
                .wake
                .wait_timeout(state, wait)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next;
        }
    }

    pub(crate) fn drain(&self) -> Vec<BridgeEvent> {
        let mut state = lock(&self.state);
        let mut events = Vec::with_capacity(state.entries.len());
        while let Some(event) = state.pop_front() {
            events.push(event);
        }
        events
    }

    pub(crate) fn close(&self) {
        let mut state = lock(&self.state);
        state.closed = true;
        drop(state);
        self.wake.notify_all();
    }

    pub(crate) fn dropped(&self) -> u64 {
        lock(&self.state).dropped
    }

    pub(crate) fn coalesced(&self) -> u64 {
        lock(&self.state).coalesced
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
