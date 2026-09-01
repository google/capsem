//! What machine took the measurement, and whether it was fit to.
//!
//! Every performance number in this repository so far was taken on an
//! unqualified machine. `gateway /vms/list CPU=0.160s > 0.140s` held the 0.6.0
//! release for two hours; it did not reproduce, and nothing recorded whether
//! the box was busy, thermally throttled, or running a governor that halves
//! clock speed. A measurement without those facts cannot be compared with
//! another one, so they go in the record and an unfit machine is refused.

use std::fs;
use std::path::Path;

use capsem_core::proctable::Process;
use serde::{Deserialize, Serialize};

use crate::schema::Host;

/// Load average per core above which a run is refused.
///
/// Per core, because "load 4" means idle on a 32-way box and saturated on a
/// dual. The gate's own benchmark steps already serialize against each other;
/// this catches everything else on the machine.
const MAX_LOAD_PER_CORE: f64 = 0.5;

/// Governors that do not hold a stable clock. A `powersave` machine and a
/// `performance` machine do not produce comparable timings.
const UNSTABLE_GOVERNORS: &[&str] = &["powersave", "ondemand", "conservative", "schedutil"];

/// A reason this machine should not be measured on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Objection {
    pub what: String,
    pub detail: String,
}

/// The verdict of `capsem-bench doctor`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fitness {
    pub host: Host,
    pub objections: Vec<Objection>,
}

impl Fitness {
    pub fn fit(&self) -> bool {
        self.objections.is_empty()
    }
}

/// One-minute load average, or `None` where the OS does not publish one.
fn load_average(proc_loadavg: &Path) -> Option<f64> {
    let text = fs::read_to_string(proc_loadavg).ok()?;
    text.split_whitespace().next()?.parse().ok()
}

/// The CPU frequency governor, where the kernel exposes one.
fn governor(sysfs: &Path) -> Option<String> {
    fs::read_to_string(sysfs).ok().map(|g| g.trim().to_string())
}

/// Judge a set of observed facts. Pure, so the policy is testable without a
/// machine that happens to be in the state under test.
pub fn assess(
    cpu_count: usize,
    load: Option<f64>,
    governor: Option<&str>,
    kvm: bool,
    strays: &[String],
) -> Vec<Objection> {
    let mut objections = Vec::new();

    if let Some(load) = load {
        let ceiling = MAX_LOAD_PER_CORE * cpu_count as f64;
        if load > ceiling {
            objections.push(Objection {
                what: "load".to_string(),
                detail: format!(
                    "one-minute load {load:.2} exceeds {ceiling:.2} for {cpu_count} cores; \
                     the machine is busy and its timings describe that, not Capsem"
                ),
            });
        }
    }

    if let Some(name) = governor {
        if UNSTABLE_GOVERNORS.contains(&name) {
            objections.push(Objection {
                what: "governor".to_string(),
                detail: format!(
                    "CPU governor is {name}; clock speed varies during the run, \
                     so a slower result may only mean a slower clock"
                ),
            });
        }
    }

    if !kvm {
        objections.push(Objection {
            what: "kvm".to_string(),
            detail: "no /dev/kvm; guest dimensions would measure emulation".to_string(),
        });
    }

    if !strays.is_empty() {
        objections.push(Objection {
            what: "strays".to_string(),
            detail: format!(
                "{} capsem process(es) already running: {}; they compete for the \
                 CPU being measured",
                strays.len(),
                strays.join(", ")
            ),
        });
    }

    objections
}

/// PIDs in this process's ancestry, including itself.
///
/// The doctor runs both directly and as a child of `capsem-gate`. Its own gate
/// is measurement infrastructure, while another Capsem process is contention.
/// The shared process-table snapshot distinguishes those cases without
/// exempting every process named `capsem-gate`.
fn ancestry(processes: &[Process], self_pid: u32) -> Vec<u32> {
    let mut ancestry = Vec::new();
    let mut current = self_pid;
    while !ancestry.contains(&current) {
        ancestry.push(current);
        let Some(process) = processes.iter().find(|process| process.pid == current) else {
            break;
        };
        current = process.parent_pid;
    }
    ancestry
}

/// Capsem processes outside this process's ancestry.
fn strays_from_processes(processes: &[Process], self_pid: u32) -> Vec<String> {
    let ancestry = ancestry(processes, self_pid);
    processes
        .iter()
        .filter(|process| !ancestry.contains(&process.pid))
        .filter_map(|process| {
            let executable = process.arguments.split_whitespace().next()?;
            let name = Path::new(executable).file_name()?.to_str()?;
            name.starts_with("capsem").then(|| name.to_string())
        })
        .collect()
}

/// Capsem processes already running, which compete for the CPU being measured.
pub fn running_capsem_processes() -> Vec<String> {
    capsem_core::proctable::processes()
        .map(|processes| strays_from_processes(&processes, std::process::id()))
        .unwrap_or_default()
}

/// Observe this machine and judge it.
pub fn examine(arch: &str, os: &str, strays: &[String]) -> Fitness {
    let cpu_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let load = load_average(Path::new("/proc/loadavg"));
    let governor = governor(Path::new(
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
    ));
    let kvm = Path::new("/dev/kvm").exists();

    let objections = assess(cpu_count, load, governor.as_deref(), kvm, strays);

    Fitness {
        host: Host {
            arch: arch.to_string(),
            os: os.to_string(),
            cpu_count,
            kvm,
            governor,
            load_before: load.unwrap_or(0.0),
        },
        objections,
    }
}

#[cfg(test)]
mod tests;
