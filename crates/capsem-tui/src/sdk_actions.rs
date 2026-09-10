//! TUI action intent uses the gateway SDK's typed HTTP contract.

use std::time::Duration;

use capsem_sdk::models::ProvisionRequest;
use capsem_sdk::operations as api;
use capsem_sdk::transport::{CallOptions, Transport};
use capsem_sdk::{Error, Hypervisor, Result, VmSelector};

use crate::app::ControlAction;
use crate::gateway_provider::ActionOutcome;

pub async fn invoke(base_url: &str, token: &str, action: &ControlAction) -> Result<ActionOutcome> {
    let vm = |id: &str| Hypervisor::new(base_url, token)?.vm(VmSelector::Id(id.into()));
    match action {
        ControlAction::CreateSession { name, profile_id } => {
            // TUI workspaces persist even when the service chooses their name.
            let input = api::CreateVmParams {
                body: ProvisionRequest {
                    name: name.clone(),
                    profile_id: profile_id.clone(),
                    persistent: true,
                    ram_mb: None,
                    cpus: None,
                    env: None,
                    from: None,
                },
            };
            let transport = Transport::new(base_url, token, Duration::from_secs(30))?;
            let vm = api::create_vm(&transport, &input, CallOptions::default()).await?;
            Ok(ActionOutcome {
                message: name
                    .as_deref()
                    .map_or_else(|| "created session".into(), |name| format!("created {name}")),
                focus_session: Some(vm.id),
            })
        }
        ControlAction::Fork { id, name } => {
            let vm = vm(id)?.fork(name, None).await?;
            Ok(ActionOutcome {
                message: format!("forked {}", vm.name().unwrap_or(name)),
                focus_session: vm.id().map(str::to_owned),
            })
        }
        ControlAction::Resume { id, label } => {
            vm(id)?.resume().await?;
            Ok(focused(id, format!("resumed {label}")))
        }
        ControlAction::Checkpoint { id, label } => {
            vm(id)?.pause().await?;
            Ok(focused(id, format!("checkpointed {label}")))
        }
        ControlAction::Suspend { id, label } => {
            vm(id)?.pause().await?;
            Ok(focused(id, format!("suspended {label}")))
        }
        ControlAction::Stop { id, label } => {
            vm(id)?.stop().await?;
            Ok(focused(id, format!("stopped {label}")))
        }
        ControlAction::Delete { id, label } => {
            vm(id)?.delete().await?;
            Ok(ActionOutcome {
                message: format!("deleted {label}"),
                focus_session: None,
            })
        }
        _ => Err(Error::InvalidInput("action is not a VM lifecycle operation")),
    }
}

fn focused(id: &str, message: String) -> ActionOutcome {
    ActionOutcome {
        message,
        focus_session: Some(id.into()),
    }
}

pub fn display_error(error: Error) -> anyhow::Error {
    match error {
        Error::Http { status, body } => {
            anyhow::anyhow!("gateway action failed ({status}): {}", String::from_utf8_lossy(&body))
        }
        error => error.into(),
    }
}

#[cfg(test)]
mod tests;
