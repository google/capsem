// capsem-fs-watch: In-VM inotify file watcher daemon.
//
// Recursively watches /root for file create/modify/delete events and streams
// them to the host over vsock port 5005 as GuestToHost messages.
//
// This binary runs inside the guest VM, launched by capsem-init.

#[path = "vsock_io.rs"]
mod vsock_io;

use std::collections::HashMap;
use std::time::Instant;

#[cfg(target_os = "linux")]
use std::os::unix::io::RawFd;
#[cfg(target_os = "linux")]
use capsem_proto::{GuestToHost, encode_guest_msg};
#[cfg(target_os = "linux")]
use vsock_io::{VSOCK_HOST_CID, vsock_connect_retry, write_all_fd};

/// vsock port for filesystem events on the host.
#[cfg(target_os = "linux")]
const VSOCK_PORT_FS_WATCH: u32 = 5005;

/// Directories to exclude from watching (not configurable in V1).
const EXCLUDED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "__pycache__",
    ".cache",
    "target",
    ".venv",
    ".npm-global",
    ".swapfile",
];

/// Debounce window in milliseconds per file path.
const DEBOUNCE_MS: u64 = 100;

/// Root path to watch.
const WATCH_ROOT: &str = "/root";

/// Prefix to strip from absolute paths before sending to host.
const ROOT_PREFIX: &str = "/root/";

// ── Pure helpers (testable on macOS) ─────────────────────────────────

/// Check if a path component matches an excluded directory name.
fn should_exclude_path(path: &str) -> bool {
    for component in path.split('/') {
        if EXCLUDED_DIRS.contains(&component) {
            return true;
        }
    }
    false
}

/// Strip the /root/ prefix from an absolute path.
/// Returns the path relative to /root, or empty string for /root itself.
fn strip_root_prefix(path: &str) -> &str {
    path.strip_prefix(ROOT_PREFIX).unwrap_or(
        if path == WATCH_ROOT { "" } else { path }
    )
}

/// Actions that can be debounced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DebouncedAction {
    Created,
    Modified,
    Deleted,
}

/// Per-path debounce entry.
struct DebouncedEntry {
    action: DebouncedAction,
    size: Option<u64>,
    deadline: Instant,
}

/// Coalesces events for the same path within DEBOUNCE_MS window.
/// Latest action wins within the window.
struct Debouncer {
    pending: HashMap<String, DebouncedEntry>,
}

impl Debouncer {
    fn new() -> Self {
        Self {
            pending: HashMap::new(),
        }
    }

    /// Record an event. If the path already has a pending event within the
    /// debounce window, update it (latest action wins). Otherwise, create new.
    fn record(&mut self, path: String, action: DebouncedAction, size: Option<u64>) {
        let deadline = Instant::now() + std::time::Duration::from_millis(DEBOUNCE_MS);
        self.pending.insert(path, DebouncedEntry {
            action,
            size,
            deadline,
        });
    }

    /// Flush all entries whose deadline has passed.
    /// Returns the flushed entries as (path, action, size) tuples.
    fn flush_expired(&mut self) -> Vec<(String, DebouncedAction, Option<u64>)> {
        let now = Instant::now();
        let mut flushed = Vec::new();
        self.pending.retain(|path, entry| {
            if entry.deadline <= now {
                flushed.push((path.clone(), entry.action, entry.size));
                false
            } else {
                true
            }
        });
        flushed
    }

    /// Time until the next pending entry expires, or None if empty.
    fn next_deadline_ms(&self) -> Option<u64> {
        let now = Instant::now();
        self.pending.values()
            .map(|e| {
                if e.deadline > now {
                    e.deadline.duration_since(now).as_millis() as u64
                } else {
                    0
                }
            })
            .min()
    }
}

#[cfg(target_os = "linux")]
fn send_event(fd: RawFd, msg: &GuestToHost) {
    match encode_guest_msg(msg) {
        Ok(frame) => {
            if let Err(e) = write_all_fd(fd, &frame) {
                eprintln!("[capsem-fs-watch] write failed: {e}");
            }
        }
        Err(e) => {
            eprintln!("[capsem-fs-watch] encode failed: {e}");
        }
    }
}

// ── inotify watcher (Linux only) ────────────────────────────────────

#[cfg(target_os = "linux")]
fn run_watcher() {
    use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify, WatchDescriptor};
    use std::os::fd::AsFd;

    eprintln!("[capsem-fs-watch] starting (pid {})", std::process::id());

    let vsock_fd = vsock_connect_retry(VSOCK_HOST_CID, VSOCK_PORT_FS_WATCH, "fs-watch");

    let inotify = Inotify::init(InitFlags::IN_NONBLOCK).expect("failed to init inotify");

    let flags = AddWatchFlags::IN_CLOSE_WRITE
        | AddWatchFlags::IN_CREATE
        | AddWatchFlags::IN_DELETE
        | AddWatchFlags::IN_MOVED_FROM
        | AddWatchFlags::IN_MOVED_TO;

    let mut wd_to_path: HashMap<WatchDescriptor, String> = HashMap::new();

    // Recursively add watches starting from WATCH_ROOT.
    fn add_watches_recursive(
        inotify: &Inotify,
        root: &str,
        flags: AddWatchFlags,
        wd_to_path: &mut HashMap<WatchDescriptor, String>,
    ) {
        if should_exclude_path(root) {
            return;
        }
        match inotify.add_watch(root, flags) {
            Ok(wd) => {
                wd_to_path.insert(wd, root.to_string());
            }
            Err(e) => {
                eprintln!("[capsem-fs-watch] add_watch failed for {root}: {e}");
                return;
            }
        }
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                if let Ok(ft) = entry.file_type() {
                    if ft.is_dir() {
                        let path = entry.path().to_string_lossy().to_string();
                        add_watches_recursive(inotify, &path, flags, wd_to_path);
                    }
                }
            }
        }
    }

    add_watches_recursive(&inotify, WATCH_ROOT, flags, &mut wd_to_path);
    eprintln!(
        "[capsem-fs-watch] watching {} directories under {WATCH_ROOT}",
        wd_to_path.len()
    );

    let mut debouncer = Debouncer::new();

    loop {
        // Poll with timeout from debouncer.
        let timeout_ms = debouncer.next_deadline_ms().unwrap_or(1000) as u16;
        let mut poll_fd = [nix::poll::PollFd::new(
            inotify.as_fd(),
            nix::poll::PollFlags::POLLIN,
        )];
        let _ = nix::poll::poll(&mut poll_fd, nix::poll::PollTimeout::from(timeout_ms));

        // Read events (non-blocking).
        let events = match inotify.read_events() {
            Ok(events) => events,
            Err(nix::errno::Errno::EAGAIN) => vec![],
            Err(e) => {
                eprintln!("[capsem-fs-watch] read_events error: {e}");
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
        };

        for event in &events {
            let name = match &event.name {
                Some(n) => n.to_string_lossy().to_string(),
                None => continue,
            };

            let parent_path = match wd_to_path.get(&event.wd) {
                Some(p) => p.clone(),
                None => continue,
            };

            let full_path = format!("{parent_path}/{name}");

            if should_exclude_path(&full_path) {
                continue;
            }

            let is_dir = event.mask.contains(AddWatchFlags::IN_ISDIR);

            // New directory: add watch and scan for missed files.
            if is_dir && event.mask.contains(AddWatchFlags::IN_CREATE) {
                add_watches_recursive(&inotify, &full_path, flags, &mut wd_to_path);
                // Scan directory for files created during race window.
                if let Ok(entries) = std::fs::read_dir(&full_path) {
                    for entry in entries.flatten() {
                        if let Ok(ft) = entry.file_type() {
                            if ft.is_file() {
                                let file_path = entry.path().to_string_lossy().to_string();
                                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                                let rel = strip_root_prefix(&file_path).to_string();
                                debouncer.record(rel, DebouncedAction::Created, Some(size));
                            }
                        }
                    }
                }
                continue;
            }

            // Skip directory events (we only track files).
            if is_dir {
                continue;
            }

            let rel_path = strip_root_prefix(&full_path).to_string();

            if event.mask.contains(AddWatchFlags::IN_CLOSE_WRITE) {
                let size = std::fs::metadata(&full_path)
                    .map(|m| m.len())
                    .unwrap_or(0);
                debouncer.record(rel_path, DebouncedAction::Modified, Some(size));
            } else if event.mask.contains(AddWatchFlags::IN_CREATE) {
                let size = std::fs::metadata(&full_path)
                    .map(|m| m.len())
                    .unwrap_or(0);
                debouncer.record(rel_path, DebouncedAction::Created, Some(size));
            } else if event.mask.contains(AddWatchFlags::IN_DELETE) {
                debouncer.record(rel_path, DebouncedAction::Deleted, None);
            } else if event.mask.contains(AddWatchFlags::IN_MOVED_FROM) {
                debouncer.record(rel_path, DebouncedAction::Deleted, None);
            } else if event.mask.contains(AddWatchFlags::IN_MOVED_TO) {
                let size = std::fs::metadata(&full_path)
                    .map(|m| m.len())
                    .unwrap_or(0);
                debouncer.record(rel_path, DebouncedAction::Created, Some(size));
            }
        }

        // Flush expired debounced entries.
        for (path, action, size) in debouncer.flush_expired() {
            let msg = match action {
                DebouncedAction::Created => GuestToHost::FileCreated {
                    path: path.clone(),
                    size: size.unwrap_or(0),
                },
                DebouncedAction::Modified => GuestToHost::FileModified {
                    path: path.clone(),
                    size: size.unwrap_or(0),
                },
                DebouncedAction::Deleted => GuestToHost::FileDeleted { path: path.clone() },
            };
            send_event(vsock_fd, &msg);
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn run_watcher() {
    eprintln!("[capsem-fs-watch] inotify not available on this platform");
    std::process::exit(1);
}

fn main() {
    run_watcher();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn should_exclude_git() {
        assert!(should_exclude_path(".git"));
        assert!(should_exclude_path("/root/.git"));
        assert!(should_exclude_path("/root/project/.git/objects"));
    }

    #[test]
    fn should_exclude_node_modules() {
        assert!(should_exclude_path("node_modules"));
        assert!(should_exclude_path("/root/project/node_modules/express"));
    }

    #[test]
    fn should_exclude_nested() {
        assert!(should_exclude_path("/root/project/__pycache__/mod.pyc"));
        assert!(should_exclude_path("/root/target/debug/build"));
        assert!(should_exclude_path("/root/.venv/lib/python3"));
    }

    #[test]
    fn should_not_exclude_normal_paths() {
        assert!(!should_exclude_path("/root/project/src"));
        assert!(!should_exclude_path("/root/project/src/app.js"));
        assert!(!should_exclude_path("/root"));
        assert!(!should_exclude_path("project/src/main.rs"));
    }

    #[test]
    fn strip_root_prefix_works() {
        assert_eq!(strip_root_prefix("/root/project/app.js"), "project/app.js");
        assert_eq!(strip_root_prefix("/root/"), "");
        assert_eq!(strip_root_prefix("/root"), "");
        assert_eq!(strip_root_prefix("/root/a"), "a");
    }

    #[test]
    fn debouncer_coalesces_same_path() {
        let mut d = Debouncer::new();
        d.record("app.js".to_string(), DebouncedAction::Created, Some(100));
        d.record("app.js".to_string(), DebouncedAction::Modified, Some(200));
        // Latest action wins.
        assert_eq!(d.pending.len(), 1);
        let entry = d.pending.get("app.js").unwrap();
        assert_eq!(entry.action, DebouncedAction::Modified);
        assert_eq!(entry.size, Some(200));
    }

    #[test]
    fn debouncer_different_paths_independent() {
        let mut d = Debouncer::new();
        d.record("a.js".to_string(), DebouncedAction::Created, Some(10));
        d.record("b.js".to_string(), DebouncedAction::Deleted, None);
        assert_eq!(d.pending.len(), 2);
    }

    #[test]
    fn debouncer_flush_expired() {
        let mut d = Debouncer::new();
        // Insert with very short deadline (already expired).
        d.pending.insert("old.js".to_string(), DebouncedEntry {
            action: DebouncedAction::Created,
            size: Some(42),
            deadline: Instant::now() - Duration::from_millis(1),
        });
        // Insert with future deadline.
        d.pending.insert("new.js".to_string(), DebouncedEntry {
            action: DebouncedAction::Modified,
            size: Some(99),
            deadline: Instant::now() + Duration::from_secs(60),
        });
        let flushed = d.flush_expired();
        assert_eq!(flushed.len(), 1);
        assert_eq!(flushed[0].0, "old.js");
        assert_eq!(flushed[0].1, DebouncedAction::Created);
        assert_eq!(d.pending.len(), 1); // "new.js" still pending
    }

    #[test]
    fn rename_as_delete_create() {
        // MOVED_FROM -> Deleted, MOVED_TO -> Created
        let mut d = Debouncer::new();
        d.record("old_name.txt".to_string(), DebouncedAction::Deleted, None);
        d.record("new_name.txt".to_string(), DebouncedAction::Created, Some(100));
        assert_eq!(d.pending.len(), 2);
        let old = d.pending.get("old_name.txt").unwrap();
        assert_eq!(old.action, DebouncedAction::Deleted);
        let new = d.pending.get("new_name.txt").unwrap();
        assert_eq!(new.action, DebouncedAction::Created);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn port_constant_is_5005() {
        assert_eq!(VSOCK_PORT_FS_WATCH, 5005);
    }

    // ── Adversarial / edge-case tests ────────────────────────────────

    #[test]
    fn should_exclude_swapfile() {
        assert!(should_exclude_path("/root/.swapfile"));
        assert!(should_exclude_path(".swapfile"));
    }

    #[test]
    fn should_exclude_cache() {
        assert!(should_exclude_path("/root/.cache"));
        assert!(should_exclude_path("/root/project/.cache/huggingface"));
    }

    #[test]
    fn should_exclude_npm_global() {
        assert!(should_exclude_path("/root/.npm-global/lib/node_modules"));
    }

    #[test]
    fn should_not_exclude_partial_matches() {
        // "target" is excluded but "targets" is not.
        assert!(!should_exclude_path("/root/project/targets"));
        // "node_modules" is excluded but "node_module" is not.
        assert!(!should_exclude_path("/root/project/node_module"));
        // ".git" is excluded but ".github" is not.
        assert!(!should_exclude_path("/root/project/.github/workflows"));
        // ".cache" is excluded but ".cached" is not.
        assert!(!should_exclude_path("/root/.cached/data"));
    }

    #[test]
    fn should_not_exclude_empty_path() {
        assert!(!should_exclude_path(""));
    }

    #[test]
    fn strip_root_prefix_non_root_passthrough() {
        // Paths not under /root/ pass through unchanged.
        assert_eq!(strip_root_prefix("/tmp/file.txt"), "/tmp/file.txt");
        assert_eq!(strip_root_prefix("relative/path"), "relative/path");
    }

    #[test]
    fn strip_root_prefix_trailing_slash() {
        assert_eq!(strip_root_prefix("/root/"), "");
    }

    #[test]
    fn strip_root_prefix_deeply_nested() {
        assert_eq!(
            strip_root_prefix("/root/a/b/c/d/e/f.txt"),
            "a/b/c/d/e/f.txt"
        );
    }

    #[test]
    fn debouncer_next_deadline_empty() {
        let d = Debouncer::new();
        assert!(d.next_deadline_ms().is_none());
    }

    #[test]
    fn debouncer_next_deadline_returns_min() {
        let mut d = Debouncer::new();
        d.pending.insert("far.js".to_string(), DebouncedEntry {
            action: DebouncedAction::Created,
            size: Some(1),
            deadline: Instant::now() + Duration::from_secs(60),
        });
        d.pending.insert("soon.js".to_string(), DebouncedEntry {
            action: DebouncedAction::Modified,
            size: Some(2),
            deadline: Instant::now() + Duration::from_millis(10),
        });
        let ms = d.next_deadline_ms().unwrap();
        assert!(ms <= 60_000, "should return the shorter deadline, got {ms}ms");
    }

    #[test]
    fn debouncer_expired_deadline_returns_zero() {
        let mut d = Debouncer::new();
        d.pending.insert("old.js".to_string(), DebouncedEntry {
            action: DebouncedAction::Deleted,
            size: None,
            deadline: Instant::now() - Duration::from_millis(100),
        });
        assert_eq!(d.next_deadline_ms(), Some(0));
    }

    #[test]
    fn debouncer_flush_empty() {
        let mut d = Debouncer::new();
        let flushed = d.flush_expired();
        assert!(flushed.is_empty());
    }

    #[test]
    fn debouncer_delete_then_create_same_path() {
        // Simulates a "save" pattern: editor deletes then creates.
        let mut d = Debouncer::new();
        d.record("file.rs".to_string(), DebouncedAction::Deleted, None);
        d.record("file.rs".to_string(), DebouncedAction::Created, Some(500));
        assert_eq!(d.pending.len(), 1);
        let entry = d.pending.get("file.rs").unwrap();
        // Latest action wins: Created replaces Deleted.
        assert_eq!(entry.action, DebouncedAction::Created);
        assert_eq!(entry.size, Some(500));
    }

    #[test]
    fn debouncer_rapid_modifications_coalesce() {
        // Simulates rapid saves: only the last one survives.
        let mut d = Debouncer::new();
        for i in 0..100 {
            d.record("hot.rs".to_string(), DebouncedAction::Modified, Some(i));
        }
        assert_eq!(d.pending.len(), 1);
        let entry = d.pending.get("hot.rs").unwrap();
        assert_eq!(entry.size, Some(99));
    }
}
