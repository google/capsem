use std::collections::HashMap;

/// A file to write into the guest filesystem at boot.
#[derive(Debug, Clone)]
pub struct GuestFile {
    pub path: String,
    pub content: String,
    pub mode: u32,
}

/// Guest VM boot configuration.
#[derive(Debug, Default, Clone)]
pub struct GuestConfig {
    pub env: Option<HashMap<String, String>>,
    pub files: Option<Vec<GuestFile>>,
}
