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
        let vm_writer_tx: Arc<Mutex<Option<std::sync::mpsc::Sender<WriterMsg>>>> =
            Arc::new(Mutex::new(None));

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

    format!(
        "{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z"
    )
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
mod tests {
    use super::*;

    #[test]
    fn visitor_message_only() {
        let v = MessageVisitor {
            message: "boot failed".into(),
            fields: vec![],
        };
        assert_eq!(v.into_message(), "boot failed");
    }

    #[test]
    fn visitor_message_with_fields() {
        let v = MessageVisitor {
            message: "failed to initialize MCP server".into(),
            fields: vec![
                ("server".into(), "Deps dev".into()),
                ("error".into(), "connection refused".into()),
            ],
        };
        assert_eq!(
            v.into_message(),
            "failed to initialize MCP server (server=Deps dev, error=connection refused)"
        );
    }

    #[test]
    fn visitor_fields_only_no_message() {
        let v = MessageVisitor {
            message: String::new(),
            fields: vec![("key".into(), "val".into())],
        };
        assert_eq!(v.into_message(), "key=val");
    }

    #[test]
    fn format_timestamp_epoch() {
        let ts = format_timestamp(0, 0);
        assert_eq!(ts, "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn format_timestamp_with_millis() {
        // 2026-03-17T10:05:32.123Z
        // seconds since epoch for 2026-03-17T10:05:32 UTC
        let secs = 1773741932;
        let ts = format_timestamp(secs, 123);
        assert_eq!(ts, "2026-03-17T10:05:32.123Z");
    }

    #[test]
    fn log_event_serialization_roundtrip() {
        let event = LogEvent {
            timestamp: "2026-03-17T10:05:32.000Z".to_string(),
            level: "INFO".to_string(),
            target: "capsem::vm::boot".to_string(),
            message: "kernel loaded".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: LogEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.timestamp, event.timestamp);
        assert_eq!(parsed.level, event.level);
        assert_eq!(parsed.target, event.target);
        assert_eq!(parsed.message, event.message);
    }

    #[test]
    fn log_handle_set_emitter_drains_buffer() {
        let (layer, handle) = TauriLogLayer::new();

        // Simulate buffered events
        {
            let mut guard = layer.early_buffer.lock().unwrap();
            if let Some(ref mut buf) = *guard {
                buf.push(LogEvent {
                    timestamp: "t1".into(),
                    level: "INFO".into(),
                    target: "test".into(),
                    message: "buffered".into(),
                });
            }
        }

        let received = Arc::new(Mutex::new(Vec::new()));
        let r = Arc::clone(&received);
        handle.set_emitter(move |event| {
            r.lock().unwrap().push(event);
        });

        let events = received.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].message, "buffered");
    }

    #[test]
    fn log_handle_emitter_set_only_once() {
        let (_layer, handle) = TauriLogLayer::new();
        let called = Arc::new(Mutex::new(false));
        let c = Arc::clone(&called);
        handle.set_emitter(move |_| { *c.lock().unwrap() = true; });

        // Second set should be silently ignored (OnceLock)
        handle.set_emitter(|_| {});

        // First emitter should still be active
        if let Some(emitter) = handle.emitter.get() {
            emitter(LogEvent {
                timestamp: "t".into(),
                level: "INFO".into(),
                target: "t".into(),
                message: "test".into(),
            });
        }
        assert!(*called.lock().unwrap());
    }

    #[test]
    fn vm_writer_writes_jsonl() {
        let (_layer, handle) = TauriLogLayer::new();

        let dir = std::env::temp_dir().join("capsem-test-log-layer");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.log");
        let file = std::fs::File::create(&path).unwrap();

        handle.set_vm_writer(file);

        // Send events through the channel
        {
            let guard = handle.vm_writer_tx.lock().unwrap();
            if let Some(ref tx) = *guard {
                tx.send(WriterMsg::Event(LogEvent {
                    timestamp: "2026-03-17T10:00:00.000Z".into(),
                    level: "INFO".into(),
                    target: "capsem::vm::boot".into(),
                    message: "kernel loaded".into(),
                }))
                .unwrap();
                tx.send(WriterMsg::Event(LogEvent {
                    timestamp: "2026-03-17T10:00:01.000Z".into(),
                    level: "WARN".into(),
                    target: "capsem::mcp".into(),
                    message: "timeout".into(),
                }))
                .unwrap();
            }
        }

        handle.clear_vm_writer();

        // Give writer thread time to finish
        std::thread::sleep(std::time::Duration::from_millis(50));

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        let e1: LogEvent = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(e1.message, "kernel loaded");
        let e2: LogEvent = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(e2.message, "timeout");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_vm_writer_without_set_is_noop() {
        let (_layer, handle) = TauriLogLayer::new();
        handle.clear_vm_writer(); // should not panic
    }

    #[test]
    fn early_buffer_caps_at_limit() {
        let (layer, _handle) = TauriLogLayer::new();
        {
            let mut guard = layer.early_buffer.lock().unwrap();
            if let Some(ref mut buf) = *guard {
                for i in 0..EARLY_BUFFER_CAP + 50 {
                    // Simulate what the layer would do
                    if buf.len() < EARLY_BUFFER_CAP {
                        buf.push(LogEvent {
                            timestamp: format!("t{i}"),
                            level: "INFO".into(),
                            target: "test".into(),
                            message: format!("msg {i}"),
                        });
                    }
                }
                assert_eq!(buf.len(), EARLY_BUFFER_CAP);
            }
        }
    }

    #[test]
    fn level_filtering_info_and_above() {
        // Verify that the level comparison is correct
        assert!(Level::DEBUG > Level::INFO); // DEBUG is "less important" = higher numeric
        assert!(Level::TRACE > Level::INFO);
        assert!(!(Level::WARN > Level::INFO)); // WARN passes the filter
        assert!(!(Level::ERROR > Level::INFO)); // ERROR passes the filter
        assert!(!(Level::INFO > Level::INFO)); // INFO passes the filter
    }
}
