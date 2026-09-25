use super::*;
use std::io::Write;

// ---- sanitize_file_path ----

#[test]
fn sanitize_strips_script_tags() {
    // The `/` inside `</script>` is in the allowlist and survives, so the
    // output keeps it. The < > ( ) are dropped.
    let r = sanitize_file_path("<script>alert(1)</script>.txt").unwrap();
    assert_eq!(r, "scriptalert1/script.txt");
}

#[test]
fn sanitize_strips_null_bytes() {
    let r = sanitize_file_path("foo\0bar.txt").unwrap();
    assert_eq!(r, "foobar.txt");
}

#[test]
fn sanitize_strips_unicode() {
    let r = sanitize_file_path("foo\u{200B}bar.txt").unwrap();
    assert_eq!(r, "foobar.txt");
}

#[test]
fn sanitize_rejects_dot_dot() {
    let err = sanitize_file_path("../etc/passwd").unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
}

#[test]
fn sanitize_rejects_embedded_dot_dot() {
    let err = sanitize_file_path("foo/../bar").unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
}

#[test]
fn sanitize_collapses_slashes() {
    let result = sanitize_file_path("foo//bar///baz");
    assert_eq!(result.unwrap(), "foo/bar/baz");
}

#[test]
fn sanitize_strips_leading_slash() {
    let result = sanitize_file_path("/foo/bar");
    assert_eq!(result.unwrap(), "foo/bar");
}

#[test]
fn sanitize_rejects_empty() {
    let err = sanitize_file_path("").unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
}

#[test]
fn sanitize_preserves_valid_path() {
    let result = sanitize_file_path("foo/bar.txt");
    assert_eq!(result.unwrap(), "foo/bar.txt");
}

#[test]
fn sanitize_preserves_hyphens_underscores_dots() {
    let result = sanitize_file_path("my-file_v2.tar.gz");
    assert_eq!(result.unwrap(), "my-file_v2.tar.gz");
}

// ---- new tests ----

#[test]
fn sanitize_rejects_only_slashes() {
    // Several slashes collapse + leading-strip to empty, then the empty
    // check fires. Confirms the order: collapse → strip → reject empty.
    let err = sanitize_file_path("///").unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(err.1, "empty path after sanitization");
}

#[test]
fn sanitize_rejects_dot_dot_after_filter() {
    // Disallowed characters drop out before the `..` check, so `.<>.`
    // collapses to `..` and is correctly rejected as traversal -- proves
    // the filter runs before the traversal check, not after it.
    let err = sanitize_file_path(".<>.").unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(err.1, "path traversal rejected");
}

// ---- file typing ----

fn typed(name: &str, data: &[u8]) -> FileType {
    identify_bytes(std::path::Path::new(name), data)
}

#[test]
fn the_file_and_bytes_paths_agree() {
    let dir = tempfile::tempdir().unwrap();
    let txt = dir.path().join("a.py");
    std::fs::File::create(&txt)
        .unwrap()
        .write_all(b"print('hi')\n")
        .unwrap();

    let from_file = identify_file(&txt, &mut std::fs::File::open(&txt).unwrap());

    assert_eq!(from_file, typed("a.py", b"print('hi')\n"));
    assert_eq!(
        (from_file.label, from_file.mime, from_file.is_text),
        ("python", "text/x-python", true)
    );
}

#[test]
fn known_extensions_name_the_type() {
    assert_eq!(typed("README.MD", b"# Title").label, "markdown");
    assert_eq!(typed("x.rs", b"fn main() {}").mime, "text/x-rust");
    assert_eq!(typed("logo.svg", b"<svg/>").mime, "image/svg+xml");
    let png = typed("shot.png", b"\x89PNG\r\n\x1a\n\0\0");
    assert_eq!((png.mime, png.is_text), ("image/png", false));
}

#[test]
fn unknown_extensions_fall_back_to_utf8_detection() {
    assert_eq!(typed("Makefile", b"all:\n\tcc x.c\n"), FileType::TEXT);
    assert_eq!(typed("notes", "d\u{e9}j\u{e0} vu\n".as_bytes()), FileType::TEXT);
    assert_eq!(typed("blob.bin", b"\x00\x01\x02"), FileType::UNKNOWN);
    assert_eq!(typed("latin1", b"caf\xe9\n"), FileType::UNKNOWN);
    assert_eq!(typed("empty", b""), FileType::TEXT);
}

#[test]
fn a_text_extension_on_binary_content_is_not_believed() {
    assert_eq!(typed("fake.txt", b"MZ\x90\x00\x03"), FileType::UNKNOWN);
    assert_eq!(typed("fake.json", b"\xff\xfe{"), FileType::UNKNOWN);
}

#[test]
fn a_character_cut_by_the_probe_still_counts_as_text() {
    let mut data = vec![b'a'; TEXT_PROBE_BYTES - 1];
    data.extend_from_slice("\u{e9}".as_bytes()); // 2 bytes: the probe keeps the first
    assert_eq!(typed("long.log", &data).mime, "text/plain");
}

/// The answer depends only on the name and the bytes -- never on a model.
///
/// Magika v1 read a `key: value` line in a `.txt` file as CSS or CSV depending
/// on a random nonce, so a complete gate failed one run in several. Typing is
/// now deterministic: a short generated payload types as its extension says.
#[test]
fn a_short_key_value_line_types_as_its_extension() {
    for nonce in ["0000000000000000b135823cc1684885", "deadbeefdeadbeefdeadbeefdeadbeef"] {
        let line = format!("upload:fps-{nonce}\n");
        assert_eq!(typed("upload.txt", line.as_bytes()).mime, "text/plain", "{nonce}");
    }
}

/// The files API used to strip a leading `/` silently, so `/root/app.py` landed
/// at `/root/root/app.py` in the VM and nothing said so. An absolute path now
/// means the path the caller sees; `exact` keeps the literal workspace form.
mod paths {
    use super::*;

    fn landed(raw: &str, exact: bool, container: bool) -> (String, String, Option<String>) {
        let path = resolve_file_path(raw, exact, container).unwrap();
        (path.relative, path.vm_path, path.container_path)
    }

    fn refused(raw: &str, container: bool) -> String {
        let AppError(status, message) = resolve_file_path(raw, false, container).unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        message
    }

    #[test]
    fn a_vm_path_under_root_lands_where_the_vm_sees_it() {
        assert_eq!(
            landed("/root/app.py", false, false),
            ("app.py".into(), "/root/app.py".into(), None)
        );
        assert_eq!(
            landed("app.py", false, false),
            ("app.py".into(), "/root/app.py".into(), None)
        );
        assert_eq!(landed("/root/src//main.rs", false, false).0, "src/main.rs");
    }

    #[test]
    fn a_container_path_under_its_workspace_lands_in_the_same_file() {
        assert_eq!(
            landed("/workspace/app.py", false, true),
            ("app.py".into(), "/root/app.py".into(), Some("/workspace/app.py".into()))
        );
        assert_eq!(landed("app.py", false, true).2, Some("/workspace/app.py".into()));
    }

    #[test]
    fn an_absolute_path_outside_the_workspace_is_refused_with_the_reason() {
        assert!(refused("/etc/passwd", false).contains("/root"));
        let container = refused("/root/app.py", true);
        assert!(container.contains("container"), "{container}");
        assert!(container.contains("/workspace"), "{container}");
        assert!(refused("/app/config.json", true).contains("container"));
    }

    #[test]
    fn exact_takes_the_path_literally_inside_the_workspace() {
        assert_eq!(landed("/root/app.py", true, false).0, "root/app.py");
        assert_eq!(landed("/root/app.py", true, false).1, "/root/root/app.py");
        assert_eq!(landed("/etc/app.py", true, true).0, "etc/app.py");
    }

    #[test]
    fn traversal_and_the_bare_workspace_are_refused_in_every_form() {
        for raw in ["/root/../etc/passwd", "../x", "/root", "/root/", "/workspace/../x"] {
            assert!(
                resolve_file_path(raw, false, raw.starts_with("/workspace")).is_err(),
                "{raw}"
            );
        }
    }

    #[test]
    fn a_directory_listing_may_name_the_workspace_root() {
        for (raw, container) in [("", false), ("/root", false), ("/root/", false), ("/workspace", true)] {
            assert_eq!(resolve_dir_path(raw, false, container).unwrap(), "", "{raw}");
        }
        assert_eq!(resolve_dir_path("/root/src", false, false).unwrap(), "src");
    }
}
