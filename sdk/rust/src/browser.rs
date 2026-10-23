//! Client-local return path for a workload's browser sign-in callback.

use std::time::Duration;

use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::{Error, Result, VM};

mod plan;
mod preview;
mod server;
use plan::CallbackPlan;

impl VM {
    /// Retire metadata after a successful local browser opener. No URL is sent.
    /// A different VM process generation cannot be acknowledged by this request.
    pub async fn acknowledge_browser_request(&self, request: &crate::models::BrowserRequest) -> Result<()> {
        let body = crate::models::BrowserAcknowledgeRequest {
            owner_generation: request
                .owner_generation
                .clone()
                .ok_or(Error::InvalidInput("browser owner generation unavailable"))?,
            request_id: request.id,
        };
        body.validate().map_err(Error::InvalidInput)?;
        let id = self.resolve().await?;
        crate::operations::acknowledge_browser_request(
            &self.client.transport,
            &crate::operations::AcknowledgeBrowserRequestParams { id, body },
            crate::transport::CallOptions {
                timeout: Some(Duration::from_secs(2)),
            },
        )
        .await?;
        Ok(())
    }
}

/// A prepared, single-use callback on this client's loopback interface.
/// Keep it alive while the user's browser signs in. Drop cancels the listener
/// and schedules exposure cleanup; `close` also waits for that cleanup.
/// URLs, OAuth state and preview cookies never enter Debug or error messages.
pub struct BrowserCallback {
    cancel: CancellationToken,
    task: Option<tokio::task::JoinHandle<Result<()>>>,
}

impl std::fmt::Debug for BrowserCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserCallback").finish_non_exhaustive()
    }
}

impl Drop for BrowserCallback {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl BrowserCallback {
    /// Prepare BEFORE opening a browser request. Ordinary web links return
    /// `None`. A malformed or unavailable loopback callback fails closed.
    /// This uses the existing authenticated HTTP-preview transport, never a
    /// raw guest-port publication. The current preview service is local-only;
    /// a remote gateway is refused rather than binding on the wrong machine.
    pub async fn prepare(vm: &VM, browser_url: &str) -> Result<Option<Self>> {
        let Some(plan) = CallbackPlan::parse(browser_url)? else {
            return Ok(None);
        };
        if !vm.client.transport.is_loopback() {
            return Err(Error::InvalidInput(
                "browser callbacks require a client-reachable preview origin",
            ));
        }
        let listeners = plan.bind()?;
        let cancel = CancellationToken::new();
        let owner_cancel = cancel.clone();
        let vm = vm.clone();
        let (ready_tx, ready_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let port = match vm
                .ports()
                .open_with(
                    plan.port,
                    crate::PortOptions {
                        host: 0,
                        authenticate: true,
                    },
                )
                .await
            {
                Ok(port) => port,
                Err(_) => {
                    let _ = ready_tx.send(Err(Error::InvalidResponse("could not prepare callback preview")));
                    return Ok(());
                }
            };
            let result = match preview::Preview::connect(&port).await {
                Ok(preview) if !owner_cancel.is_cancelled() => {
                    if ready_tx.send(Ok(())).is_ok() {
                        server::run(listeners, plan, preview, owner_cancel, Duration::from_secs(600)).await
                    } else {
                        Ok(())
                    }
                }
                Ok(_) => {
                    let _ = ready_tx.send(Err(Error::Cancelled));
                    Ok(())
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                    Ok(())
                }
            };
            // Setup/cancellation retains the exact Port until cleanup completes.
            // Each SDK request has a finite deadline; no detached helper process.
            vm.ports()
                .close(&port)
                .await
                .map_err(|_| Error::InvalidResponse("callback preview cleanup failed"))?;
            result
        });
        let owner = Self {
            cancel,
            task: Some(task),
        };
        match ready_rx.await {
            Ok(Ok(())) => Ok(Some(owner)),
            Ok(Err(error)) => {
                owner.close().await?;
                Err(error)
            }
            Err(_) => {
                owner.close().await?;
                Err(Error::InvalidResponse("callback preparation ended"))
            }
        }
    }

    pub async fn close(mut self) -> Result<()> {
        self.cancel.cancel();
        self.task
            .take()
            .expect("callback task owned")
            .await
            .map_err(|_| Error::InvalidResponse("callback worker ended"))?
    }

    /// Delivery, cancellation or the ten-minute lifetime has ended and cleanup
    /// has completed. Clients can discard finished handles when opening again.
    pub fn is_finished(&self) -> bool {
        self.task.as_ref().is_none_or(tokio::task::JoinHandle::is_finished)
    }
}

#[cfg(test)]
mod tests;
