use crate::client::Client;

use crate::{models, operations as api, DiagnosticOptions, NetworkLogOptions, Result, TriageOptions, VM};

/// Paths are the guest's: `/root/x` in a VM, `/workspace/x` in its container,
/// or `x` relative to the workspace; see [`Files::exact`].
pub struct Files<'a>(pub(crate) &'a VM, pub(crate) bool);
pub struct Stats<'a>(pub(crate) &'a VM);
pub struct Container<'a>(pub(crate) &'a VM);
pub struct Ports<'a>(pub(crate) &'a VM);
pub struct VmNetworks<'a>(pub(crate) &'a VM);
pub struct Networks<'a>(pub(crate) &'a Client);
pub struct Debug<'a>(pub(crate) &'a Client);
pub struct Profiles<'a>(pub(crate) &'a Client);
pub struct ProfileMcp<'a> {
    client: &'a Client,
    profile_id: String,
}
pub struct ProfileMcpServer<'a> {
    client: &'a Client,
    profile_id: String,
    pub info: models::McpServerInfoResponse,
}
pub struct McpTools<'a> {
    client: &'a Client,
    profile_id: String,
    server_id: String,
}

impl Files<'_> {
    /// Take every path literally, relative to the workspace root, even when it
    /// is absolute: `/root/x` is then the workspace's `root/x`.
    #[must_use]
    pub const fn exact(self) -> Self {
        Self(self.0, true)
    }

    fn exact_param(&self) -> Option<bool> {
        self.1.then_some(true)
    }

    pub async fn read(&self, path: &str) -> Result<Vec<u8>> {
        let params = api::DownloadVmFileParams {
            id: self.0.resolve().await?,
            path: path.into(),
            exact: self.exact_param(),
        };
        api::download_vm_file(&self.0.client.transport, &params, self.0.client.options).await
    }

    pub async fn write(&self, path: &str, data: Vec<u8>) -> Result<models::UploadResponse> {
        let params = api::UploadVmFileParams {
            id: self.0.resolve().await?,
            path: path.into(),
            exact: self.exact_param(),
            body: data,
        };
        api::upload_vm_file(&self.0.client.transport, &params, self.0.client.options).await
    }

    /// An empty `path` lists the workspace root.
    pub async fn list(&self, path: &str, depth: Option<i64>) -> Result<models::FileListResponse> {
        let params = api::ListVmFilesParams {
            id: self.0.resolve().await?,
            path: (!path.is_empty()).then(|| path.to_owned()),
            depth,
            exact: self.exact_param(),
        };
        api::list_vm_files(&self.0.client.transport, &params, self.0.client.options).await
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

#[derive(Clone, PartialEq, Eq)]
pub struct Port {
    pub id: String,
    pub guest: u16,
    pub host: Option<u16>,
    pub authenticate: bool,
    pub url: Option<String>,
    pub bootstrap_token: Option<String>,
    pub expires_in_seconds: Option<u16>,
}

/// The bootstrap token opens a browser session on the workload, so it is
/// redacted the way [`crate::Registry`] redacts its credentials.
impl std::fmt::Debug for Port {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Port")
            .field("id", &self.id)
            .field("guest", &self.guest)
            .field("host", &self.host)
            .field("authenticate", &self.authenticate)
            .field("url", &self.url)
            .field(
                "bootstrap_token",
                &if self.bootstrap_token.is_some() {
                    "<redacted>"
                } else {
                    "<none>"
                },
            )
            .field("expires_in_seconds", &self.expires_in_seconds)
            .finish()
    }
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
                    id: id.clone(),
                    exposure_id: port.id.clone(),
                },
                self.0.client.options,
            )
            .await;
            let session = match session {
                Ok(session) => session,
                Err(error) => {
                    // The caller never receives a Port to close, so remove the
                    // exposure best-effort and report the session failure.
                    let _ = api::delete_vm_exposure(
                        &self.0.client.transport,
                        &api::DeleteVmExposureParams {
                            id,
                            exposure_id: port.id,
                        },
                        self.0.client.options,
                    )
                    .await;
                    return Err(error);
                }
            };
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

    pub async fn list(&self) -> Result<Vec<models::NetworkInfo>> {
        Ok(api::list_networks(&self.0.transport, self.0.options).await?.networks)
    }

    pub async fn inspect(&self, network_id: &str) -> Result<models::NetworkInfo> {
        api::get_network(
            &self.0.transport,
            &api::GetNetworkParams { id: network_id.into() },
            self.0.options,
        )
        .await
    }

    pub async fn delete(&self, network: &models::NetworkInfo) -> Result<models::VmActionResponse> {
        api::delete_network(
            &self.0.transport,
            &api::DeleteNetworkParams { id: network.id.clone() },
            self.0.options,
        )
        .await
    }

    pub async fn logs(
        &self,
        network: &models::NetworkInfo,
        options: NetworkLogOptions,
    ) -> Result<models::NetworkLogsResponse> {
        api::get_network_logs(
            &self.0.transport,
            &api::GetNetworkLogsParams {
                id: network.id.clone(),
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

impl VmNetworks<'_> {
    pub async fn list(&self) -> Result<Vec<models::NetworkInfo>> {
        let vm_id = self.0.resolve().await?;
        let response = api::list_networks(&self.0.client.transport, self.0.client.options).await?;
        Ok(response
            .networks
            .into_iter()
            .filter(|network| network.members.iter().any(|member| member.vm_id == vm_id))
            .collect())
    }

    pub async fn attach(&self, network: &models::NetworkInfo) -> Result<models::NetworkInfo> {
        api::attach_network_member(
            &self.0.client.transport,
            &api::AttachNetworkMemberParams {
                id: network.id.clone(),
                vm_id: self.0.resolve().await?,
            },
            self.0.client.options,
        )
        .await
    }

    pub async fn detach(&self, network: &models::NetworkInfo) -> Result<models::NetworkInfo> {
        api::detach_network_member(
            &self.0.client.transport,
            &api::DetachNetworkMemberParams {
                id: network.id.clone(),
                vm_id: self.0.resolve().await?,
            },
            self.0.client.options,
        )
        .await
    }
}

impl Debug<'_> {
    pub async fn panics(&self, options: DiagnosticOptions) -> Result<models::PanicsResponse> {
        api::get_panics(
            &self.0.transport,
            &api::GetPanicsParams {
                since: options.since,
                limit: options.limit,
                id: None,
            },
            self.0.options,
        )
        .await
    }

    pub async fn triage(&self, options: TriageOptions) -> Result<models::TriageResponse> {
        api::get_triage(
            &self.0.transport,
            &api::GetTriageParams {
                since: options.since,
                limit: options.limit,
                id: options.vm_id,
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

    pub fn mcp(&self, profile: &models::ProfileSummary) -> ProfileMcp<'a> {
        ProfileMcp {
            client: self.0,
            profile_id: profile.id.clone(),
        }
    }
}

impl<'a> ProfileMcp<'a> {
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

    pub async fn get(&self, name: &str) -> Result<ProfileMcpServer<'a>> {
        let matches = self
            .servers()
            .await?
            .0
            .into_iter()
            .filter(|server| server.name == name)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(crate::Error::InvalidInput("MCP server name must resolve exactly once"));
        }
        Ok(ProfileMcpServer {
            client: self.client,
            profile_id: self.profile_id.clone(),
            info: matches.into_iter().next().expect("one MCP server matched"),
        })
    }
}

impl<'a> ProfileMcpServer<'a> {
    pub fn tools(&self) -> McpTools<'a> {
        McpTools {
            client: self.client,
            profile_id: self.profile_id.clone(),
            server_id: self.info.name.clone(),
        }
    }

    pub async fn refresh(&self) -> Result<models::McpRefreshResponse> {
        api::refresh_profile_mcp_server(
            &self.client.transport,
            &api::RefreshProfileMcpServerParams {
                profile_id: self.profile_id.clone(),
                server_id: self.info.name.clone(),
            },
            self.client.options,
        )
        .await
    }
}

impl McpTools<'_> {
    pub async fn list(&self) -> Result<models::McpToolsListResponse> {
        api::list_profile_mcp_tools(
            &self.client.transport,
            &api::ListProfileMcpToolsParams {
                profile_id: self.profile_id.clone(),
                server_id: self.server_id.clone(),
            },
            self.client.options,
        )
        .await
    }

    pub async fn call(&self, name: &str, arguments: models::Value) -> Result<models::Value> {
        api::call_profile_mcp_tool(
            &self.client.transport,
            &api::CallProfileMcpToolParams {
                profile_id: self.profile_id.clone(),
                server_id: self.server_id.clone(),
                tool_id: name.into(),
                body: arguments,
            },
            self.client.options,
        )
        .await
    }
}
