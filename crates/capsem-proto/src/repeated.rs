//! One run of identical structured events inside an archive block.
//!
//! The event type is supplied by the ledger owner. Keeping this envelope in
//! the shared protocol lets every structured event family use the same count
//! and time-range semantics while raw request/response bytes stay untouched.

use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// Structured event records share the ledger's 10 MiB body ceiling.
pub const MAX_ENCODED_EVENT_BYTES: usize = 10 * 1024 * 1024;

/// A single event has no redundant last timestamp or count on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRun<T> {
    pub first_timestamp_unix_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_timestamp_unix_ms: Option<i64>,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub count: u64,
    pub event: T,
}

const fn one() -> u64 {
    1
}

fn is_one(value: &u64) -> bool {
    *value == 1
}

impl<T: PartialEq> EventRun<T> {
    pub fn new(timestamp_unix_ms: i64, event: T) -> Self {
        Self {
            first_timestamp_unix_ms: timestamp_unix_ms,
            last_timestamp_unix_ms: None,
            count: 1,
            event,
        }
    }

    /// Extend only an identical, time-ordered run. Callers start a new run on
    /// `false`; they never discard a distinct event or invent a timestamp.
    pub fn absorb(&mut self, timestamp_unix_ms: i64, event: &T) -> bool {
        if *event != self.event
            || timestamp_unix_ms < self.last_timestamp_unix_ms.unwrap_or(self.first_timestamp_unix_ms)
            || self.count == u64::MAX
        {
            return false;
        }
        self.count += 1;
        self.last_timestamp_unix_ms = Some(timestamp_unix_ms);
        true
    }
}

impl<T> EventRun<T> {
    pub fn last_timestamp_unix_ms(&self) -> i64 {
        self.last_timestamp_unix_ms.unwrap_or(self.first_timestamp_unix_ms)
    }

    pub fn validate(&self) -> bool {
        self.count != 0
            && self.last_timestamp_unix_ms() >= self.first_timestamp_unix_ms
            && (self.count == 1) == self.last_timestamp_unix_ms.is_none()
    }
}

impl<T: Serialize> EventRun<T> {
    pub fn encode(&self) -> Result<Vec<u8>> {
        if !self.validate() {
            bail!("invalid repeated event count or time range");
        }
        let encoded = rmp_serde::to_vec_named(self).context("encode repeated event")?;
        if encoded.len() > MAX_ENCODED_EVENT_BYTES {
            bail!("repeated event exceeds {MAX_ENCODED_EVENT_BYTES} bytes");
        }
        Ok(encoded)
    }
}

impl<T: DeserializeOwned> EventRun<T> {
    pub fn decode(encoded: &[u8]) -> Result<Self> {
        if encoded.len() > MAX_ENCODED_EVENT_BYTES {
            bail!("repeated event exceeds {MAX_ENCODED_EVENT_BYTES} bytes");
        }
        let run: Self = rmp_serde::from_slice(encoded).context("decode repeated event")?;
        if !run.validate() {
            bail!("invalid repeated event count or time range");
        }
        Ok(run)
    }
}
