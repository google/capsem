//! One original command budget, excluding only its verified human wait.
use capsem_proto::ipc::ExecApprovalProgress;
use std::time::Duration;
use tokio::time::Instant;

enum Phase {
    Ordinary,
    Waiting {
        ask_id: String,
        event_id: String,
        ordinary_remaining: Duration,
    },
    Resolved,
}

pub(super) struct ExecBudget {
    deadline: Option<Instant>,
    phase: Phase,
}

impl ExecBudget {
    pub(super) fn new(timeout_secs: Option<u64>) -> Self {
        Self {
            deadline: timeout_secs.map(|seconds| Instant::now() + Duration::from_secs(seconds)),
            phase: Phase::Ordinary,
        }
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub(super) fn waiting(&self) -> bool {
        matches!(self.phase, Phase::Waiting { .. })
    }

    pub(super) fn observe(&mut self, progress: &ExecApprovalProgress) -> Result<(), String> {
        let now = Instant::now();
        match progress {
            ExecApprovalProgress::Registered(registration) => {
                registration.validate().map_err(str::to_string)?;
                if !matches!(self.phase, Phase::Ordinary) {
                    return Err("duplicate execution approval registration".into());
                }
                let ordinary_remaining = self
                    .deadline
                    .and_then(|deadline| deadline.checked_duration_since(now))
                    .filter(|remaining| !remaining.is_zero())
                    .ok_or("execution budget expired before approval registration")?;
                self.deadline = Some(now + Duration::from_millis(registration.remaining_ms));
                self.phase = Phase::Waiting {
                    ask_id: registration.ask_id.clone(),
                    event_id: registration.event_id.clone(),
                    ordinary_remaining,
                };
            }
            ExecApprovalProgress::Resolved { ask_id, event_id } => {
                let Phase::Waiting {
                    ask_id: original_ask,
                    event_id: original_event,
                    ordinary_remaining,
                } = &self.phase
                else {
                    return Err("execution approval resolution has no live registration".into());
                };
                if original_ask != ask_id || original_event != event_id {
                    return Err("execution approval resolution identity mismatch".into());
                }
                if self.deadline.is_none_or(|deadline| now >= deadline) {
                    return Err("execution approval expired before resolution".into());
                }
                self.deadline = Some(now + *ordinary_remaining);
                self.phase = Phase::Resolved;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
