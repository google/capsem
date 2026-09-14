//! The service's private network requests: a TCP connection admitted to
//! this VM (`PrivateAccept`), plugging this VM's cable into a network's
//! switch (`LinkAttach`), and taking the cable down when it leaves
//! (`LinkDetach`). The first two answer with the handoff socket the asker
//! presents its token on; every refusal names its reason.
use super::*;

pub(super) fn handle(message: ServiceToProcess, job_store: Arc<JobStore>, output: mpsc::Sender<ProcessToService>) {
    tokio::spawn(async move {
        let response = match message {
            ServiceToProcess::PrivateAccept {
                id,
                token,
                network,
                network_name,
                source_vm,
                source_name,
                source_generation,
                source_address,
                source_port,
                port,
            } => {
                let accepted = (|| {
                    let network = capsem_core::security_engine::network::NetworkIdentity::parse(&network, network_name)
                        .map_err(anyhow::Error::msg)?;
                    let source = crate::private_handoff::source_vm(source_vm, source_name, source_generation);
                    let handoff = job_store.private.get().context("no private handoff on this owner")?;
                    handoff.expect(&token, network, source, (source_address, source_port).into(), port)?;
                    Ok::<_, anyhow::Error>(handoff.socket_path().to_string_lossy().into_owned())
                })();
                let (handoff_socket, error) = outcome(accepted);
                ProcessToService::PrivateAcceptResult {
                    id,
                    handoff_socket,
                    error,
                }
            }
            ServiceToProcess::LinkAttach {
                id,
                token,
                network,
                network_name,
                address,
                prefix,
            } => {
                let linked = async {
                    let network = capsem_core::security_engine::network::NetworkIdentity::parse(&network, network_name)
                        .map_err(anyhow::Error::msg)?;
                    let cables = job_store.cables.get().context("no cables on this owner")?;
                    cables.expect(&token, network, address, prefix).await?;
                    let handoff = job_store.private.get().context("no private handoff on this owner")?;
                    Ok::<_, anyhow::Error>(handoff.socket_path().to_string_lossy().into_owned())
                }
                .await;
                let (handoff_socket, error) = outcome(linked);
                ProcessToService::LinkAttachResult {
                    id,
                    handoff_socket,
                    error,
                }
            }
            ServiceToProcess::LinkDetach { id, network } => {
                let detached = async {
                    let network =
                        capsem_core::security_engine::network::NetworkIdentity::parse(&network, String::new())
                            .map_err(anyhow::Error::msg)?;
                    let cables = job_store.cables.get().context("no cables on this owner")?;
                    cables.detach(&network.id.to_string()).await
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
