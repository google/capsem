//! `capsem.bench.v1`: the one shape every benchmark records.
//!
//! Eleven archive categories carried about ten mutually incompatible shapes.
//! `version` meant the measuring tool's semver in some files and a schema tag
//! in others; some carried `arch`, some `profile`, some neither; time was
//! `timestamp` on the guest clock, `host_recorded_at` on the host clock, or
//! absent. Exactly eight metrics across two of eleven categories were
//! machine-addressable, through dotted paths hardcoded in the ratchet.
//!
//! So: one envelope, one clock, explicit identity, and a flat metric list
//! whose `key` is stable. Ratcheting, plotting and diffing then work the same
//! way for every dimension, and adding a dimension does not extend the reader.

use serde::{Deserialize, Serialize};

use crate::stats::Summary;

/// Bumped only when a reader must change. Records name it so an old file is
/// recognised rather than mis-parsed.
pub const SCHEMA: &str = "capsem.bench.v1";

/// What is being measured. One variant per collector.
///
/// The wire form is kebab-case and is used in filenames and budget keys, so
/// renaming a variant is a data migration, not a refactor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Dimension {
    // Host, driving the service and gateway.
    Routes,
    Websocket,
    Lifecycle,
    Fork,
    Scaling,
    Install,
    Resources,
    Vsock,
    Criterion,
    Protocol,
    // Guest, inside the VM.
    Disk,
    Rootfs,
    Storage,
    Startup,
    Snapshot,
    McpLoad,
    MitmLoad,
    DnsLoad,
}

impl Dimension {
    /// Every dimension, for `capsem-bench list` and for a full sweep.
    pub const ALL: &'static [Dimension] = &[
        Dimension::Routes,
        Dimension::Websocket,
        Dimension::Lifecycle,
        Dimension::Fork,
        Dimension::Scaling,
        Dimension::Install,
        Dimension::Resources,
        Dimension::Vsock,
        Dimension::Criterion,
        Dimension::Protocol,
        Dimension::Disk,
        Dimension::Rootfs,
        Dimension::Storage,
        Dimension::Startup,
        Dimension::Snapshot,
        Dimension::McpLoad,
        Dimension::MitmLoad,
        Dimension::DnsLoad,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Dimension::Routes => "routes",
            Dimension::Websocket => "websocket",
            Dimension::Lifecycle => "lifecycle",
            Dimension::Fork => "fork",
            Dimension::Scaling => "scaling",
            Dimension::Install => "install",
            Dimension::Resources => "resources",
            Dimension::Vsock => "vsock",
            Dimension::Criterion => "criterion",
            Dimension::Protocol => "protocol",
            Dimension::Disk => "disk",
            Dimension::Rootfs => "rootfs",
            Dimension::Storage => "storage",
            Dimension::Startup => "startup",
            Dimension::Snapshot => "snapshot",
            Dimension::McpLoad => "mcp-load",
            Dimension::MitmLoad => "mitm-load",
            Dimension::DnsLoad => "dns-load",
        }
    }

    /// Needs a booted VM, so `quick` skips it.
    ///
    /// This is what makes a dev-loop run finish in under a minute: everything
    /// that provisions a guest is the expensive half.
    pub fn needs_vm(self) -> bool {
        !matches!(
            self,
            Dimension::Criterion | Dimension::Protocol | Dimension::Routes | Dimension::Websocket
        )
    }
}

/// The unit a metric is expressed in. Compared before two numbers are, so a
/// millisecond is never ratcheted against a second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Seconds,
    Milliseconds,
    Nanoseconds,
    Bytes,
    Megabytes,
    RequestsPerSecond,
    MegabitsPerSecond,
    Operations,
    Ratio,
    Count,
}

/// One measured thing: a stable key, a unit, and its distribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    /// Dotted and stable across releases -- `gateway./vms/list.cpu_s`. This is
    /// the identity a trend line and a budget both hang on, so it must survive
    /// renaming the function that produced it.
    pub key: String,
    pub unit: Unit,
    #[serde(flatten)]
    pub summary: Summary,
}

/// Which release the numbers describe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Release {
    /// Strict semver from the workspace manifest. The clock-derived scheme
    /// this repo used to carry -- `1.5.1783712334` -- ordered releases but
    /// described none of them, and was retired.
    pub version: String,
    pub channel: String,
    pub commit: String,
}

/// The machine, because a number without one is not comparable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Host {
    pub arch: String,
    pub os: String,
    pub cpu_count: usize,
    pub kvm: bool,
    /// CPU frequency governor where the OS exposes one. A `powersave` machine
    /// and a `performance` machine do not produce comparable timings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub governor: Option<String>,
    /// One-minute load average when the run started.
    pub load_before: f64,
}

/// One dimension's measurements, as written to disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub schema: String,
    pub dimension: Dimension,
    /// Host clock, RFC 3339, UTC. One clock, so guest and host records order.
    pub recorded_at: String,
    pub release: Release,
    pub host: Host,
    pub profile: String,
    /// Reduced-sample dev-loop run. Never promotable to evidence.
    pub quick: bool,
    pub metrics: Vec<Metric>,
    /// Bulk output kept beside the record rather than inside it. The parallel
    /// benchmark embedded 80 KB of captured stdout in its artifact.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidecar: Option<String>,
}

impl Record {
    /// The filename this record is stored under.
    ///
    /// Identity is in the name because a lane that omits it overwrites its
    /// sibling: `route-latency` carried neither arch nor profile, so the
    /// `code` and `co-work` runs of one gate wrote the same path.
    pub fn filename(&self) -> String {
        format!(
            "{}_{}_{}_{}.json",
            self.dimension.as_str(),
            self.release.version,
            self.host.arch,
            self.profile
        )
    }

    pub fn metric(&self, key: &str) -> Option<&Metric> {
        self.metrics.iter().find(|metric| metric.key == key)
    }
}

#[cfg(test)]
mod tests;
