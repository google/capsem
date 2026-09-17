use std::time::Duration;

use crate::transport::{CallOptions, Transport};
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub(crate) struct Client {
    pub transport: Transport,
    pub options: CallOptions,
}

impl Client {
    pub fn new(url: &str, token: &str) -> Result<Self> {
        Ok(Self {
            transport: Transport::new(url, token, Duration::from_secs(30))?,
            options: CallOptions::default(),
        })
    }

    /// The service's ceiling for one exec or run, which is also its default.
    const EXEC_TIMEOUT_CEILING: Duration = Duration::from_secs(60 * 60);
    /// The gateway's budget for readiness, boot and teardown around a command.
    const GATEWAY_REQUEST_BUDGET: Duration = Duration::from_secs(120);

    /// Call options for exec or run: the service answers only when the command
    /// ends, so wait at least as long as the gateway does for it.
    pub fn command_options(&self, timeout_secs: Option<u64>) -> CallOptions {
        let command = timeout_secs.map_or(Self::EXEC_TIMEOUT_CEILING, Duration::from_secs);
        let deadline = command.saturating_add(Self::GATEWAY_REQUEST_BUDGET);
        CallOptions {
            timeout: Some(
                self.options
                    .timeout
                    .unwrap_or_else(|| self.transport.timeout())
                    .max(deadline),
            ),
        }
    }

    pub fn set_timeout(&mut self, timeout: Duration) -> Result<()> {
        if timeout.is_zero() {
            return Err(Error::InvalidInput("request timeout must be positive"));
        }
        self.options.timeout = Some(timeout);
        Ok(())
    }
}
