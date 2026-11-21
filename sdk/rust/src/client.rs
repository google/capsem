use std::time::Duration;

use capsem_api::{
    ForkRequest, ProvisionRequest, VmLifecycleIntent, VmLifecycleSelection, VmReadinessResponse, VmStartRequest,
};

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
            response.startup_timeout_seconds
        }
        None => 0,
    };
    Ok(preparation_call_options(transport, startup, options))
}

/// Bind a create-from request to the source it observed, and plan the wait for
/// the destination's approved startup.
pub async fn prepare_create(
    transport: &Transport,
    request: &mut ProvisionRequest,
    options: CallOptions,
) -> Result<CallOptions> {
    request.validate_environment().map_err(Error::InvalidInput)?;
    if let Some(source) = request.from.as_deref() {
        let readiness = lifecycle_readiness(
            transport,
            source,
            VmLifecycleIntent::Clone,
            request.expected_source.as_ref(),
            options,
        )
        .await?;
        let inherits_startup =
            request.environment.is_none() && request.container.is_none() && readiness.selection.environment.is_some();
        request.expected_source = Some(readiness.selection);
        if inherits_startup {
            return Ok(preparation_call_options(
                transport,
                readiness.startup_timeout_seconds,
                options,
            ));
        }
    }
    create_call_options(transport, request, options).await
}

/// Bind start or resume to the original VM generation and plan its startup wait.
pub async fn prepare_start(
    transport: &Transport,
    id: &str,
    request: &mut VmStartRequest,
    options: CallOptions,
) -> Result<CallOptions> {
    request.validate(id).map_err(Error::InvalidInput)?;
    let readiness = lifecycle_readiness(
        transport,
        id,
        VmLifecycleIntent::Start,
        request.expected.as_ref(),
        options,
    )
    .await?;
    let startup = readiness
        .selection
        .environment
        .as_ref()
        .map_or(0, |_| readiness.startup_timeout_seconds);
    request.expected = Some(readiness.selection);
    Ok(preparation_call_options(transport, startup, options))
}

/// Bind a fork to the source generation and desired revision it observed.
pub async fn prepare_fork(
    transport: &Transport,
    id: &str,
    request: &mut ForkRequest,
    options: CallOptions,
) -> Result<()> {
    request.validate_expected(id).map_err(Error::InvalidInput)?;
    let readiness = lifecycle_readiness(
        transport,
        id,
        VmLifecycleIntent::Fork,
        request.expected.as_ref(),
        options,
    )
    .await?;
    request.expected = Some(readiness.selection);
    Ok(())
}

fn selection_changed() -> Error {
    Error::Http {
        status: 409,
        body: br#"{"error":"lifecycle selection changed; request a new readiness view"}"#.to_vec(),
    }
}

pub(crate) fn preparation_call_options(
    transport: &Transport,
    startup_seconds: u32,
    options: CallOptions,
) -> CallOptions {
    CallOptions {
        timeout: Some(
            options
                .timeout
                .unwrap_or_else(|| transport.timeout())
                .max(CallOptions::command_timeout(Some(110 + u64::from(startup_seconds)))),
        ),
    }
}

async fn lifecycle_readiness(
    transport: &Transport,
    id: &str,
    intent: VmLifecycleIntent,
    expected: Option<&VmLifecycleSelection>,
    options: CallOptions,
) -> Result<VmReadinessResponse> {
    capsem_api::validate_name("VM", id).map_err(|_| Error::InvalidInput("invalid VM identifier"))?;
    let response = crate::operations::get_vm_readiness(
        transport,
        &crate::operations::GetVmReadinessParams { id: id.into(), intent },
        options,
    )
    .await;
    let readiness = match response {
        Err(Error::Http { status: 404, .. }) if expected.is_some() => return Err(selection_changed()),
        result => result?,
    };
    if intent == VmLifecycleIntent::Clone {
        readiness.selection.validate()
    } else {
        readiness.selection.validate_target(id)
    }
    .map_err(Error::InvalidResponse)?;
    if expected.is_some_and(|value| *value != readiness.selection) {
        return Err(selection_changed());
    }
    Ok(readiness)
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

    /// Default wait for workload preparation without a saved startup script.
    pub fn create_options(&self) -> CallOptions {
        preparation_call_options(&self.transport, 0, self.options)
    }

    pub fn set_timeout(&mut self, timeout: Duration) -> Result<()> {
        if timeout.is_zero() {
            return Err(Error::InvalidInput("request timeout must be positive"));
        }
        self.options.timeout = Some(timeout);
        Ok(())
    }
}
