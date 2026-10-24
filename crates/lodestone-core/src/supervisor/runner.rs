//! InstanceRunner: drives the supervisor FSM (spec §4.4) against a real
//! `Platform::launch_server` handle. Pumps stdout/stderr into a tail buffer,
//! marks readiness from a log marker, honors graceful stop with a grace
//! timeout, classifies exits the way §4.4 requires, and auto-restarts only
//! honest crashes — never `KilledBySystem` (the device won; tuning is user
//! work, not silent restart loops).

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

use crate::platform::{ExitInfo, ExitKind, LaunchSpec, Platform, ServerHandle};
use crate::supervisor::{CrashTracker, FsmEvent, InstancePolicy, InstanceState, step};

const TAIL_CAP: usize = 256;

/// What a complete run ended with. `phases` keeps every FSM state visited
/// with its timestamp, for the crash-analysis screen (spec: readable report).
#[derive(Debug, Clone)]
pub struct RunReport {
    pub phases: Vec<(InstanceState, u64)>,
    pub restarts: u32,
    pub outcome: RunOutcome,
    pub tail: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunOutcome {
    /// Graceful stop honored (grace SIGKILL escalation is still a stop).
    StoppedClean,
    /// Validation/config failure before any process could spawn.
    FailedToStart { reason: String },
    /// Crashes exceeded the policy window; final exit class recorded.
    GaveUpAfterCrashes { last: ExitKind, count: u32 },
    /// The OS (governor/OOM/user) SIGKILLed the process group.
    KilledBySystem { exit: ExitInfo },
    /// FSM reached a state the transition table does not define.
    InvalidTransition { details: String },
    /// Bounded by the outer deadline (used to bound behavior in tests).
    TimedOut,
}

impl RunOutcome {
    pub fn is_ok(&self) -> bool {
        matches!(self, RunOutcome::StoppedClean)
    }
}

pub struct InstanceRunner {
    platform: Arc<dyn Platform>,
    policy: InstancePolicy,
    instance_id: String,
}

pub fn mark(phases: &mut Vec<(InstanceState, u64)>, started: &Instant, state: InstanceState) {
    let ms = started.elapsed().as_millis() as u64;
    if phases.last().map(|(s, _)| *s) != Some(state) {
        phases.push((state, ms));
    }
}

pub fn push_tail(tail: &mut VecDeque<String>, line: String) {
    if tail.len() >= TAIL_CAP {
        tail.pop_front();
    }
    tail.push_back(line);
}

pub async fn exhaust_pendings(handle: &mut ServerHandle) {
    use futures_util::FutureExt;
    loop {
        match handle.recv_output().now_or_never() {
            Some(Some(_)) => continue,
            Some(None) => break,
            None => break,
        }
    }
}

enum AttemptEnd {
    Exited { info: ExitInfo },
}

async fn take_exit(exit: &mut Option<oneshot::Receiver<ExitInfo>>) -> Option<ExitInfo> {
    match exit.as_mut() {
        Some(rx) => rx.await.ok(),
        None => None,
    }
}

impl InstanceRunner {
    pub fn new(
        platform: Arc<dyn Platform>,
        policy: InstancePolicy,
        instance_id: impl Into<String>,
    ) -> Self {
        InstanceRunner {
            platform,
            policy,
            instance_id: instance_id.into(),
        }
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn policy(&self) -> &InstancePolicy {
        &self.policy
    }

    fn spawn_attempt(
        &self,
        spec: LaunchSpec,
    ) -> (ServerHandle, Option<oneshot::Receiver<ExitInfo>>) {
        let mut handle = self.platform.launch_server(spec);
        let exit = handle.take_exit();
        (handle, exit)
    }

    /// Runs the whole lifecycle — spawn, readiness, graceful stop, exit —
    /// restarting honest crashes within the policy until a terminal outcome.
    pub async fn run(
        &mut self,
        spec: LaunchSpec,
        stop_requested_after: Option<Duration>,
        overall_deadline: Option<Duration>,
    ) -> Result<RunReport, crate::error::SupervisorError> {
        if let Err(reason) = crate::platform::validate_launch_spec(&spec) {
            return Ok(RunReport {
                phases: vec![(InstanceState::Stopped, 0)],
                restarts: 0,
                outcome: RunOutcome::FailedToStart { reason },
                tail: vec![],
            });
        }

        let started = Instant::now();
        let mut phases = vec![(InstanceState::Stopped, 0u64)];
        let mut tail: VecDeque<String> = VecDeque::with_capacity(128);
        let mut restarts = 0u32;
        let mut tracker = CrashTracker::new(
            self.policy.max_crashes,
            self.policy.crash_window.as_millis() as u64,
        );
        let mut backoff = self.policy.backoff_initial;
        let mut state = InstanceState::Stopped;

        loop {
            // ---- enter Starting ------------------------------------------------
            match step(state, FsmEvent::StartRequested) {
                Some(next) if next == InstanceState::Starting => {
                    state = next;
                }
                _ => {
                    return Ok(RunReport {
                        phases,
                        restarts,
                        outcome: RunOutcome::InvalidTransition {
                            details: format!("{state:?} + StartRequested"),
                        },
                        tail: tail.into(),
                    });
                }
            }
            mark(&mut phases, &started, state);

            let attempt_started = Instant::now();
            let (mut handle, mut exit) = self.spawn_attempt(spec.clone());
            let mut ready_seen = false;
            let mut stopping = false;
            let mut grace_kill_armed = false;
            let mut grace_deadline: Option<Instant> = None;

            // ---- live-until-exit ------------------------------------------------
            let end: AttemptEnd = loop {
                if let Some(deadline) = overall_deadline {
                    if started.elapsed() > deadline {
                        handle.kill();
                        return Ok(RunReport {
                            phases,
                            restarts,
                            outcome: RunOutcome::TimedOut,
                            tail: tail.into(),
                        });
                    }
                }

                let stop_requested = stop_requested_after.is_some() && !stopping;
                let grace_remaining = if stopping && !grace_kill_armed {
                    Some(grace_deadline.unwrap())
                } else {
                    None
                };

                tokio::select! {
                    line = handle.recv_output() => {
                        if let Some(line) = line {
                            if !ready_seen && line.line.contains(&self.policy.ready_marker) {
                                ready_seen = true;
                                if let Some(next) = step(InstanceState::Starting, FsmEvent::Ready) {
                                    state = next;
                                    mark(&mut phases, &started, state);
                                }
                            }
                            push_tail(&mut tail, line.line);
                        }
                    }

                    info = take_exit(&mut exit), if exit.is_some() => {
                        match info {
                            Some(info) => break AttemptEnd::Exited { info },
                            None => {
                                // The receiver vanished without reporting.
                                handle.kill();
                                break AttemptEnd::Exited {
                                    info: ExitInfo { code: Some(-1), signal: None },
                                };
                            }
                        }
                    }

                    _ = tokio::time::sleep_until(
                        tokio::time::Instant::from_std(
                            attempt_started + stop_requested_after.unwrap_or_default(),
                        ),
                    ), if stop_requested => {
                        if let Some(next) = step(state, FsmEvent::StopRequested) {
                            state = next;
                            mark(&mut phases, &started, state);
                        }
                        let _ = handle.send_console(self.policy.stop_command.as_str());
                        stopping = true;
                        grace_deadline = Some(Instant::now() + self.policy.stop_grace);
                    }

                    _ = tokio::time::sleep_until(
                        tokio::time::Instant::from_std(
                            grace_remaining.unwrap_or_else(Instant::now),
                        ),
                    ), if grace_remaining.is_some() => {
                        grace_kill_armed = true;
                        handle.kill();
                    }
                }
            };

            // ---- post-exit: flush tail, FSM deposit, classification ---------
            use futures_util::FutureExt;
            while let Some(Some(line)) = handle.recv_output().now_or_never() {
                push_tail(&mut tail, line.line);
            }

            let AttemptEnd::Exited { info } = end;
            let Some(next_state) = step(state, FsmEvent::ProcessExited) else {
                return Ok(RunReport {
                    phases,
                    restarts,
                    outcome: RunOutcome::InvalidTransition {
                        details: format!("{state:?} + ProcessExited"),
                    },
                    tail: tail.into(),
                });
            };
            state = next_state;
            mark(&mut phases, &started, state);

            if stopping {
                return Ok(RunReport {
                    phases,
                    restarts,
                    outcome: RunOutcome::StoppedClean,
                    tail: tail.into(),
                });
            }

            // Exit classification (§4.4): 137 / sig9 means the system
            // reaped the process; no restart loop against the device.
            if info.kind() == ExitKind::KilledBySystem {
                return Ok(RunReport {
                    phases,
                    restarts,
                    outcome: RunOutcome::KilledBySystem { exit: info },
                    tail: tail.into(),
                });
            }

            let now_ms = started.elapsed().as_millis() as u64;
            if tracker.record(now_ms) {
                return Ok(RunReport {
                    phases,
                    restarts,
                    outcome: RunOutcome::GaveUpAfterCrashes {
                        last: info.kind(),
                        count: self.policy.max_crashes as u32,
                    },
                    tail: tail.into(),
                });
            }

            if !backoff.is_zero() {
                tokio::time::sleep(backoff).await;
            }
            backoff = backoff
                .checked_mul(2)
                .map(|b| b.min(self.policy.backoff_cap))
                .unwrap_or(self.policy.backoff_cap);
            restarts += 1;
        }
    }
}
