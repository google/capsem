//! Structured terminal continuation; storage/lifecycle remain separate owners.

use super::{callbacks, options, osc_budget::OscBudget, Inner, Proxy, SessionTerminal};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::checkpoint::{CheckpointLimits, TermCheckpoint};
use alacritty_terminal::vte::ansi::{checkpoint::ProcessorCheckpoint, Processor};
use alacritty_terminal::Term;
use capsem_proto::terminal::{TerminalNotification, TerminalSummary};
use capsem_proto::{
    TERMINAL_CHECKPOINT_DEPTH, TERMINAL_CHECKPOINT_KIB, TERMINAL_CHECKPOINT_PENDING_KIB, TERMINAL_COMBINING_CHARS,
    TERMINAL_CONTROL_STRING_BYTES, TERMINAL_HISTORY_CELLS, TERMINAL_MAX_CELLS, TERMINAL_MAX_COLS, TERMINAL_MAX_ROWS,
    TERMINAL_METADATA_BYTES,
};
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Write};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, watch};

pub const MAX_CHECKPOINT_BYTES: usize = TERMINAL_CHECKPOINT_KIB * 1024;
const PENDING_BYTES: usize = TERMINAL_CHECKPOINT_PENDING_KIB * 1024;
const VERSION: u16 = 1;

#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("terminal checkpoint exceeds byte limit")]
    TooLarge,
    #[error("invalid terminal checkpoint: {0}")]
    Invalid(&'static str),
    #[error("decode terminal checkpoint: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error("encode terminal checkpoint: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u16,
    cols: u16,
    rows: u16,
    revision: u64,
    model: TermCheckpoint,
    parser: ProcessorCheckpoint,
    osc_budget: OscBudget,
    metadata: Metadata,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    title: String,
    icon_name: String,
    bell_count: u64,
    last_notification: Option<TerminalNotification>,
}

impl Metadata {
    fn valid(&self) -> bool {
        let text = |value: &str| value.len() <= TERMINAL_METADATA_BYTES && !value.chars().any(char::is_control);
        text(&self.title)
            && text(&self.icon_name)
            && self.last_notification.as_ref().is_none_or(|notification| {
                !notification.id.is_empty()
                    && text(&notification.id)
                    && text(&notification.title)
                    && text(&notification.body)
            })
    }
}

fn limits() -> CheckpointLimits {
    CheckpointLimits {
        max_columns: TERMINAL_MAX_COLS,
        max_lines: TERMINAL_MAX_ROWS,
        max_history: TERMINAL_HISTORY_CELLS / alacritty_terminal::term::MIN_COLUMNS,
        max_cells: 2 * TERMINAL_MAX_CELLS + TERMINAL_HISTORY_CELLS,
        max_text_bytes: TERMINAL_CONTROL_STRING_BYTES,
        max_combining_chars: TERMINAL_COMBINING_CHARS,
    }
}

impl SessionTerminal {
    /// Capture coherent model/parser state, then encode after releasing the
    /// publication lock. Call on a blocking worker during lifecycle quiescence.
    /// No raw input is replayed and no view/control authority is persisted.
    pub fn checkpoint(&self) -> Result<Vec<u8>, CheckpointError> {
        let inner = self.inner.lock().unwrap();
        let effects = inner.effects.0.lock().unwrap();
        if !effects.events.is_empty() || !effects.replies.is_empty() {
            return Err(CheckpointError::Invalid("unpublished terminal effects"));
        }
        let saved = Saved {
            version: VERSION,
            cols: inner.term.columns() as u16,
            rows: inner.term.screen_lines() as u16,
            revision: inner.revision,
            model: inner.term.checkpoint(&limits()).map_err(CheckpointError::Invalid)?,
            parser: inner
                .parser
                .checkpoint(PENDING_BYTES)
                .map_err(CheckpointError::Invalid)?,
            osc_budget: inner.osc_budget.clone(),
            metadata: Metadata {
                title: effects.title.clone(),
                icon_name: effects.icon_name.clone(),
                bell_count: effects.bell_count,
                last_notification: effects.last_notification.clone(),
            },
        };
        drop(effects);
        drop(inner);
        let mut output = LimitedWriter::new(MAX_CHECKPOINT_BYTES);
        saved.serialize(&mut rmp_serde::Serializer::new(&mut output).with_struct_map())?;
        Ok(output.bytes)
    }

    /// Validate and restore before exposing a new owner to IPC or guest bytes.
    /// Restore emits neither terminal input nor bell/notification callbacks.
    pub fn from_checkpoint(bytes: &[u8], capacity: usize) -> Result<Arc<Self>, CheckpointError> {
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return Err(CheckpointError::TooLarge);
        }
        let mut decoder = rmp_serde::Deserializer::new(Cursor::new(bytes));
        decoder.set_max_depth(TERMINAL_CHECKPOINT_DEPTH);
        let saved = Saved::deserialize(&mut decoder)?;
        if decoder.position() != bytes.len() as u64 {
            return Err(CheckpointError::Invalid("trailing bytes"));
        }
        if saved.version != VERSION || saved.revision == u64::MAX || capacity == 0 {
            return Err(CheckpointError::Invalid("version, revision or subscription capacity"));
        }
        super::size(saved.cols, saved.rows).map_err(CheckpointError::Invalid)?;
        if !saved.osc_budget.valid_checkpoint() || !saved.metadata.valid() {
            return Err(CheckpointError::Invalid("OSC continuation or metadata"));
        }
        let effects = Proxy::default();
        let model = Term::from_checkpoint(
            saved.model,
            options(usize::from(saved.cols)),
            effects.clone(),
            &limits(),
        )
        .map_err(CheckpointError::Invalid)?;
        if (model.columns(), model.screen_lines()) != (usize::from(saved.cols), usize::from(saved.rows)) {
            return Err(CheckpointError::Invalid("model dimensions disagree with header"));
        }
        let parser = Processor::from_checkpoint(saved.parser, PENDING_BYTES).map_err(CheckpointError::Invalid)?;
        let summary = TerminalSummary {
            revision: saved.revision,
            title: saved.metadata.title.clone(),
            bell_count: saved.metadata.bell_count,
            last_notification: saved.metadata.last_notification.clone(),
        };
        *effects.0.lock().unwrap() = callbacks::Callbacks {
            title: saved.metadata.title,
            icon_name: saved.metadata.icon_name,
            bell_count: saved.metadata.bell_count,
            last_notification: saved.metadata.last_notification,
            cols: saved.cols,
            rows: saved.rows,
            ..Default::default()
        };
        Ok(Arc::new(Self {
            inner: Mutex::new(Inner {
                term: model,
                parser,
                effects,
                osc_budget: saved.osc_budget,
                revision: saved.revision,
                next_view: 0,
                controller: None,
                accepting_views: true,
                updates: broadcast::channel(capacity).0,
                summary: watch::channel(summary).0,
            }),
        }))
    }
}

struct LimitedWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl LimitedWriter {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }
}

impl Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let needed = self.bytes.len().saturating_add(bytes.len());
        if needed > self.limit {
            return Err(std::io::Error::from(std::io::ErrorKind::FileTooLarge));
        }
        if needed > self.bytes.capacity() {
            let capacity = self.bytes.capacity().saturating_mul(2).min(self.limit).max(needed);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(std::io::Error::other)?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
