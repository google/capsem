use super::*;

impl Registry {
    pub(super) fn creation_failed(&self, ticket: &Ticket) -> Result<()> {
        let _lease = self.lease()?;
        let mut record = self.ticket_record(ticket)?;
        if record.state == State::Creating {
            record.state = State::Unknown;
            self.write(&record)?;
        }
        Ok(())
    }
    /// Authenticated observation only. This does not grant a create ticket or
    /// renew a lifetime; the service must reconcile before any renewal.
    pub fn inspect(&self, request: Uuid, capability: &Capability) -> Result<Option<Snapshot>> {
        ensure!(!request.is_nil(), "managed request identity is nil");
        let _lease = self.lease()?;
        let Some(record) = self.read(request)? else {
            return Ok(None);
        };
        ensure!(
            blake3::Hash::from(record.capability_hash) == capability.hash(request),
            "managed capability mismatch"
        );
        Ok(Some(Snapshot::from(&record)))
    }

    /// Service-startup reconciliation after previous continuation tasks have
    /// been retired. This trusted private-store operation is never a client
    /// claim handler. Unknown records cannot authorize another create.
    pub fn recover_interrupted(&self, request: Uuid) -> Result<Option<(Ticket, Snapshot)>> {
        ensure!(!request.is_nil(), "managed request identity is nil");
        let _lease = self.lease()?;
        let Some(mut record) = self.read(request)? else {
            return Ok(None);
        };
        if matches!(record.state, State::Reserved | State::Creating | State::Active) {
            record.state = State::Unknown;
            self.write(&record)?;
        }
        self.runtime_deadlines()?.remove(&request);
        Ok(Some((
            Ticket {
                request,
                generation: record.generation,
            },
            Snapshot::from(&record),
        )))
    }

    /// Bind an authoritative live VM discovered during recovery for cleanup.
    /// Recovery never activates it or substitutes a different generation.
    pub fn reconcile_vm(&self, ticket: &Ticket, binding: VmBinding) -> Result<Snapshot> {
        let _lease = self.lease()?;
        let mut record = self.ticket_record(ticket)?;
        ensure!(
            matches!(record.state, State::Unknown | State::Closing),
            "managed recovery is not pending"
        );
        if let Some(existing) = &record.vm {
            ensure!(existing == &binding, "managed VM generation cannot be rebound");
        }
        record.vm = Some(binding);
        record.state = State::Closing;
        self.write(&record)?;
        self.runtime_deadlines()?.remove(&record.request);
        Ok(Snapshot::from(&record))
    }

    /// A service-owned absence barrier: previous creation tasks are retired,
    /// authoritative VM ownership has been checked, and orphaned session bytes
    /// and grants have been cleaned. A lookup miss alone is insufficient.
    pub fn confirm_absent(&self, ticket: &Ticket) -> Result<Snapshot> {
        let _lease = self.lease()?;
        let mut record = self.ticket_record(ticket)?;
        ensure!(
            matches!(record.state, State::Unknown | State::Closing),
            "managed absence reconciliation is not pending"
        );
        ensure!(
            record.vm.is_none(),
            "bound managed VM requires matching cleanup completion"
        );
        record.state = State::Closed;
        self.write(&record)?;
        self.runtime_deadlines()?.remove(&record.request);
        Ok(Snapshot::from(&record))
    }
}
