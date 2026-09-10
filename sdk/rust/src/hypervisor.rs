use std::time::Duration;

use crate::client::Client;
use crate::{models, operations as api, CreateOptions, Error, LogOptions, Result, VmSelector, VM};

/// A gateway connection. Clones and VM handles share the HTTP connection pool.
#[derive(Debug, Clone)]
pub struct Hypervisor {
    client: Client,
}

impl Hypervisor {
    pub fn new(url: &str, token: &str) -> Result<Self> {
        Ok(Self {
            client: Client::new(url, token)?,
        })
    }

    /// Set this handle's HTTP deadline, including response body reads.
    pub fn with_timeout(mut self, timeout: Duration) -> Result<Self> {
        self.client.set_timeout(timeout)?;
        Ok(self)
    }

    pub async fn info(&self) -> Result<models::HypervisorInfo> {
        api::get_hypervisor_info(&self.client.transport, self.client.options).await
    }

    /// Restart an idle managed service. Reconnect with a fresh gateway token.
    pub async fn restart(&self) -> Result<models::RestartResponse> {
        api::restart_hypervisor(&self.client.transport, self.client.options).await
    }

    pub async fn list(&self) -> Result<models::ListResponse> {
        api::list_vms(&self.client.transport, self.client.options).await
    }

    pub fn vm(&self, selector: VmSelector) -> Result<VM> {
        VM::bind(self.client.clone(), selector)
    }

    pub async fn create(&self, profile: &str, options: CreateOptions) -> Result<VM> {
        if options.vcpu == Some(0) {
            return Err(Error::InvalidInput("vcpu must be positive"));
        }
        let name = options.name.filter(|name| !name.is_empty());
        let body = models::ProvisionRequest {
            profile_id: profile.to_owned(),
            persistent: name.is_some(),
            name,
            cpus: options.vcpu,
            ram_mb: options.memory.map(crate::Memory::megabytes).transpose()?,
            env: options.env,
            from: None,
        };
        let result = api::create_vm(
            &self.client.transport,
            &api::CreateVmParams { body },
            self.client.options,
        )
        .await?;
        VM::created(self.client.clone(), result.id, result.name)
    }

    pub async fn log(&self, source: models::HostLogSource, options: LogOptions) -> Result<models::HostLogsResponse> {
        let params = api::GetHypervisorLogsParams {
            name: source,
            grep: options.grep,
            tail: options.tail,
            max_bytes: options.max_bytes,
        };
        api::get_hypervisor_logs(&self.client.transport, &params, self.client.options).await
    }

    pub async fn update(&self) -> Result<models::UpdateActionResponse> {
        let params = api::UpdateHypervisorParams {
            body: models::UpdateApplyRequest {
                confirmed: true,
                dry_run: false,
            },
        };
        api::update_hypervisor(&self.client.transport, &params, self.client.options).await
    }
}
