//! Runtime boundary for Capsem UI/workspace state.
//!
//! This crate is the mainline-facing seam for records, projection, checkpoints,
//! topology, and editable Loro-backed state. The implementation is re-exported
//! from `capsem-ui-catalog` during the prototype so existing routes keep
//! working while the artifact/type ownership split is made explicit.

pub mod editable_state {
    pub use capsem_ui_catalog::editable_state::*;
}

pub mod storage;

pub mod workspace {
    pub use capsem_ui_catalog::workspace::*;
}
