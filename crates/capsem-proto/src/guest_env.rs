//! One private file is one named MessagePack record, consumed before confinement.

use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "guest-env.msgpack";
pub const MAX_BYTES: usize = 1024 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GuestEnvironment {
    pub values: Vec<(String, String)>,
}
