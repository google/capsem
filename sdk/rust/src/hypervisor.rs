use std::time::Duration;

use crate::client::Client;
use crate::{
    models, operations as api,
    resources::{Networks, Profiles},
    CreateOptions, DiagnosticOptions, Error, LogOptions, Result, RunOptions, TriageOptions, VmSelector, VM,
};

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

    pub fn networks(&self) -> Networks<'_> {
        Networks(&self.client)
    }

    pub fn profiles(&self) -> Profiles<'_> {
        Profiles(&self.client)
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
            networks: options.networks,
            container: None,
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

    pub async fn run(&self, command: &str, options: RunOptions) -> Result<models::ExecResponse> {
        if options.vcpu == Some(0) {
            return Err(Error::InvalidInput("vcpu must be positive"));
        }
        api::run_vm(
            &self.client.transport,
            &api::RunVmParams {
                body: models::RunRequest {
                    command: command.into(),
                    profile_id: options.profile.unwrap_or_else(|| "code".into()),
                    timeout_secs: options.timeout_secs,
                    ram_mb: options.memory.map(crate::Memory::megabytes).transpose()?,
                    cpus: options.vcpu,
                    env: options.env,
                },
            },
            self.client.options,
        )
        .await
    }

    pub async fn purge(&self, all: bool) -> Result<models::PurgeResponse> {
        api::purge_vms(
            &self.client.transport,
            &api::PurgeVmsParams {
                body: models::PurgeRequest { all },
            },
            self.client.options,
        )
        .await
    }

    pub async fn panics(&self, options: DiagnosticOptions) -> Result<models::PanicsResponse> {
        api::get_panics(
            &self.client.transport,
            &api::GetPanicsParams {
                since: options.since,
                limit: options.limit,
                id: None,
            },
            self.client.options,
        )
        .await
    }

    pub async fn triage(&self, options: TriageOptions) -> Result<models::TriageResponse> {
        api::get_triage(
            &self.client.transport,
            &api::GetTriageParams {
                since: options.since,
                limit: options.limit,
                id: options.vm_id,
            },
            self.client.options,
        )
        .await
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
