//! Custom tracing layer for Tauri UI events and per-VM log files.
//!
//! Provides [`TauriLogLayer`] -- a tracing [`Layer`] that captures structured
//! log events and routes them to:
//! 1. The frontend via a deferred Tauri event emitter callback
//! 2. A per-VM JSONL log file via a background writer thread

use std::fmt::Write as _;
use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

/// A structured log event for UI display and per-VM file logging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEvent {
    pub timestamp: String,
    pub level: String,
    pub target: String,
    pub message: String,
}

/// Visitor that extracts the `message` field and appends structured fields.
///
/// Given `warn!(server = %name, error = %e, "failed to initialize")`,
/// produces: `"failed to initialize (server=Deps dev, error=connection refused)"`.
struct MessageVisitor {
    message: String,
    fields: Vec<(String, String)>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            write!(&mut self.message, "{value:?}").ok();
        } else {
            self.fields.push((field.name().to_string(), format!("{value:?}")));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.push((field.name().to_string(), value.to_string()));
        }
    }
}

impl MessageVisitor {
    fn into_message(self) -> String {
        if self.fields.is_empty() {
            return self.message;
        }
        let pairs: Vec<String> = self.fields.iter().map(|(k, v)| format!("{k}={v}")).collect();
        if self.message.is_empty() {
            pairs.join(", ")
        } else {
            format!("{} ({})", self.message, pairs.join(", "))
        }
    }
}

/// Max buffered events before the emitter is connected.
const EARLY_BUFFER_CAP: usize = 200;

type EmitterFn = Box<dyn Fn(LogEvent) + Send + Sync>;

/// Handle for injecting the Tauri emitter and per-VM file writer after init.
pub struct LogHandle {
    emitter: Arc<OnceLock<EmitterFn>>,
    early_buffer: Arc<Mutex<Option<Vec<LogEvent>>>>,
    vm_writer_tx: Arc<Mutex<Option<std::sync::mpsc::Sender<WriterMsg>>>>,
}

enum WriterMsg {
    Event(LogEvent),
    Shutdown,
}

impl LogHandle {
    /// Set the Tauri event emitter callback. Called once when `AppHandle` is available.
    /// Drains any buffered early events through the emitter.
    pub fn set_emitter<F: Fn(LogEvent) + Send + Sync + 'static>(&self, f: F) {
        let emitter_fn: EmitterFn = Box::new(f);
        if self.emitter.set(emitter_fn).is_ok() {
            // Drain early buffer
            if let Ok(mut guard) = self.early_buffer.lock() {
                if let Some(buf) = guard.take() {
                    if let Some(emitter) = self.emitter.get() {
                        for event in buf {
                            emitter(event);
                        }
                    }
                }
            }
        }
    }

    /// Start writing log events to a per-VM file. Spawns a background writer thread.
    pub fn set_vm_writer(&self, file: std::fs::File) {
        let (tx, rx) = std::sync::mpsc::channel::<WriterMsg>();
        std::thread::spawn(move || {
            let mut writer = std::io::BufWriter::new(file);
            while let Ok(msg) = rx.recv() {
                match msg {
                    WriterMsg::Event(event) => {
                        if let Ok(json) = serde_json::to_string(&event) {
                            let _ = writeln!(writer, "{json}");
                        }
                    }
                    WriterMsg::Shutdown => {
                        let _ = writer.flush();
                        break;
                    }
                }
            }
            let _ = writer.flush();
        });
        *self.vm_writer_tx.lock().unwrap() = Some(tx);
    }

    /// Stop the per-VM writer thread, flushing remaining events.
    pub fn clear_vm_writer(&self) {
        if let Some(tx) = self.vm_writer_tx.lock().unwrap().take() {
            let _ = tx.send(WriterMsg::Shutdown);
            // The thread will drain and exit
        }
    }
}

/// Custom tracing Layer that emits structured log events to the frontend
/// and optionally writes them to a per-VM log file.
pub struct TauriLogLayer {
    emitter: Arc<OnceLock<EmitterFn>>,
    early_buffer: Arc<Mutex<Option<Vec<LogEvent>>>>,
    vm_writer_tx: Arc<Mutex<Option<std::sync::mpsc::Sender<WriterMsg>>>>,
}

impl TauriLogLayer {
    /// Create a new layer and its control handle.
    pub fn new() -> (Self, LogHandle) {
        let emitter = Arc::new(OnceLock::new());
        let early_buffer = Arc::new(Mutex::new(Some(Vec::new())));
        let vm_writer_tx: Arc<Mutex<Option<std::sync::mpsc::Sender<WriterMsg>>>> = Arc::new(Mutex::new(None));

        let layer = Self {
            emitter: Arc::clone(&emitter),
            early_buffer: Arc::clone(&early_buffer),
            vm_writer_tx: Arc::clone(&vm_writer_tx),
        };
        let handle = LogHandle {
            emitter,
            early_buffer,
            vm_writer_tx,
        };
        (layer, handle)
    }

    fn make_event(&self, event: &Event<'_>) -> LogEvent {
        let meta = event.metadata();
        let mut visitor = MessageVisitor {
            message: String::new(),
            fields: Vec::new(),
        };
        event.record(&mut visitor);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs();
        let millis = now.subsec_millis();
        let timestamp = format_timestamp(secs, millis);

        LogEvent {
            timestamp,
            level: meta.level().to_string(),
            target: meta.target().to_string(),
            message: visitor.into_message(),
        }
    }
}

fn format_timestamp(secs: u64, millis: u32) -> String {
    // Convert epoch seconds to date/time components
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    // Civil date from days since epoch (algorithm from Howard Hinnant)
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z")
}

impl<S: Subscriber> Layer<S> for TauriLogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        // Only INFO and above pass through to UI/per-VM log
        let level = *event.metadata().level();
        if level > Level::INFO {
            return;
        }

        let log_event = self.make_event(event);

        // Send to VM file writer (non-blocking channel send)
        if let Ok(guard) = self.vm_writer_tx.lock() {
            if let Some(ref tx) = *guard {
                let _ = tx.send(WriterMsg::Event(log_event.clone()));
            }
        }

        // Send to UI emitter
        if let Some(emitter) = self.emitter.get() {
            emitter(log_event);
        } else {
            // Buffer early events before the emitter is connected
            if let Ok(mut guard) = self.early_buffer.lock() {
                if let Some(ref mut buf) = *guard {
                    if buf.len() < EARLY_BUFFER_CAP {
                        buf.push(log_event);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
