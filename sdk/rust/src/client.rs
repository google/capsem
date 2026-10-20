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

    /// Call options for exec or run: the service answers only when the command
    /// ends, so wait at least as long as the gateway does for it.
    pub fn command_options(&self, timeout_secs: Option<u64>) -> CallOptions {
        let deadline = CallOptions::command_timeout(timeout_secs);
        CallOptions {
            timeout: Some(
                self.options
                    .timeout
                    .unwrap_or_else(|| self.transport.timeout())
                    .max(deadline),
            ),
        }
    }

    /// Create/start/resume allow 110 seconds for workload readiness.
    pub fn create_options(&self) -> CallOptions {
        self.command_options(Some(110))
    }

    pub fn set_timeout(&mut self, timeout: Duration) -> Result<()> {
        if timeout.is_zero() {
            return Err(Error::InvalidInput("request timeout must be positive"));
        }
        self.options.timeout = Some(timeout);
        Ok(())
    }
}
