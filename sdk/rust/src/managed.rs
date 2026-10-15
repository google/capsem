//! Finite ownership for newly created unnamed sessions, independent of borrowed VM handles.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::time::{Duration, SystemTime};

use capsem_crypto::{random, secrets::Zeroizing};
use futures::FutureExt;
use tokio_util::sync::CancellationToken;

use crate::{client::Client, models, operations as api, CreateOptions, Error, Hypervisor, Result, VM};

/// Recovery authority returned with a failed close. Debug never includes its capability.
pub struct ManagedSession {
    client: Client,
    request_id: String,
    token: Option<models::ManagedOwnershipToken>,
    ownership: Option<models::ManagedSessionOwnership>,
    vm_id: Option<String>,
}

impl std::fmt::Debug for ManagedSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedSession")
            .field("request_id", &self.request_id)
            .field("ownership", &self.ownership)
            .field("closed", &self.token.is_none())
            .finish()
    }
}

impl ManagedSession {
    fn new(client: Client) -> Result<Self> {
        let mut bytes = Zeroizing::new([0_u8; 32]);
        random::fill(bytes.as_mut()).map_err(|_| Error::Managed("ownership entropy unavailable"))?;
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let token: String = bytes
            .iter()
            .flat_map(|byte| {
                [
                    char::from(HEX[usize::from(byte >> 4)]),
                    char::from(HEX[usize::from(byte & 15)]),
                ]
            })
            .collect();
        Ok(Self {
            client,
            request_id: random::uuid_v4().to_string(),
            token: Some(models::ManagedOwnershipToken::new(token).map_err(Error::Managed)?),
            ownership: None,
            vm_id: None,
        })
    }

    /// Last validated metadata from this ownership generation.
    pub fn ownership(&self) -> Option<&models::ManagedSessionOwnership> {
        self.ownership.as_ref()
    }

    fn validate(&mut self, ownership: models::ManagedSessionOwnership) -> Result<()> {
        models::validate_managed_request_id(&ownership.generation).map_err(Error::Managed)?;
        if ownership.request_id != self.request_id
            || ownership.lease_seconds == 0
            || ownership.renew_after_seconds == 0
            || ownership.renew_after_seconds >= ownership.lease_seconds
            || humantime::parse_rfc3339(&ownership.expires_at)
                .map_err(|_| Error::Managed("invalid ownership expiry"))?
                <= SystemTime::now()
            || self
                .ownership
                .as_ref()
                .is_some_and(|known| known.generation != ownership.generation)
        {
            return Err(Error::Managed("invalid or changed ownership generation"));
        }
        self.ownership = Some(ownership);
        Ok(())
    }

    fn validate_vm(&mut self, response: &models::ProvisionResponse) -> Result<()> {
        if response.id.is_empty() || response.persistent || self.vm_id.as_ref().is_some_and(|id| *id != response.id) {
            return Err(Error::Managed("invalid or changed managed VM identity"));
        }
        self.vm_id = Some(response.id.clone());
        Ok(())
    }

    /// Await original-generation cleanup. Failed requests retain authority for explicit retry.
    pub async fn close(&mut self, outcome: models::ManagedSessionOutcome) -> Result<()> {
        let Some(token) = &self.token else {
            return Ok(());
        };
        let result = api::close_managed_session(
            &self.client.transport,
            &api::CloseManagedSessionParams {
                request_id: self.request_id.clone(),
                body: models::ManagedSessionCloseRequest {
                    ownership_token: token.clone(),
                    outcome,
                },
            },
            self.client.create_options(),
        )
        .await?;
        models::validate_managed_request_id(&result.generation).map_err(Error::Managed)?;
        if result.request_id != self.request_id
            || self
                .ownership
                .as_ref()
                .is_some_and(|known| known.generation != result.generation)
            || self
                .vm_id
                .as_ref()
                .is_some_and(|id| result.session_id.as_ref() != Some(id))
        {
            return Err(Error::Managed("close did not confirm original ownership"));
        }
        self.token = None;
        Ok(())
    }

    fn next_claim(&self) -> impl Future<Output = Result<models::ManagedSessionClaimResponse>> + Send + 'static {
        let client = self.client.clone();
        let params = api::ClaimManagedSessionParams {
            request_id: self.request_id.clone(),
            body: models::ManagedSessionClaimRequest {
                ownership_token: self.token.as_ref().expect("live owner").clone(),
            },
        };
        let delay = self.ownership.as_ref().map_or(Duration::from_millis(50), |meta| {
            Duration::from_secs(u64::from(meta.renew_after_seconds))
        });
        let remaining = self.ownership.as_ref().map(|meta| {
            // Validation admitted this timestamp. Zero remaining time expires immediately.
            humantime::parse_rfc3339(&meta.expires_at)
                .expect("validated expiry")
                .duration_since(SystemTime::now())
                .unwrap_or_default()
        });
        async move {
            let claim = async {
                tokio::time::sleep(delay).await;
                api::claim_managed_session(&client.transport, &params, client.options).await
            };
            match remaining {
                Some(remaining) => tokio::time::timeout(remaining, claim)
                    .await
                    .map_err(|_| Error::Managed("ownership lease expired"))?,
                None => claim.await,
            }
        }
    }

    fn claimed(&mut self, reply: models::ManagedSessionClaimResponse) -> Result<()> {
        self.validate(reply.ownership)?;
        if !matches!(
            reply.state,
            models::ManagedSessionState::Pending | models::ManagedSessionState::Active
        ) {
            return Err(Error::Managed("ownership is no longer live"));
        }
        if let Some(response) = reply.session {
            self.validate_vm(&response)?;
        }
        Ok(())
    }

    async fn created(&mut self, body: models::ProvisionRequest, cancel: &CancellationToken) -> Result<VM> {
        let client = self.client.clone();
        let params = api::CreateVmParams { body };
        let create = api::create_vm(&client.transport, &params, client.create_options());
        tokio::pin!(create);
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(Error::Cancelled),
                result = &mut create => {
                    let response = result?;
                    self.validate(response.ownership.clone().ok_or(Error::Managed("create omitted ownership"))?)?;
                    self.validate_vm(&response)?;
                    return VM::created(client.clone(), response.id, response.name, Some(params.body.container.is_some()));
                },
                result = self.next_claim() => match result {
                    Err(Error::Http { status:404, .. }) if self.ownership.is_none() => {},
                    result => self.claimed(result?)?,
                },
            }
        }
    }

    async fn run<T, F, Fut>(&mut self, body: models::ProvisionRequest, cancel: &CancellationToken, work: F) -> Result<T>
    where
        F: FnOnce(VM, CancellationToken) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let vm = self.created(body, cancel).await?;
        // Invocation and polling both sit inside the unwind boundary; a user panic still closes.
        let body = AssertUnwindSafe(async { work(vm, cancel.clone()).await }).catch_unwind();
        tokio::pin!(body);
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(Error::Cancelled),
                result = &mut body => return result.map_err(|_| Error::Managed("managed callback panicked"))?,
                reply = self.next_claim() => self.claimed(reply?)?,
            }
        }
    }
}

#[cfg(test)]
mod tests;

impl Hypervisor {
    /// Create an unnamed ephemeral VM, renew ownership, and await cleanup after the callback.
    /// Dropping this future signals cancellation; its owned task continues cleanup on the runtime.
    pub async fn with_managed<T, F, Fut>(&self, options: CreateOptions, work: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(VM, CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
    {
        if options.name.is_some() {
            return Err(Error::InvalidInput("managed sessions must be unnamed"));
        }
        let mut body = Self::provision_request(options)?;
        let mut session = ManagedSession::new(self.managed_client())?;
        body.managed = Some(models::ManagedSessionCreate {
            request_id: session.request_id.clone(),
            ownership_token: session.token.as_ref().expect("new owner").clone(),
        });
        let cancel = CancellationToken::new();
        let guard = cancel.clone().drop_guard();
        let task = tokio::spawn(async move {
            let result = session.run(body, &cancel, work).await;
            let outcome = match &result {
                Ok(_) => models::ManagedSessionOutcome::Completed,
                Err(Error::Cancelled) => models::ManagedSessionOutcome::Cancelled,
                Err(_) => models::ManagedSessionOutcome::Failed,
            };
            cancel.cancel();
            if let Err(source) = session.close(outcome).await {
                return Err(Error::ManagedCleanup {
                    source: Box::new(source),
                    session: Box::new(session),
                });
            }
            result
        });
        let result = task
            .await
            .map_err(|_| Error::Managed("ownership task terminated unexpectedly"))?;
        drop(guard);
        result
    }
}
