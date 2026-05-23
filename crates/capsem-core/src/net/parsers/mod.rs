//! Wire-format parsers fed chunk-by-chunk; emit higher-level events.
//!
//! Each parser lives in its own file with a sibling `tests.rs`. New parsers
//! join this module without surgery to anything else: the MITM pipeline
//! (T1+) registers them as hooks that subscribe to L1 chunk events and emit
//! L2 protocol-classified events.
//!
pub mod sse_parser;
