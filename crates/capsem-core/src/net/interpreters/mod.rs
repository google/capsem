//! Provider/protocol-specific interpreters: consume L2 events from the
//! parsers module and emit L3 semantic events (model calls / tool calls
//! summaries, tool-use deltas, etc).
//!
//! Each interpreter lives in its own file with a sibling `tests.rs`. They
//! become hooks under T1+ of the mitm-redesign sprint: each interpreter
//! subscribes to `Event::SseEvent` (or `Event::JsonRpcMessage` for MCP) on
//! its provider's domain and emits the L3 events its telemetry consumers
//! need.

pub mod anthropic_interpreter;
pub mod google_interpreter;
pub mod openai_interpreter;
