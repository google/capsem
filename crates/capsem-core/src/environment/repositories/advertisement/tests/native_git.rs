//! Native Git is the independent advertisement oracle, never a product helper.
use super::*;
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

fn git(directory: &Path, arguments: &[&str], input: &[u8], protocol: Option<&str>) -> Vec<u8> {
    let mut command = Command::new("git");
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .current_dir(directory)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(protocol) = protocol {
        command.env("GIT_PROTOCOL", protocol);
    }
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    output.stdout
}
#[test]
fn actual_upload_pack_advertisements_select_exact_commits_and_peeled_tags() {
    for format in ["sha1", "sha256"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(
            root,
            &["init", "--bare", &format!("--object-format={format}"), "."],
            b"",
            None,
        );
        let tree = String::from_utf8(git(root, &["mktree"], b"", None)).unwrap();
        let commit = String::from_utf8(git(
            root,
            &["commit-tree", tree.trim(), "-m", "fixture commit"],
            b"",
            None,
        ))
        .unwrap();
        let commit = commit.trim();
        git(root, &["update-ref", "refs/heads/main", commit], b"", None);
        git(root, &["symbolic-ref", "HEAD", "refs/heads/main"], b"", None);
        git(root, &["tag", "light", commit], b"", None);
        git(
            root,
            &["tag", "-a", "-m", "release fixture", "release", commit],
            b"",
            None,
        );
        for protocol in [None, Some("version=1")] {
            let mut bytes = packet("# service=git-upload-pack\n");
            bytes.extend_from_slice(b"0000");
            bytes.extend(git(
                root,
                &["upload-pack", "--strict", "--stateless-rpc", "--advertise-refs", "."],
                b"",
                protocol,
            ));
            for name in [
                "HEAD",
                "main",
                "refs/heads/main",
                "light",
                "release",
                "refs/tags/release",
            ] {
                assert_eq!(resolve(&bytes, name).unwrap(), commit, "{format} {protocol:?} {name}");
            }
        }
    }
}
