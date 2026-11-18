use std::time::Duration;

use crate::transport::{CallOptions, Transport};
use crate::{Error, Result};

/// Plan a creation wait from its exact approved revision before issuing the
/// mutation. Timing metadata grants no permission to launch that revision.
pub async fn create_call_options(
    transport: &Transport,
    request: &crate::models::ProvisionRequest,
    options: CallOptions,
) -> Result<CallOptions> {
    request.validate_environment().map_err(Error::InvalidInput)?;
    let startup = match request.environment.as_ref().filter(|_| !request.attach) {
        Some(environment) => {
            let response = crate::operations::get_environment_readiness(
                transport,
                &crate::operations::GetEnvironmentReadinessParams {
                    id: environment.environment_id.clone(),
                    revision_id: environment.revision_id.clone(),
                },
                options,
            )
            .await?;
            if response.environment != *environment {
                return Err(Error::InvalidResponse("environment readiness identity changed"));
            }
            u64::from(response.startup_timeout_seconds)
        }
        None => 0,
    };
    Ok(CallOptions {
        timeout: Some(
            options
                .timeout
                .unwrap_or_else(|| transport.timeout())
                .max(Duration::from_secs(110 + 120 + startup)),
        ),
    })
}

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
