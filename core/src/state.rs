//! Tracks one server's connection state from successive poll results.

use crate::client::{PollFailure, PollResult};
use crate::models::StrataSnapshot;

/// Failed polls in a row before a server counts as offline, so one dropped packet doesn't alert.
pub const FAILURES_BEFORE_OFFLINE: u32 = 3;

/// Unix seconds. A plain number keeps this code and its tests free of the system clock.
pub type Time = f64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ServerStatus {
    #[default]
    Connecting,
    Online,
    Offline,
    Rejected,
}

#[derive(Debug, Clone, Default)]
pub struct ServerState {
    failures: u32,
    pub status: ServerStatus,
    pub latest: Option<StrataSnapshot>,
    pub active_address: Option<String>,
    pub last_error: Option<String>,
    pub last_failure: PollFailure,
    pub last_seen: Option<Time>,
}

impl ServerState {
    /// The latest poll didn't answer, even if too few have failed to call the server offline,
    /// so `latest` is from an earlier poll.
    pub fn last_poll_failed(&self) -> bool {
        self.failures > 0
    }

    pub fn apply(&mut self, result: PollResult, now: Time) {
        if let Some(snapshot) = result.snapshot {
            self.failures = 0;
            self.status = ServerStatus::Online;
            self.latest = Some(snapshot);
            self.active_address = result.address;
            self.last_seen = Some(now);
            self.last_error = None;
            self.last_failure = PollFailure::None;
            return;
        }

        self.failures += 1;
        self.last_failure = result.failure;
        let detail = result.detail.unwrap_or_default();
        self.last_error = Some(match result.address {
            Some(address) => format!("{address}: {detail}"),
            None => detail,
        });

        // A definite answer rather than a possible network blip, so it doesn't wait.
        if matches!(
            result.failure,
            PollFailure::KeyMissing
                | PollFailure::KeyRejected
                | PollFailure::HostRefused
                | PollFailure::Incompatible
                | PollFailure::NoTelemetry
        ) {
            self.status = ServerStatus::Rejected;
        } else if self.failures >= FAILURES_BEFORE_OFFLINE {
            self.status = ServerStatus::Offline;
        }
    }
}
