//! Wire-format parsers fed chunk-by-chunk; emit higher-level events.
//!
//! Each parser lives in its own file with a sibling `tests.rs`. New parsers
//! join this module without surgery to anything else: the MITM pipeline
//! (T1+) registers them as hooks that subscribe to L1 chunk events and emit
//! L2 protocol-classified events.
//!
//! `dns_parser` is the exception to "chunk-by-chunk": DNS messages are
//! datagrams that arrive whole (UDP) or length-prefixed (TCP), so the
//! parser is a one-shot decode rather than a stateful feeder. It still
//! lives here because it's a wire-format codec consumed by a higher-level
//! handler -- the same shape as the SSE / provider parsers.

pub mod dns_parser;
pub mod sse_parser;
