//! The service's network cable requests: plugging this VM's cable into a
//! network's switch (`LinkAttach`), answered with the seat the service
//! presents its token on, and taking the cable down when the VM leaves
//! (`LinkDetach`). Every refusal names its reason.
use super::*;

pub(super) fn handle(message: ServiceToProcess, job_store: Arc<JobStore>, output: mpsc::Sender<ProcessToService>) {
    tokio::spawn(async move {
        let response = match message {
            ServiceToProcess::LinkAttach {
                id,
                token,
                network,
                network_name,
                address,
                prefix,
                generation,
            } => {
                let linked = async {
                    let network = capsem_core::security_engine::network::NetworkIdentity::parse(&network, network_name)
                        .map_err(anyhow::Error::msg)?;
                    let cables = job_store.cables.get().context("no cables on this owner")?;
                    cables.expect(&token, network, address, prefix, generation).await?;
                    let seat = job_store.cable_seat.get().context("no cable seat on this owner")?;
                    Ok::<_, anyhow::Error>(seat.to_string_lossy().into_owned())
                }
                .await;
                let (handoff_socket, error) = outcome(linked);
                ProcessToService::LinkAttachResult {
                    id,
                    handoff_socket,
                    error,
                }
            }
            ServiceToProcess::LinkDetach {
                id,
                network,
                generation,
            } => {
                let detached = async {
                    let network =
                        capsem_core::security_engine::network::NetworkIdentity::parse(&network, String::new())
                            .map_err(anyhow::Error::msg)?;
                    let cables = job_store.cables.get().context("no cables on this owner")?;
                    cables.detach(&network.id.to_string(), generation).await
                }
                .await;
                ProcessToService::LinkDetachResult {
                    id,
                    error: detached.err().map(|error| format!("{error:#}")),
                }
            }
            other => {
                error!(?other, "not a private network request");
                return;
            }
        };
        capsem_core::try_send!("private_result", output.send(response).await);
    });
}

/// The socket to present the token on, or an empty path and the reason.
fn outcome(result: Result<String>) -> (String, Option<String>) {
    match result {
        Ok(handoff_socket) => (handoff_socket, None),
        Err(error) => (String::new(), Some(format!("{error:#}"))),
    }
}
