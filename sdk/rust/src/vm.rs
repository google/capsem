use std::sync::Arc;
use std::time::Duration;

use tokio::sync::OnceCell;

use crate::client::Client;
use crate::resources::{Copy, Snapshots, Stats};
use crate::{models, operations as api, Error, Result, VmSelector};

mod queries;

#[derive(Debug)]
struct Identity {
    id: OnceCell<String>,
    name: Option<String>,
}

/// A VM handle. Clones share canonical identity and the HTTP connection pool.
#[derive(Debug, Clone)]
pub struct VM {
    pub(crate) client: Client,
    identity: Arc<Identity>,
}

impl VM {
    pub fn new(url: &str, token: &str, selector: VmSelector) -> Result<Self> {
        Self::bind(Client::new(url, token)?, selector)
    }

    pub(crate) fn bind(client: Client, selector: VmSelector) -> Result<Self> {
        let (id, name) = match selector {
            VmSelector::Id(id) if !id.is_empty() => (Some(id), None),
            VmSelector::Name(name) if !name.is_empty() => (None, Some(name)),
            _ => return Err(Error::InvalidInput("select a VM by one nonempty name or id")),
        };
        Ok(Self {
            client,
            identity: Arc::new(Identity {
                id: OnceCell::new_with(id),
                name,
            }),
        })
    }

    pub(crate) fn created(client: Client, id: String, name: String) -> Result<Self> {
        let mut vm = Self::bind(client, VmSelector::Id(id))?;
        Arc::get_mut(&mut vm.identity)
            .expect("new VM identity is not shared")
            .name = Some(name);
        Ok(vm)
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Result<Self> {
        self.client.set_timeout(timeout)?;
        Ok(self)
    }

    /// Known immediately for ID selection, or after the first name lookup.
    pub fn id(&self) -> Option<&str> {
        self.identity.id.get().map(String::as_str)
    }

    pub fn name(&self) -> Option<&str> {
        self.identity.name.as_deref()
    }

    pub(crate) async fn resolve(&self) -> Result<String> {
        let id = self
            .identity
            .id
            .get_or_try_init(|| async {
                let name = self.name().expect("unresolved VM has a name");
                let response = api::list_vms(&self.client.transport, self.client.options).await?;
                let matches = response
                    .sandboxes
                    .into_iter()
                    .filter(|vm| vm.name.as_deref() == Some(name))
                    .collect::<Vec<_>>();
                if matches.len() != 1 {
                    return Err(Error::VmLookup {
                        name: name.to_owned(),
                        matches: matches.len(),
                    });
                }
                Ok(matches.into_iter().next().expect("exactly one match").id)
            })
            .await?;
        Ok(id.clone())
    }

    pub async fn info(&self) -> Result<models::SandboxInfo> {
        api::get_vm_info(
            &self.client.transport,
            &api::GetVmInfoParams {
                id: self.resolve().await?,
            },
            self.client.options,
        )
        .await
    }

    pub async fn exec(&self, command: &str, timeout_secs: Option<u64>) -> Result<models::ExecResponse> {
        let params = api::ExecVmParams {
            id: self.resolve().await?,
            body: models::ExecRequest {
                command: command.into(),
                timeout_secs,
            },
        };
        api::exec_vm(&self.client.transport, &params, self.client.options).await
    }

    pub async fn start(&self) -> Result<models::ProvisionResponse> {
        api::start_vm(
            &self.client.transport,
            &api::StartVmParams {
                id: self.resolve().await?,
            },
            self.client.options,
        )
        .await
    }

    pub async fn stop(&self) -> Result<models::StopResponse> {
        api::stop_vm(
            &self.client.transport,
            &api::StopVmParams {
                id: self.resolve().await?,
            },
            self.client.options,
        )
        .await
    }

    pub async fn pause(&self) -> Result<models::VmActionResponse> {
        api::pause_vm(
            &self.client.transport,
            &api::PauseVmParams {
                id: self.resolve().await?,
            },
            self.client.options,
        )
        .await
    }

    pub async fn resume(&self) -> Result<models::ProvisionResponse> {
        api::resume_vm(
            &self.client.transport,
            &api::ResumeVmParams {
                id: self.resolve().await?,
            },
            self.client.options,
        )
        .await
    }

    pub async fn delete(&self) -> Result<models::VmActionResponse> {
        api::delete_vm(
            &self.client.transport,
            &api::DeleteVmParams {
                id: self.resolve().await?,
            },
            self.client.options,
        )
        .await
    }

    pub async fn fork(&self, name: &str, description: Option<String>) -> Result<Self> {
        let params = api::ForkVmParams {
            id: self.resolve().await?,
            body: models::ForkRequest {
                name: name.into(),
                description,
            },
        };
        let result = api::fork_vm(&self.client.transport, &params, self.client.options).await?;
        Self::created(self.client.clone(), result.id, result.name)
    }

    pub fn copy(&self) -> Copy<'_> {
        Copy(self)
    }
    pub fn snapshots(&self) -> Snapshots<'_> {
        Snapshots(self)
    }
    pub fn stats(&self) -> Stats<'_> {
        Stats(self)
    }
}
