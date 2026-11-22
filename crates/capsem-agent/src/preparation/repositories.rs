//! Prepare pinned Git checkouts as the workload user, before its startup script.

use super::{supervise_child, Failure};
use capsem_foundation::unix::{
    contained::{ContainedDir, ContainedOpenOptions},
    process::cancellation,
};
use nix::{
    libc,
    sys::stat::{mkdirat, Mode},
};
use std::ffi::{CStr, CString, OsStr};
use std::io::{self, Read};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt, process::CommandExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub(super) struct Repository<'a> {
    url: &'a str,
    commit: &'a str,
    destination: &'a Path,
}

impl<'a> Repository<'a> {
    pub(super) fn parse(values: &'a [String]) -> Result<Self, Failure> {
        let [url, commit, destination] = values else {
            return Err(Failure::InvalidInput);
        };
        let authority = url.strip_prefix("https://").and_then(|url| url.split('/').next());
        if !authority.is_some_and(|name| !name.is_empty() && !name.contains('@'))
            || url.contains(['?', '#', '\\'])
            || url.chars().any(char::is_control)
            || !matches!(commit.len(), 40 | 64)
            || !commit
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            || destination.contains('\\')
            || destination.chars().any(char::is_control)
            || destination.split('/').any(|part| matches!(part, "" | "." | ".."))
        {
            return Err(Failure::InvalidInput);
        }
        Ok(Self {
            url,
            commit,
            destination: Path::new(destination),
        })
    }
}

pub(super) fn prepare(
    workspace: &Path,
    repositories: &[Repository<'_>],
    signals: &cancellation::Guard,
) -> Result<(), Failure> {
    let root = ContainedDir::open_root_no_follow(workspace).map_err(|_| Failure::RepositoryDestination)?;
    let deadline =
        Instant::now() + Duration::from_secs(u64::from(capsem_proto::preparation::REPOSITORY_PREPARATION_SECONDS));
    for repository in repositories {
        prepare_one(&root, repository, deadline, signals)?;
    }
    Ok(())
}

fn prepare_one(
    root: &ContainedDir,
    repository: &Repository<'_>,
    deadline: Instant,
    signals: &cancellation::Guard,
) -> Result<(), Failure> {
    let (Some(parent), Some(name)) = (repository.destination.parent(), repository.destination.file_name()) else {
        return Err(Failure::InvalidInput);
    };
    let parent = root
        .walk_creating(parent, 0o755)
        .map_err(|_| Failure::RepositoryDestination)?;
    match parent.descend(name) {
        Ok(existing) => return verify_existing(&existing, repository.url, deadline, signals),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(Failure::RepositoryDestination),
    }
    let mut entropy = [0; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut device| device.read_exact(&mut entropy))
        .map_err(|_| Failure::RepositoryDestination)?;
    let temporary = format!(".capsem-repository-{:032x}", u128::from_ne_bytes(entropy));
    let temporary = OsStr::new(&temporary);
    mkdirat(Some(parent.as_fd().as_raw_fd()), temporary, Mode::S_IRWXU).map_err(|_| Failure::RepositoryDestination)?;
    let staging = parent.descend(temporary).map_err(|_| Failure::RepositoryDestination)?;
    let result =
        checkout(&staging, repository, deadline, signals).and_then(|()| match publish(&parent, temporary, name) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let existing = parent.descend(name).map_err(|_| Failure::RepositoryDestination)?;
                verify_existing(&existing, repository.url, deadline, signals)
            }
            Err(_) => Err(Failure::RepositoryDestination),
        });
    if parent
        .descend(temporary)
        .is_ok_and(|remaining| same_directory(&staging, &remaining))
    {
        let _ = parent.remove_tree(temporary);
    }
    result
}

fn same_directory(left: &ContainedDir, right: &ContainedDir) -> bool {
    left.metadata()
        .ok()
        .zip(right.metadata().ok())
        .is_some_and(|(left, right)| (left.dev(), left.ino()) == (right.dev(), right.ino()))
}

fn publish(parent: &ContainedDir, temporary: &OsStr, name: &OsStr) -> io::Result<()> {
    let temporary = CString::new(temporary.as_bytes())?;
    let name = CString::new(name.as_bytes())?;
    let fd = parent.as_fd().as_raw_fd();
    // Both C strings contain only single path components and the parent fd
    // remains owned throughout. Publishing may never replace an existing name.
    // Use the Linux syscall directly because the static guest libc does not export renameat2.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            fd,
            temporary.as_ptr(),
            fd,
            name.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn checkout(
    directory: &ContainedDir,
    repository: &Repository<'_>,
    deadline: Instant,
    signals: &cancellation::Guard,
) -> Result<(), Failure> {
    let format = if repository.commit.len() == 40 {
        "--object-format=sha1"
    } else {
        "--object-format=sha256"
    };
    run(
        git(directory).args(["init", "--quiet", "--template=", format]),
        deadline,
        signals,
    )?;
    run(
        git(directory).args(["remote", "add", "--", "origin", repository.url]),
        deadline,
        signals,
    )?;
    run(
        git(directory).args([
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-recurse-submodules",
            "--depth=1",
            "--",
            "origin",
            repository.commit,
        ]),
        deadline,
        signals,
    )?;
    run(
        git(directory).args(["checkout", "--quiet", "--detach", repository.commit, "--"]),
        deadline,
        signals,
    )
}

fn verify_existing(
    directory: &ContainedDir,
    approved: &str,
    deadline: Instant,
    signals: &cancellation::Guard,
) -> Result<(), Failure> {
    let git_dir = directory
        .descend(OsStr::new(".git"))
        .map_err(|_| Failure::RepositoryDestination)?;
    git_dir
        .descend(OsStr::new("objects"))
        .map_err(|_| Failure::RepositoryDestination)?;
    git_dir
        .open_file(OsStr::new("HEAD"), ContainedOpenOptions::read_only())
        .map_err(|_| Failure::RepositoryDestination)?;
    let config = git_dir
        .open_file(OsStr::new("config"), ContainedOpenOptions::read_only())
        .map_err(|_| Failure::RepositoryDestination)?;
    if config.metadata().map_err(|_| Failure::RepositoryDestination)?.len()
        > capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES as u64
    {
        return Err(Failure::RepositoryDestination);
    }
    // Parse only the pinned config descriptor. The fresh process does not load
    // repository settings, includes, redirects, hooks, or credential helpers.
    let mut child = git(&git_dir)
        .args([
            "config",
            "--file=-",
            "--no-includes",
            "--null",
            "--get-all",
            "remote.origin.url",
        ])
        .stdin(config)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(spawn_failure)?;
    let mut bytes = Vec::new();
    let pipe = child.stdout.take().expect("Git config command has piped stdout");
    let capture = pipe.take(approved.len() as u64 + 2).read_to_end(&mut bytes);
    supervise_child(&mut child, deadline, signals).map_err(|error| match error {
        Failure::Exit { .. } => Failure::RepositoryDestination,
        error => error,
    })?;
    capture.map_err(|_| Failure::RepositoryDestination)?;
    if CStr::from_bytes_with_nul(&bytes).is_ok_and(|url| url.to_bytes() == approved.as_bytes()) {
        Ok(())
    } else {
        Err(Failure::RepositoryDestination)
    }
}

fn git(directory: &ContainedDir) -> Command {
    let cwd = PathBuf::from(format!("/proc/self/fd/{}", directory.as_fd().as_raw_fd()));
    let mut command = Command::new("git");
    command
        .env_clear()
        .env(
            "PATH",
            std::env::var_os("PATH").unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".into()),
        )
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ALLOW_PROTOCOL", "https")
        .env("GIT_SSL_CAINFO", "/etc/ssl/certs/ca-certificates.crt")
        .args(["-c", "http.followRedirects=false", "-c", "core.hooksPath=/dev/null"])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    command
}

fn spawn_failure(error: io::Error) -> Failure {
    if error.kind() == io::ErrorKind::NotFound {
        Failure::RepositoryGitUnavailable
    } else {
        Failure::Spawn
    }
}

fn run(command: &mut Command, deadline: Instant, signals: &cancellation::Guard) -> Result<(), Failure> {
    let mut child = command.spawn().map_err(spawn_failure)?;
    supervise_child(&mut child, deadline, signals).map_err(|error| match error {
        Failure::Exit { .. } => Failure::RepositoryCheckout,
        error => error,
    })
}
