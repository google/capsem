//! Presentation state for the last explicit lifecycle result.
use crate::gateway::StatusResponse;

#[derive(Default)]
pub(crate) struct ActionFeedback {
    message: Option<String>,
}

impl ActionFeedback {
    pub(crate) fn record(&mut self, message: Option<String>) {
        self.message = message;
    }
    pub(crate) fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
    pub(crate) fn apply(&self, status: &mut StatusResponse) {
        status.action_error.clone_from(&self.message);
    }
}

#[cfg(test)]
mod tests;
