//! Requests that have to wait for the queue to commit.
//!
//! The writer thread batches writes; a flush barrier and a retention request
//! are the two things that cannot be batched with them, because both are
//! answers about the state of the ledger *after* everything already queued
//! has been committed. Both therefore end the drain, force the disk flush,
//! and are answered once it has run.
//!
//! Keeping the pair here rather than in the loop is what makes that shared
//! shape visible: retention arrived as a second barrier, and the loop did not
//! have to learn a second way of waiting for one.

use rusqlite::Connection;
use tracing::{info, warn};

use super::bodies::BodyArchive;
use super::retention::{retain_bodies, RetainOutcome};
use super::{FlushOutcome, RetainReply, WriteOp, WriterMessage};

/// Barrier requests collected from one drain of the writer channel.
#[derive(Default)]
pub(super) struct Barriers {
    flushes: Vec<tokio::sync::oneshot::Sender<FlushOutcome>>,
    retains: Vec<(String, RetainReply)>,
}

impl Barriers {
    /// Sort one message into `batch` or into this set. `true` when it was a
    /// barrier, which ends the drain: a barrier answers for what was queued
    /// before it, so nothing queued after it may join the same transaction.
    pub(super) fn accept(&mut self, message: WriterMessage, batch: &mut Vec<WriteOp>) -> bool {
        match message {
            WriterMessage::Write(op) => {
                batch.push(op);
                false
            }
            WriterMessage::Flush(reply) => {
                self.flushes.push(reply);
                true
            }
            WriterMessage::Retain { cutoff, reply } => {
                self.retains.push((cutoff, reply));
                true
            }
        }
    }

    /// Whether anything is waiting, and so whether the disk flush is due.
    pub(super) fn waiting(&self) -> bool {
        !self.flushes.is_empty() || !self.retains.is_empty()
    }

    /// Answer every waiting request now that the flush has run.
    ///
    /// A failed flush fails the retention too, rather than compacting an
    /// archive whose most recent blocks may not have their index rows yet:
    /// those rows are invisible to the keep query, so their blocks would be
    /// dropped and then indexed at offsets that no longer exist.
    pub(super) fn answer(&mut self, conn: &Connection, bodies: &mut BodyArchive, flush: &FlushOutcome) {
        for reply in self.flushes.drain(..) {
            let _ = reply.send(flush.clone());
        }
        for (cutoff, reply) in self.retains.drain(..) {
            let outcome = match flush {
                Err(error) => Err(format!(
                    "session body retention skipped, the ledger did not flush: {error}"
                )),
                Ok(()) => retain_bodies(conn, bodies, &cutoff),
            };
            report(&cutoff, &outcome);
            let _ = reply.send(outcome);
        }
    }
}

fn report(cutoff: &str, outcome: &Result<RetainOutcome, String>) {
    match outcome {
        Ok(retained) => info!(
            cutoff,
            blocks_dropped = retained.blocks_dropped,
            blocks_kept = retained.blocks_kept,
            rows_dropped = retained.rows_dropped,
            bytes_reclaimed = retained.bytes_reclaimed,
            "session body retention completed"
        ),
        Err(error) => warn!(cutoff, error = %error, "session body retention failed"),
    }
}
