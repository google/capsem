//! One batch of write ops into the memory schema, and what it counted.
//!
//! A batch is one transaction. The counters advance only after it commits, so
//! an op that rolled back with its batch, or was refused on its own retry,
//! never reaches them.

use super::*;
use crate::counters::{self, LedgerTally, StoredEffect};
use capsem_telemetry::db::{DB_WRITE_OPS_TOTAL, DB_WRITE_OP_REJECTED_TOTAL};

/// Storage work completed by a batch or by its per-operation salvage pass.
pub(super) struct BatchWriteOutcome {
    pub(super) tables: BTreeSet<&'static str>,
    pub(super) written: usize,
}

/// Re-run a failed batch one op at a time so a single rejected row cannot
/// discard the valid telemetry batched alongside it.
///
/// The batch is one transaction for throughput, which means a schema CHECK
/// violation on one op rolls back every op beside it. On a security ledger that
/// turns one malformed row from one producer into a silent hole covering an
/// arbitrary window of unrelated events, so the batch failure path pays for a
/// second pass. Nothing here runs when the batch commits.
pub(super) fn retry_batch_ops_individually(
    conn: &Connection,
    batch: &[WriteOp],
    bodies: &mut BodyArchive,
    exec_floor: i64,
    tally: &mut LedgerTally,
) -> BatchWriteOutcome {
    let expected_writes = batch.iter().filter(|op| write_op_affects_storage(op)).count();
    let mut salvaged = BatchWriteOutcome {
        tables: BTreeSet::new(),
        written: 0,
    };
    for op in batch {
        if !write_op_affects_storage(op) {
            continue;
        }
        let op_kind = op.kind();
        match execute_memory_batch(conn, std::slice::from_ref(op), bodies, exec_floor, tally) {
            Ok(outcome) => {
                salvaged.tables.extend(outcome.tables);
                salvaged.written += outcome.written;
            }
            Err(error) => {
                // Loud on purpose: a rejected op is a producer bug, and a
                // ledger that quietly loses rows is worse than one that
                // complains about them.
                error!(
                    error = %error,
                    op_kind,
                    event_id = op.event_id(),
                    "db rejected a write op; dropping it alone"
                );
                ::metrics::counter!(DB_WRITE_OP_REJECTED_TOTAL, "op_kind" => op_kind).increment(1);
            }
        }
    }
    if salvaged.written < expected_writes {
        warn!(
            rejected = expected_writes - salvaged.written,
            salvaged = salvaged.written,
            "db batch retry completed with rejected ops"
        );
    }
    salvaged
}

pub(super) fn execute_memory_batch(
    conn: &Connection,
    batch: &[WriteOp],
    bodies: &mut BodyArchive,
    exec_floor: i64,
    tally: &mut LedgerTally,
) -> rusqlite::Result<BatchWriteOutcome> {
    let stored_ops = batch.iter().filter(|op| write_op_affects_storage(op)).count();
    if stored_ops == 0 {
        return Ok(BatchWriteOutcome {
            tables: BTreeSet::new(),
            written: 0,
        });
    }

    let tx = conn.unchecked_transaction()?;
    // Bodies staged by a transaction that rolls back must not leave index
    // rows behind: their event's row is gone, and the retry pass stages them
    // again. Their bytes stay in the open block, unreferenced.
    let staged_mark = bodies.staged_mark();
    let mut affected_tables = BTreeSet::new();
    let mut op_counts = std::collections::BTreeMap::<&'static str, usize>::new();
    let mut effects = Vec::with_capacity(stored_ops);
    let outcome = insert_batch_ops(
        &tx,
        batch,
        bodies,
        exec_floor,
        &mut affected_tables,
        &mut op_counts,
        &mut effects,
    )
    .and_then(|()| counted_on_disk_now(&tx, tally, &effects))
    .and_then(|advanced| tx.commit().map(|()| advanced));
    let advanced = match outcome {
        Ok(advanced) => advanced,
        Err(error) => {
            bodies.rollback_staged(staged_mark);
            return Err(error);
        }
    };
    match advanced {
        Some(advanced) => *tally = advanced,
        None => {
            for (op, effect) in &effects {
                tally.record(op, effect);
            }
        }
    }
    for (kind, count) in op_counts {
        ::metrics::counter!(DB_WRITE_OPS_TOTAL, "insert_type" => kind).increment(count as u64);
    }
    Ok(BatchWriteOutcome {
        tables: affected_tables,
        written: stored_ops,
    })
}

/// The counters this batch leaves behind, stored in its own transaction,
/// when one of its ops changed a row that was already on disk.
///
/// Almost every op writes the memory schema, which reaches the disk at the
/// next flush together with the snapshot. An exec completion whose start row
/// was already flushed is the exception: it updates that row on disk as this
/// batch commits. The snapshot counting it must commit with it, or a crash
/// before the next flush leaves a completed row the snapshot does not count.
fn counted_on_disk_now(
    tx: &rusqlite::Transaction<'_>,
    tally: &LedgerTally,
    effects: &[(&WriteOp, StoredEffect)],
) -> rusqlite::Result<Option<LedgerTally>> {
    if !effects.iter().any(|(_, effect)| effect.updated_disk_row) {
        return Ok(None);
    }
    let mut advanced = tally.clone();
    for (op, effect) in effects {
        advanced.record(op, effect);
    }
    counters::store(tx, advanced.counters())?;
    Ok(Some(advanced))
}

fn insert_batch_ops<'a>(
    tx: &rusqlite::Transaction<'_>,
    batch: &'a [WriteOp],
    bodies: &mut BodyArchive,
    exec_floor: i64,
    affected_tables: &mut BTreeSet<&'static str>,
    op_counts: &mut std::collections::BTreeMap<&'static str, usize>,
    effects: &mut Vec<(&'a WriteOp, StoredEffect)>,
) -> rusqlite::Result<()> {
    for op in batch {
        if !write_op_affects_storage(op) {
            continue;
        }
        *op_counts.entry(op.kind()).or_default() += 1;
        affected_memory_tables(op, affected_tables);
        bodies.close_if_due();
        let mut effect = StoredEffect::default();
        match op {
            WriteOp::TransportEvent(e) => event_rows::insert_transport_event(tx, e, WriteTarget::Memory)?,
            WriteOp::NetEvent(e) => insert_net_event(tx, e, WriteTarget::Memory, bodies)?,
            WriteOp::ModelCall(m) => insert_model_call(tx, m, WriteTarget::Memory, bodies)?,
            WriteOp::McpCall(c) => insert_mcp_call(tx, c, WriteTarget::Memory, bodies)?,
            WriteOp::FileEvent(f) => insert_file_event(tx, f, WriteTarget::Memory)?,
            WriteOp::ExecEvent(e) => insert_exec_event(tx, e, WriteTarget::Memory)?,
            WriteOp::ExecEventComplete(c) => {
                let completion = update_exec_event(tx, c, exec_floor, bodies)?;
                effect.exec_completed = completion.first;
                effect.updated_disk_row = completion.on_disk;
            }
            WriteOp::AuditEvent(a) => insert_audit_event(tx, a, WriteTarget::Memory)?,
            WriteOp::DnsEvent(d) => insert_dns_event(tx, d, WriteTarget::Memory)?,
            WriteOp::SubstitutionEvent(s) => insert_substitution_event(tx, s, WriteTarget::Memory)?,
            WriteOp::SecurityRuleEvent(e) => {
                effect.forensic = insert_security_rule_event(tx, e, WriteTarget::Memory, bodies)?;
            }
            WriteOp::SecurityAskEvent(e) => insert_security_ask_event(tx, e, WriteTarget::Memory, bodies)?,
            WriteOp::SecurityDecisionEvent(e) => insert_security_decision_event(tx, e, WriteTarget::Memory, bodies)?,
            WriteOp::ProfileMutationEvent(e) => insert_profile_mutation_event(tx, e, WriteTarget::Memory)?,
            WriteOp::Network(n) => event_rows::upsert_network(tx, n, WriteTarget::Memory)?,
            WriteOp::NetworkMembership(m) => event_rows::upsert_network_membership(tx, m, WriteTarget::Memory)?,
        }
        effects.push((op, effect));
    }
    Ok(())
}
