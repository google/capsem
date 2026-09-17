use crate::client::Client;

use crate::{models, operations as api, NetworkLogOptions, Result, VM};

pub struct Copy<'a>(pub(crate) &'a VM);
pub struct Snapshots<'a>(pub(crate) &'a VM);
pub struct Stats<'a>(pub(crate) &'a VM);
pub struct Container<'a>(pub(crate) &'a VM);
pub struct Ports<'a>(pub(crate) &'a VM);
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    pub id: String,
    pub guest: u16,
    pub host: Option<u16>,
    pub authenticate: bool,
    pub url: Option<String>,
    pub bootstrap_token: Option<String>,
    pub expires_in_seconds: Option<u16>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PortOptions {
    pub host: u16,
    pub authenticate: bool,
}

impl Port {
    fn from_exposure(exposure: models::ExposureInfo) -> Self {
        Self {
            id: exposure.id,
            guest: exposure.guest_port,
            host: exposure.host_port,
            authenticate: exposure.access == models::ExposureAccess::HttpPreview,
            url: None,
            bootstrap_token: None,
            expires_in_seconds: None,
        }
    }
}

impl Ports<'_> {
    pub async fn open(&self, guest: u16) -> Result<Port> {
        self.open_with(guest, PortOptions::default()).await
    }

    pub async fn open_with(&self, guest: u16, options: PortOptions) -> Result<Port> {
        if guest == 0 {
            return Err(crate::Error::InvalidInput("guest port must be between 1 and 65535"));
        }
        if options.authenticate && options.host != 0 {
            return Err(crate::Error::InvalidInput(
                "authenticated ports cannot select a host port",
            ));
        }
        let id = self.0.resolve().await?;
        let exposure = api::create_vm_exposure(
            &self.0.client.transport,
            &api::CreateVmExposureParams {
                id: id.clone(),
                body: models::ExposureRequest {
                    guest_port: guest,
                    host_port: options.host,
                    target: self.0.port_target().await?,
                    access: if options.authenticate {
                        models::ExposureAccess::HttpPreview
                    } else {
                        models::ExposureAccess::LoopbackTcp
                    },
                },
            },
            self.0.client.options,
        )
        .await?;
        let mut port = Port::from_exposure(exposure);
        port.authenticate = options.authenticate;
        if options.authenticate {
            let session = api::create_vm_preview_session(
                &self.0.client.transport,
                &api::CreateVmPreviewSessionParams {
                    id,
                    exposure_id: port.id.clone(),
                },
                self.0.client.options,
            )
            .await?;
            port.host = None;
            port.url = Some(session.url);
            port.bootstrap_token = Some(session.bootstrap_token);
            port.expires_in_seconds = Some(session.expires_in_seconds);
        }
        Ok(port)
    }

    pub async fn list(&self) -> Result<Vec<Port>> {
        let response = api::list_vm_exposures(
            &self.0.client.transport,
            &api::ListVmExposuresParams {
                id: self.0.resolve().await?,
            },
            self.0.client.options,
        )
        .await?;
        Ok(response.exposures.into_iter().map(Port::from_exposure).collect())
    }

    pub async fn close(&self, port: &Port) -> Result<models::VmActionResponse> {
        api::delete_vm_exposure(
            &self.0.client.transport,
            &api::DeleteVmExposureParams {
                id: self.0.resolve().await?,
                exposure_id: port.id.clone(),
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
    pub async fn list(&self) -> Result<Vec<models::ProfileSummary>> {
        Ok(api::list_profiles(&self.0.transport, self.0.options).await?.profiles)
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
