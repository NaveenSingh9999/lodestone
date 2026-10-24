use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

pub mod runner;
pub use runner::{InstanceRunner, RunOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstanceState {
    Stopped,
    Installing,
    Starting,
    Running,
    Stopping,
    Crashed,
    Restarting,
    Updating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsmEvent {
    StartRequested,
    Ready,
    StopRequested,
    ProcessExited,
}

pub fn step(state: InstanceState, event: FsmEvent) -> Option<InstanceState> {
    use FsmEvent::*;
    use InstanceState::*;
    Some(match (state, event) {
        (Stopped | Crashed | Restarting | Installing, StartRequested) => Starting,
        (Starting, Ready) => Running,
        (Starting, StopRequested) => Stopping,
        (Starting, ProcessExited) => Crashed,
        (Running, StopRequested) => Stopping,
        (Running, ProcessExited) => Crashed,
        (Stopping, ProcessExited) => Stopped,
        (Restarting, ProcessExited) => Stopped,
        _ => return None,
    })
}

pub struct CrashTracker {
    max_crashes: usize,
    window_ms: u64,
    times: VecDeque<u64>,
}

impl CrashTracker {
    pub fn new(max_crashes: usize, window_ms: u64) -> Self {
        CrashTracker {
            max_crashes,
            window_ms,
            times: VecDeque::new(),
        }
    }

    pub fn record(&mut self, now_ms: u64) -> bool {
        self.times.push_back(now_ms);
        while let Some(front) = self.times.front() {
            if now_ms.saturating_sub(*front) > self.window_ms {
                self.times.pop_front();
            } else {
                break;
            }
        }
        self.times.len() >= self.max_crashes
    }

    pub fn reset(&mut self) {
        self.times.clear();
    }
}

/// Restart policy for instance supervision (spec §4.4): auto-restart honest
/// crashes within a window; hold a backoff; never restart after the system
/// itself killed the process.
#[derive(Debug, Clone)]
pub struct InstancePolicy {
    /// Unique log marker considered the server "ready" (vanilla/paper: `Done (`).
    pub ready_marker: String,
    /// Console command sent for a graceful stop (vanilla: `stop`).
    pub stop_command: String,
    /// Time allowed for an instance to exit after a graceful stop before
    /// SIGKILL.
    pub stop_grace: std::time::Duration,
    /// Max unexpected crashes within `crash_window` before giving up.
    pub max_crashes: usize,
    pub crash_window: std::time::Duration,
    /// Base restart backoff; doubles per consecutive crash up to `backoff_cap`.
    pub backoff_initial: std::time::Duration,
    pub backoff_cap: std::time::Duration,
}

impl Default for InstancePolicy {
    fn default() -> Self {
        InstancePolicy {
            ready_marker: "Done (".to_string(),
            stop_command: "stop".to_string(),
            stop_grace: std::time::Duration::from_secs(30),
            max_crashes: 3,
            crash_window: std::time::Duration::from_secs(10 * 60),
            backoff_initial: std::time::Duration::from_secs(1),
            backoff_cap: std::time::Duration::from_secs(30),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path() {
        let mut s = InstanceState::Stopped;
        s = step(s, FsmEvent::StartRequested).unwrap();
        assert_eq!(s, InstanceState::Starting);
        s = step(s, FsmEvent::Ready).unwrap();
        assert_eq!(s, InstanceState::Running);
        s = step(s, FsmEvent::StopRequested).unwrap();
        assert_eq!(s, InstanceState::Stopping);
        s = step(s, FsmEvent::ProcessExited).unwrap();
        assert_eq!(s, InstanceState::Stopped);
    }

    #[test]
    fn start_then_exit_is_crash() {
        let s = InstanceState::Stopped;
        let s = step(s, FsmEvent::StartRequested).unwrap();
        let s = step(s, FsmEvent::ProcessExited).unwrap();
        assert_eq!(s, InstanceState::Crashed);
        let s = step(s, FsmEvent::StartRequested).unwrap();
        assert_eq!(s, InstanceState::Starting);
    }

    #[test]
    fn running_ready_is_invalid() {
        assert!(step(InstanceState::Running, FsmEvent::Ready).is_none());
        assert!(step(InstanceState::Stopped, FsmEvent::ProcessExited).is_none());
    }

    #[test]
    fn crash_loop_in_window() {
        let mut t = CrashTracker::new(3, 10 * 60 * 1000);
        assert!(!t.record(0));
        assert!(!t.record(60 * 1000));
        assert!(t.record(120 * 1000));
        t.reset();
        assert!(!t.record(0));
    }

    #[test]
    fn spaced_crashes_do_not_loop() {
        let mut t = CrashTracker::new(3, 10 * 60 * 1000);
        assert!(!t.record(0));
        assert!(!t.record(11 * 60 * 1000));
        assert!(!t.record(22 * 60 * 1000));
    }
}
