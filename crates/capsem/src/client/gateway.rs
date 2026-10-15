//! Local discovery and presentation around the shared authenticated SDK.

use super::{ApiResponse, ConnectMode, DirectServiceLifetime, ListResponse, ServiceAdminClient};
use anyhow::{Context, Result};
use capsem_sdk::transport::{CallOptions, MediaType, Request, Transport};
use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;

pub struct GatewayClient {
    bootstrap: ServiceAdminClient,
    connection: tokio::sync::OnceCell<Transport>,
}

impl GatewayClient {
    pub fn new(socket: PathBuf, auto_launch: bool) -> Self {
        Self::with_direct_service_lifetime(socket, auto_launch, DirectServiceLifetime::Persistent)
    }

    pub(crate) fn with_direct_service_lifetime(
        socket: PathBuf,
        auto_launch: bool,
        lifetime: DirectServiceLifetime,
    ) -> Self {
        Self {
            bootstrap: ServiceAdminClient::with_direct_service_lifetime(socket, auto_launch, lifetime),
            connection: tokio::sync::OnceCell::new(),
        }
    }

    pub(super) fn transport(&self) -> futures::future::BoxFuture<'_, Result<&Transport>> {
        Box::pin(async move {
            self.connection
                .get_or_try_init(|| async {
                    let explicit_gateway = std::env::var_os("CAPSEM_GATEWAY_URL").is_some();
                    if self.bootstrap.auto_launch && !explicit_gateway {
                        self.bootstrap.ensure_service().await?;
                    }
                    let run_dir = self
                        .bootstrap
                        .uds_path
                        .parent()
                        .context("service socket has no run directory")?;
                    if !self.bootstrap.auto_launch
                        && !explicit_gateway
                        && crate::gateway_url_from_runtime(run_dir).await.is_none()
                    {
                        drop(
                            self.bootstrap
                                .connect_with_timeout(ConnectMode::FailFast)
                                .await
                                .with_context(|| {
                                    format!("cannot connect to service at {}", self.bootstrap.uds_path.display())
                                })?,
                        );
                    }
                    let url = crate::wait_for_gateway_url(run_dir).await?;
                    let token = match std::env::var("CAPSEM_GATEWAY_TOKEN") {
                        Ok(token) => token,
                        Err(std::env::VarError::NotPresent) => {
                            let endpoint = reqwest::Url::parse(&url)?;
                            let local = endpoint.host_str().is_some_and(|host| {
                                host == "localhost"
                                    || host
                                        .parse::<std::net::IpAddr>()
                                        .is_ok_and(|address| address.is_loopback())
                            });
                            anyhow::ensure!(local, "a remote gateway requires an explicit CAPSEM_GATEWAY_TOKEN");
                            let path = run_dir.join("gateway.token");
                            let bytes = tokio::task::spawn_blocking(move || {
                                capsem_foundation::unix::fs::read_regular_file_no_follow(&path)
                            })
                            .await
                            .context("read gateway token task")?
                            .context("gateway token unavailable; start or restart the local service")?;
                            String::from_utf8(bytes).context("gateway token is not UTF-8")?
                        }
                        Err(_) => anyhow::bail!("gateway token environment is not UTF-8"),
                    };
                    Transport::new(&url, token.trim(), CallOptions::command_timeout(None)).map_err(anyhow::Error::new)
                })
                .await
        })
    }

    pub async fn request<T: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<T>,
    ) -> Result<R> {
        let body = body.as_ref().map(serde_json::to_vec).transpose()?;
        let (bytes, _) = self.request_bytes(method, path, body, Some("application/json")).await?;
        serde_json::from_slice(&bytes).context("failed to parse gateway response")
    }

    pub async fn get<R: DeserializeOwned>(&self, path: &str) -> Result<R> {
        self.request::<(), R>("GET", path, None).await
    }

    pub async fn post<T: Serialize, R: DeserializeOwned>(&self, path: &str, body: T) -> Result<R> {
        self.request("POST", path, Some(body)).await
    }

    pub async fn delete<R: DeserializeOwned>(&self, path: &str) -> Result<R> {
        self.request::<(), R>("DELETE", path, None).await
    }

    pub async fn request_bytes(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
        content_type: Option<&str>,
    ) -> Result<(Vec<u8>, Option<String>)> {
        let method = reqwest::Method::from_bytes(method.as_bytes())?;
        anyhow::ensure!(
            path.starts_with('/') && !path.contains('#'),
            "invalid CLI operation path"
        );
        let (path, raw_query) = path.split_once('?').unwrap_or((path, ""));
        let parts = path[1..]
            .split('/')
            .enumerate()
            .map(|(index, value)| Ok((format!("part{index}"), urlencoding::decode(value)?.into_owned())))
            .collect::<Result<Vec<_>>>()?;
        let mut template = String::new();
        for (key, _) in &parts {
            write!(template, "/{{{key}}}").expect("writing a String cannot fail");
        }
        let parameters = parts
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect::<Vec<_>>();
        let query_url = reqwest::Url::parse(&format!("http://query.invalid/?{raw_query}"))?;
        let query = query_url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Vec<_>>();
        let query_refs = query
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect::<Vec<_>>();
        let content_type = match content_type {
            None | Some("application/json") => MediaType::Json,
            Some("application/octet-stream") => MediaType::Binary,
            Some("application/gzip") => MediaType::Gzip,
            Some(_) => anyhow::bail!("unsupported CLI request content type"),
        };
        let options = if method == reqwest::Method::GET {
            CallOptions {
                timeout: Some(Duration::from_secs(30)),
            }
        } else if path == "/run" || path.ends_with("/exec") {
            let bytes = body.as_deref().context("command request requires a body")?;
            let timeout_secs = if path == "/run" {
                serde_json::from_slice::<capsem_api::RunRequest>(bytes)?.timeout_secs
            } else {
                serde_json::from_slice::<capsem_api::ExecRequest>(bytes)?.timeout_secs
            };
            CallOptions {
                timeout: Some(CallOptions::command_timeout(timeout_secs)),
            }
        } else {
            CallOptions::default()
        };
        let response = self
            .transport()
            .await?
            .request_response(
                method,
                &template,
                Request {
                    parameters: &parameters,
                    query: &query_refs,
                    body,
                    content_type,
                    accept: content_type,
                    options,
                },
            )
            .await
            .map_err(gateway_error)?;
        Ok((response.body, response.content_type))
    }

    pub async fn listed_session_id(&self, typed: &str) -> Result<Option<String>> {
        let response: ApiResponse<ListResponse> = self.get("/vms/list").await?;
        Ok(response
            .into_result()?
            .sandboxes
            .into_iter()
            .find(|session| session.id == typed || session.name.as_deref() == Some(typed))
            .map(|session| session.id))
    }

    pub(crate) async fn finish_direct_request<T>(&self, result: Result<T>) -> Result<T> {
        self.bootstrap.finish_direct_request(result).await
    }
}

pub(super) fn gateway_error(error: capsem_sdk::Error) -> anyhow::Error {
    if let capsem_sdk::Error::Http { status, body } = &error {
        if let Ok(reason) = serde_json::from_slice::<capsem_api::ErrorResponse>(body) {
            return anyhow::anyhow!("gateway HTTP {status}: {}", reason.error);
        }
        return anyhow::anyhow!("gateway HTTP {status}: {}", String::from_utf8_lossy(body));
    }
    anyhow::Error::new(error)
}

#[cfg(test)]
mod tests;
