use crate::client::Client;
use std::time::Duration;

use crate::{models, operations as api, Error, NetworkLogOptions, Result, VM};

pub struct Copy<'a>(pub(crate) &'a VM);
pub struct Snapshots<'a>(pub(crate) &'a VM);
pub struct Stats<'a>(pub(crate) &'a VM);
pub struct Container<'a>(pub(crate) &'a VM);
pub struct Exposures<'a>(pub(crate) &'a VM);
pub struct Networks<'a>(pub(crate) &'a Client);
pub struct Profiles<'a>(pub(crate) &'a Client);
pub struct ProfileMcp<'a> {
    client: &'a Client,
    profile_id: String,
}

impl Copy<'_> {
    pub async fn from_vm(&self, path: &str) -> Result<Vec<u8>> {
        let params = api::DownloadVmFileParams {
            id: self.0.resolve().await?,
            path: path.into(),
        };
        api::download_vm_file(&self.0.client.transport, &params, self.0.client.options).await
    }

    pub async fn to_vm(&self, path: &str, data: Vec<u8>) -> Result<models::UploadResponse> {
        let params = api::UploadVmFileParams {
            id: self.0.resolve().await?,
            path: path.into(),
            body: data,
        };
        api::upload_vm_file(&self.0.client.transport, &params, self.0.client.options).await
    }
}

impl Snapshots<'_> {
    pub async fn list(&self) -> Result<models::SnapshotsList> {
        api::list_vm_snapshots(
            &self.0.client.transport,
            &api::ListVmSnapshotsParams {
                id: self.0.resolve().await?,
            },
            self.0.client.options,
        )
        .await
    }

    pub async fn status(&self) -> Result<models::SnapshotsStatus> {
        api::get_vm_snapshots_status(
            &self.0.client.transport,
            &api::GetVmSnapshotsStatusParams {
                id: self.0.resolve().await?,
            },
            self.0.client.options,
        )
        .await
    }
}

impl Stats<'_> {
    pub async fn summary(&self) -> Result<models::VmStatsSummaryResponse> {
        api::get_vm_stats_summary(
            &self.0.client.transport,
            &api::GetVmStatsSummaryParams {
                id: self.0.resolve().await?,
            },
            self.0.client.options,
        )
        .await
    }

    pub async fn details(&self) -> Result<models::VmStatsDetailResponse> {
        api::get_vm_stats_detail(
            &self.0.client.transport,
            &api::GetVmStatsDetailParams {
                id: self.0.resolve().await?,
            },
            self.0.client.options,
        )
        .await
    }
}

impl Container<'_> {
    pub async fn status(&self) -> Result<models::ContainerStatusResponse> {
        api::get_vm_container(
            &self.0.client.transport,
            &api::GetVmContainerParams {
                id: self.0.resolve().await?,
            },
            self.0.client.options,
        )
        .await
    }

    pub async fn wait(&self, interval: Duration) -> Result<models::ContainerStatusResponse> {
        if interval.is_zero() {
            return Err(Error::InvalidInput("container wait interval must be positive"));
        }
        loop {
            let status = self.status().await?;
            if !matches!(
                status.state,
                models::ContainerState::Pulling | models::ContainerState::Staging | models::ContainerState::Starting
            ) {
                return Ok(status);
            }
            tokio::time::sleep(interval).await;
        }
    }
}

impl Exposures<'_> {
    pub async fn create(&self, request: models::ExposureRequest) -> Result<models::ExposureInfo> {
        api::create_vm_exposure(
            &self.0.client.transport,
            &api::CreateVmExposureParams {
                id: self.0.resolve().await?,
                body: request,
            },
            self.0.client.options,
        )
        .await
    }

    pub async fn list(&self) -> Result<models::ExposureListResponse> {
        api::list_vm_exposures(
            &self.0.client.transport,
            &api::ListVmExposuresParams {
                id: self.0.resolve().await?,
            },
            self.0.client.options,
        )
        .await
    }

    pub async fn delete(&self, exposure_id: &str) -> Result<models::VmActionResponse> {
        api::delete_vm_exposure(
            &self.0.client.transport,
            &api::DeleteVmExposureParams {
                id: self.0.resolve().await?,
                exposure_id: exposure_id.into(),
            },
            self.0.client.options,
        )
        .await
    }
}

impl Networks<'_> {
    pub async fn create(&self, name: &str) -> Result<models::NetworkInfo> {
        api::create_network(
            &self.0.transport,
            &api::CreateNetworkParams {
                body: models::CreateNetworkRequest { name: name.into() },
            },
            self.0.options,
        )
        .await
    }

    pub async fn list(&self) -> Result<models::NetworkListResponse> {
        api::list_networks(&self.0.transport, self.0.options).await
    }

    pub async fn inspect(&self, network_id: &str) -> Result<models::NetworkInfo> {
        api::get_network(
            &self.0.transport,
            &api::GetNetworkParams { id: network_id.into() },
            self.0.options,
        )
        .await
    }

    pub async fn delete(&self, network_id: &str) -> Result<models::VmActionResponse> {
        api::delete_network(
            &self.0.transport,
            &api::DeleteNetworkParams { id: network_id.into() },
            self.0.options,
        )
        .await
    }

    pub async fn attach(&self, network_id: &str, vm_id: &str) -> Result<models::NetworkInfo> {
        api::attach_network_member(
            &self.0.transport,
            &api::AttachNetworkMemberParams {
                id: network_id.into(),
                vm_id: vm_id.into(),
            },
            self.0.options,
        )
        .await
    }

    pub async fn detach(&self, network_id: &str, vm_id: &str) -> Result<models::NetworkInfo> {
        api::detach_network_member(
            &self.0.transport,
            &api::DetachNetworkMemberParams {
                id: network_id.into(),
                vm_id: vm_id.into(),
            },
            self.0.options,
        )
        .await
    }

    pub async fn logs(&self, network_id: &str, options: NetworkLogOptions) -> Result<models::NetworkLogsResponse> {
        api::get_network_logs(
            &self.0.transport,
            &api::GetNetworkLogsParams {
                id: network_id.into(),
                cursor: options.cursor,
                limit: options.limit,
                vm: options.vm,
                connection: options.connection,
                r#type: options.event_type,
                decision: options.decision,
                since: options.since,
                until: options.until,
            },
            self.0.options,
        )
        .await
    }
}

impl<'a> Profiles<'a> {
    pub async fn list(&self) -> Result<models::ProfilesListResponse> {
        api::list_profiles(&self.0.transport, self.0.options).await
    }

    pub fn mcp(&self, profile_id: &str) -> ProfileMcp<'a> {
        ProfileMcp {
            client: self.0,
            profile_id: profile_id.into(),
        }
    }
}

impl ProfileMcp<'_> {
    pub async fn info(&self) -> Result<models::ProfileMcpInfoResponse> {
        api::get_profile_mcp_info(
            &self.client.transport,
            &api::GetProfileMcpInfoParams {
                profile_id: self.profile_id.clone(),
            },
            self.client.options,
        )
        .await
    }

    pub async fn servers(&self) -> Result<models::McpServersListResponse> {
        api::list_profile_mcp_servers(
            &self.client.transport,
            &api::ListProfileMcpServersParams {
                profile_id: self.profile_id.clone(),
            },
            self.client.options,
        )
        .await
    }

    pub async fn default_permission(&self) -> Result<models::McpDefaultPermissionResponse> {
        api::get_profile_mcp_default(
            &self.client.transport,
            &api::GetProfileMcpDefaultParams {
                profile_id: self.profile_id.clone(),
            },
            self.client.options,
        )
        .await
    }

    pub async fn tools(&self, server_id: &str) -> Result<models::McpToolsListResponse> {
        api::list_profile_mcp_tools(
            &self.client.transport,
            &api::ListProfileMcpToolsParams {
                profile_id: self.profile_id.clone(),
                server_id: server_id.into(),
            },
            self.client.options,
        )
        .await
    }

    pub async fn refresh(&self, server_id: &str) -> Result<models::McpRefreshResponse> {
        api::refresh_profile_mcp_server(
            &self.client.transport,
            &api::RefreshProfileMcpServerParams {
                profile_id: self.profile_id.clone(),
                server_id: server_id.into(),
            },
            self.client.options,
        )
        .await
    }

    pub async fn call(&self, server_id: &str, tool_id: &str, arguments: models::Value) -> Result<models::Value> {
        api::call_profile_mcp_tool(
            &self.client.transport,
            &api::CallProfileMcpToolParams {
                profile_id: self.profile_id.clone(),
                server_id: server_id.into(),
                tool_id: tool_id.into(),
                body: arguments,
            },
            self.client.options,
        )
        .await
    }
}
