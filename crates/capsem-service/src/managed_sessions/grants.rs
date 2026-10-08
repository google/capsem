//! The lifecycle's original registry scope and spawn identity own retirement.
use super::*;
use capsem_credentials::{GrantAuthority, GrantSession};

struct BrokerRetirement {
    authority: Arc<GrantAuthority>,
    binding: VmBinding,
    work: Arc<Work>,
}

pub(super) fn retirement(
    authority: Arc<GrantAuthority>,
    binding: &VmBinding,
    work: &Arc<Work>,
) -> Arc<dyn GrantRetirement> {
    Arc::new(BrokerRetirement {
        authority,
        binding: binding.clone(),
        work: Arc::clone(work),
    })
}

pub(super) fn bound_session(ticket: &Ticket, binding: &VmBinding, work: &Work) -> Result<GrantSession> {
    anyhow::ensure!(
        work.scope.get() == Some(&(ticket.request(), ticket.generation())),
        "managed credential ownership changed"
    );
    let session = uuid::Uuid::parse_str(binding.id()).context("managed credential session is not canonical")?;
    anyhow::ensure!(
        !session.is_nil() && session.to_string() == binding.id(),
        "managed credential session is not canonical"
    );
    Ok(GrantSession::new(
        *session.as_bytes(),
        *ticket.generation().as_bytes(),
        *binding.generation().as_bytes(),
    )?)
}

impl GrantRetirement for BrokerRetirement {
    fn revoke(&self, ticket: Ticket, binding: VmBinding) -> EffectFuture<()> {
        let authority = Arc::clone(&self.authority);
        let work = Arc::clone(&self.work);
        let expected = self.binding.clone();
        Box::pin(async move {
            anyhow::ensure!(binding == expected, "managed credential spawn changed");
            bound_session(&ticket, &binding, &work)?;
            let session = uuid::Uuid::parse_str(binding.id())?;
            // Terminal close retires all this owner's runtime and preserved
            // grants. The account connection belongs to the user and survives.
            authority.revoke_owner(*session.as_bytes(), *ticket.generation().as_bytes())?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests;
