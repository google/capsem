//! Sibling case modules of `mitm_integration.rs`. Each reaches the shared
//! fixtures (`make_proxy_config*`, `spawn_proxy`, `spawn_fake_upstream`,
//! `read_http11_request`) through `super::*`.
use super::*;

mod deadlines;
mod host_normalization;
mod websocket_policy;
