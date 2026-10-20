//! Cleanup-only startup adapters. A durable absence of spawn intent proves
//! that the write-ahead prepare barrier never authorized a VM or its grants.
use super::*;

pub(crate) fn effects(
    state: Arc<ServiceState>,
    snapshot: &Snapshot,
    authority: Arc<capsem_credentials::GrantAuthority>,
) -> Result<Arc<dyn Effects>> {
    if snapshot.vm().is_some() || snapshot.spawn_intent().is_some() {
        return Ok(Arc::new(ManagedLifecycle::for_recovery_with_broker(
            state, snapshot, authority,
        )?));
    }
    Ok(Arc::new(Unprepared {
        request: snapshot.request(),
        generation: snapshot.generation(),
    }))
}

struct Unprepared {
    request: uuid::Uuid,
    generation: uuid::Uuid,
}

impl Effects for Unprepared {
    fn is_live(&self, _binding: &VmBinding) -> Result<bool> {
        anyhow::bail!("unprepared reservation has no VM authority")
    }

    fn create(
        &self,
        _ticket: Ticket,
        _cancel: CancellationToken,
        _reporter: BindingReporter,
    ) -> EffectFuture<VmBinding> {
        Box::pin(async { anyhow::bail!("startup recovery cannot create") })
    }

    fn cleanup(&self, _ticket: Ticket, _binding: VmBinding) -> EffectFuture<()> {
        Box::pin(async { anyhow::bail!("unprepared reservation cannot clean a VM") })
    }

    fn reconcile(&self, ticket: Ticket, intent: Option<VmBinding>) -> EffectFuture<Option<VmBinding>> {
        let expected = (self.request, self.generation);
        Box::pin(async move {
            anyhow::ensure!(
                (ticket.request(), ticket.generation()) == expected,
                "unprepared recovery ownership changed"
            );
            anyhow::ensure!(intent.is_none(), "unprepared recovery received a spawn intent");
            Ok(None)
        })
    }
}

#[cfg(test)]
mod tests;
