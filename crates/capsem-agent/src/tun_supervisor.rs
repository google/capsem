//! Keep one `capsem-tun` running for every network cable the host plugged.
//!
//! The host says which cables this guest has (`PlugCable`, `UnplugCable` on
//! the control channel): an id, and the address and prefix of the network
//! the cable belongs to. Each cable's pump is a separate binary with no
//! authority beyond its own tap and one VSOCK stream, so it is a child here
//! rather than a thread. When it exits -- the host end closed, the VM
//! resumed, a bug -- it is started again after a bounded pause, for as long
//! as the cable stays plugged: a cable without its pump carries nothing, and
//! nothing else would notice. Unplugging kills the pump for good, and its tap
//! goes with it. The table outlives any one control connection.
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const TUN_BINARY: &str = "/usr/local/bin/capsem-tun";
const FIRST_RESTART_DELAY: Duration = Duration::from_secs(1);
const MAX_RESTART_DELAY: Duration = Duration::from_secs(30);
/// How often a supervisor looks at its pump and at its stop flag.
const POLL: Duration = Duration::from_millis(50);

/// What the host said about one cable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CableSpec {
    pub cable: u32,
    pub address: Ipv4Addr,
    pub prefix: u8,
}

impl CableSpec {
    pub fn arguments(&self) -> Vec<String> {
        vec![
            "--cable".into(),
            self.cable.to_string(),
            "--address".into(),
            self.address.to_string(),
            "--prefix".into(),
            self.prefix.to_string(),
        ]
    }
}

struct Supervised {
    spec: CableSpec,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

pub struct Cables {
    binary: PathBuf,
    running: Mutex<HashMap<u32, Supervised>>,
}

/// This guest's cables.
pub fn cables() -> &'static Cables {
    static CABLES: OnceLock<Cables> = OnceLock::new();
    CABLES.get_or_init(|| Cables::new(TUN_BINARY.into()))
}

impl Cables {
    pub fn new(binary: PathBuf) -> Self {
        Self {
            binary,
            running: Mutex::new(HashMap::new()),
        }
    }

    /// Keep a pump running for `spec`: a cable already running as specified
    /// is left alone, one plugged with a new address is restarted.
    pub fn plug(&self, spec: CableSpec) {
        let mut running = self.running.lock().unwrap();
        if running
            .get(&spec.cable)
            .is_some_and(|supervised| supervised.spec == spec)
        {
            return;
        }
        if let Some(replaced) = running.remove(&spec.cable) {
            stop(replaced);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let binary = self.binary.clone();
        let supervised_spec = spec.clone();
        let stopped = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name(format!("capsem-tun-cable{}", spec.cable))
            .spawn(move || supervise(&binary, &supervised_spec, &stopped))
            .expect("spawn capsem-tun supervisor");
        eprintln!(
            "[capsem-agent] cable {} plugged as {}/{}",
            spec.cable, spec.address, spec.prefix
        );
        running.insert(spec.cable, Supervised { spec, stop, thread });
    }

    /// Stop the cable's pump for good; its tap goes with it. Returns once the
    /// pump is dead and reaped.
    pub fn unplug(&self, cable: u32) {
        let removed = self.running.lock().unwrap().remove(&cable);
        if let Some(supervised) = removed {
            stop(supervised);
            eprintln!("[capsem-agent] cable {cable} unplugged");
        }
    }
}

fn stop(supervised: Supervised) {
    supervised.stop.store(true, Ordering::SeqCst);
    if supervised.thread.join().is_err() {
        eprintln!(
            "[capsem-agent] capsem-tun supervisor for cable {} panicked",
            supervised.spec.cable
        );
    }
}

/// Run the pump and run it again whenever it exits, until told to stop; then
/// kill and reap whatever is running.
fn supervise(binary: &std::path::Path, spec: &CableSpec, stop: &AtomicBool) {
    let mut delay = FIRST_RESTART_DELAY;
    while !stop.load(Ordering::SeqCst) {
        let started = Instant::now();
        match Command::new(binary).args(spec.arguments()).spawn() {
            Ok(child) => match watch(child, stop) {
                Some(status) => eprintln!("[capsem-agent] capsem-tun for cable {} exited: {status}", spec.cable),
                None => return,
            },
            Err(error) => eprintln!(
                "[capsem-agent] capsem-tun for cable {} could not start: {error}",
                spec.cable
            ),
        }
        // A pump that lived a while earned a fresh backoff; one that dies at
        // once backs off up to the cap so a broken binary does not spin the
        // guest.
        delay = if started.elapsed() > MAX_RESTART_DELAY {
            FIRST_RESTART_DELAY
        } else {
            (delay * 2).min(MAX_RESTART_DELAY)
        };
        let resume = Instant::now() + delay;
        while Instant::now() < resume {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(POLL);
        }
    }
}

/// Wait for the pump to exit, or kill it once told to stop (`None`).
fn watch(mut child: Child, stop: &AtomicBool) -> Option<std::process::ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if stop.load(Ordering::SeqCst) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(error) => {
                eprintln!("[capsem-agent] capsem-tun could not be watched: {error}");
                let _ = child.kill();
                return child.wait().ok();
            }
        }
    }
}

#[cfg(test)]
mod tests;
