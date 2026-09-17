use std::time::Duration;

use crate::client::Client;
use crate::{
    models, operations as api,
    resources::{Debug, Networks, Profiles},
    CreateOptions, Error, LogOptions, Result, RunOptions, VmSelector, VM,
};

/// A gateway connection. Clones and VM handles share the HTTP connection pool.
#[derive(Debug, Clone)]
pub struct Hypervisor {
    client: Client,
    /// The catalog default, resolved once per handle.
    default_profile: tokio::sync::OnceCell<String>,
}

impl Hypervisor {
    fn memory_mb(memory: Option<u64>) -> Result<Option<u64>> {
        memory
            .map(|gib| {
                gib.checked_mul(1024)
                    .filter(|value| *value > 0)
                    .ok_or(Error::InvalidInput(
                        "memory must be a positive GiB count without overflow",
                    ))
            })
            .transpose()
    }

    pub fn new(url: &str, token: &str) -> Result<Self> {
        Ok(Self {
            client: Client::new(url, token)?,
            default_profile: tokio::sync::OnceCell::new(),
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

    /// The profile the gateway's catalog uses when a call names none, read
    /// from `GET /status` on first use and cached for this handle, so no
    /// profile name is compiled into the SDK.
    pub async fn default_profile_id(&self) -> Result<String> {
        self.default_profile
            .get_or_try_init(|| async {
                let catalog = self.info().await?.profiles;
                catalog
                    .and_then(|catalog| catalog.default_profile_id)
                    .filter(|id| !id.is_empty())
                    .ok_or(Error::InvalidInput(
                        "the gateway profile catalog names no default profile; pass a profile from profiles().list()",
                    ))
            })
            .await
            .cloned()
    }

    async fn profile_id(&self, profile: Option<models::ProfileSummary>) -> Result<String> {
        match profile {
            Some(profile) => Ok(profile.id),
            None => self.default_profile_id().await,
        }
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

    pub fn debug(&self) -> Debug<'_> {
        Debug(&self.client)
    }

    pub async fn create(&self, options: CreateOptions) -> Result<VM> {
        if options.cpus == Some(0) {
            return Err(Error::InvalidInput("cpus must be positive"));
        }
        if options.image.is_none() && (!options.command.is_empty() || options.registry.is_some()) {
            return Err(Error::InvalidInput("container command and registry require an image"));
        }
        let name = options.name.filter(|name| !name.is_empty());
        let has_container = options.image.is_some();
        let (env, container) = match options.image {
            Some(image) if !image.is_empty() => (
                None,
                Some(models::ContainerSpec {
                    image,
                    args: options.command,
                    env: options.env.unwrap_or_default().into_iter().collect(),
                    registry: options.registry.map(Into::into),
                    attach: false,
                }),
            ),
            Some(_) => return Err(Error::InvalidInput("image must be a nonempty string")),
            None => (options.env, None),
        };
        // Every local check first: an invalid argument must be refused before
        // the client asks the gateway anything.
        let ram_mb = Self::memory_mb(options.memory)?;
        let body = models::ProvisionRequest {
            profile_id: self.profile_id(options.profile).await?,
            persistent: name.is_some(),
            name,
            cpus: options.cpus,
            ram_mb,
            env,
            from: None,
            networks: options.networks.into_iter().map(|network| network.name).collect(),
            container,
        };
        let result = api::create_vm(
            &self.client.transport,
            &api::CreateVmParams { body },
            self.client.options,
        )
        .await?;
        VM::created(self.client.clone(), result.id, result.name, Some(has_container))
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
        if options.cpus == Some(0) {
            return Err(Error::InvalidInput("cpus must be positive"));
        }
        let call = self.client.command_options(options.timeout_secs);
        let ram_mb = Self::memory_mb(options.memory)?;
        let profile_id = self.profile_id(options.profile).await?;
        api::run_vm(
            &self.client.transport,
            &api::RunVmParams {
                body: models::RunRequest {
                    command: command.into(),
                    profile_id,
                    timeout_secs: options.timeout_secs,
                    ram_mb,
                    cpus: options.cpus,
                    env: options.env,
                },
            },
            call,
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
