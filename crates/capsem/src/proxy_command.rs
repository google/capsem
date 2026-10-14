use std::net::IpAddr;
use std::time::Duration;

use anyhow::Result;
use capsem_api::{
    CreateProxyRequest, CreateProxyResponse, ProxyHeartbeatResponse, ProxyLeaseRequest, StopProxyResponse,
};
use clap::Args;

use crate::client::{ApiResponse, UdsClient};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Args, Debug)]
pub(crate) struct ProxyArgs {
    /// Provider key from the effective built-in, settings, and corp policy
    #[arg(long, default_value = "openai")]
    pub(crate) provider: String,
    /// Address for the unauthenticated model API listener
    #[arg(long, default_value = "127.0.0.1")]
    pub(crate) bind: IpAddr,
    /// Model API port; zero lets the OS select an available port
    #[arg(long, default_value_t = 0)]
    pub(crate) port: u16,
}

pub(crate) async fn run(client: &UdsClient, args: &ProxyArgs) -> Result<()> {
    let response: ApiResponse<CreateProxyResponse> = client
        .post(
            "/proxies",
            &CreateProxyRequest {
                provider: args.provider.clone(),
                bind: args.bind.to_string(),
                port: args.port,
            },
        )
        .await?;
    let started = response.into_result()?;
    println!("Proxy session: {}", started.session_id);
    println!("Base URL: {}", started.base_url);
    println!("Press Ctrl-C to stop.");

    let request = ProxyLeaseRequest {
        lease_token: started.lease_token.clone(),
    };
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.tick().await;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let lease_result = loop {
        tokio::select! {
            _ = interrupt.recv() => break Ok(()),
            _ = terminate.recv() => break Ok(()),
            _ = heartbeat.tick() => {
                let renewed: Result<ApiResponse<ProxyHeartbeatResponse>> = client
                    .post(&format!("/proxies/{}/heartbeat", started.session_id), &request)
                    .await;
                match renewed.and_then(ApiResponse::into_result) {
                    Ok(_) => {}
                    Err(error) => break Err(error.context("standalone proxy lease heartbeat failed")),
                }
            }
        }
    };

    let stopped: Result<ApiResponse<StopProxyResponse>> = client
        .post(&format!("/proxies/{}/stop", started.session_id), &request)
        .await;
    let stop_result = stopped.and_then(ApiResponse::into_result).map(|_| ());
    match (lease_result, stop_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(error)) => Err(error.context("standalone proxy cleanup failed")),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup)) => Err(error.context(format!("standalone proxy cleanup also failed: {cleanup:#}"))),
    }
}
