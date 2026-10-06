use super::*;

impl Registry {
    /// The only permission to perform a create side effect. Persist before
    /// spawning; a repeat or a close racing this operation never admits again.
    pub fn begin_create(&self, ticket: &Ticket, clock: LeaseClock) -> Result<bool> {
        let _lease = self.lease()?;
        let mut record = self.ticket_record(ticket)?;
        if record.state != State::Reserved {
            return Ok(false);
        }
        ensure!(record.lease.is_some(), "managed lease was not initialized");
        let mut deadlines = self.runtime_deadlines()?;
        if self.expire_if_due(&mut record, clock, &mut deadlines)? {
            return Ok(false);
        }
        record.state = State::Creating;
        self.write(&record)?;
        Ok(true)
    }

    /// Persist the close intent, including when it arrives before reservation.
    /// A Creating record stays Closing until its outcome has been reconciled.
    pub fn close(&self, request: Uuid, capability: &Capability) -> Result<Snapshot> {
        ensure!(!request.is_nil(), "managed request identity is nil");
        let _lease = self.lease()?;
        let mut record = match self.read(request)? {
            Some(record) => {
                ensure!(
                    blake3::Hash::from(record.capability_hash) == capability.hash(request),
                    "managed capability mismatch"
                );
                record
            }
            None => Record {
                schema_version: 1,
                request,
                generation: Uuid::new_v4(),
                capability_hash: *capability.hash(request).as_bytes(),
                state: State::Closed,
                vm: None,
                lease: None,
            },
        };
        record.state = match record.state {
            State::Reserved | State::Closed => State::Closed,
            State::Creating | State::Active | State::Closing | State::Unknown => State::Closing,
        };
        self.write(&record)?;
        self.runtime_deadlines()?.remove(&request);
        Ok(Snapshot::from(&record))
    }

    /// A late create result after close is bound for cleanup, never activated.
    pub fn bind_created(&self, ticket: &Ticket, binding: VmBinding, clock: LeaseClock) -> Result<Snapshot> {
        let _lease = self.lease()?;
        let mut record = self.ticket_record(ticket)?;
        ensure!(
            matches!(record.state, State::Creating | State::Active | State::Closing),
            "managed creation is no longer admissible"
        );
        if let Some(existing) = &record.vm {
            ensure!(existing == &binding, "managed VM generation cannot be rebound");
        }
        if matches!(record.state, State::Creating | State::Active) {
            let mut deadlines = self.runtime_deadlines()?;
            self.expire_if_due(&mut record, clock, &mut deadlines)?;
        }
        record.vm = Some(binding);
        if record.state == State::Creating {
            record.state = State::Active;
        }
        self.write(&record)?;
        Ok(Snapshot::from(&record))
    }

    /// The service calls this only after VM teardown, session cleanup and grant
    /// revocation have all completed for this exact binding. Failures leave the
    /// durable Closing intent available for reconciliation.
    pub fn complete_cleanup(&self, ticket: &Ticket, binding: &VmBinding) -> Result<Snapshot> {
        let _lease = self.lease()?;
        let mut record = self.ticket_record(ticket)?;
        ensure!(record.state == State::Closing, "managed close is not pending");
        ensure!(
            record.vm.as_ref() == Some(binding),
            "managed cleanup VM generation mismatch"
        );
        record.state = State::Closed;
        record.vm = None;
        self.write(&record)?;
        self.runtime_deadlines()?.remove(&record.request);
        Ok(Snapshot::from(&record))
    }
}
