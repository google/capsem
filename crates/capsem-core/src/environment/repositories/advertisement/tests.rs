use super::*;

fn packet(line: &str) -> Vec<u8> {
    let mut bytes = format!("{:04x}", line.len() + 4).into_bytes();
    bytes.extend_from_slice(line.as_bytes());
    bytes
}
pub(in crate::environment::repositories) fn advertisement(refs: &[(&str, char)], format: &str) -> Vec<u8> {
    let mut bytes = packet("# service=git-upload-pack\n");
    bytes.extend_from_slice(b"0000");
    for (index, (name, character)) in refs.iter().enumerate() {
        let sha = character.to_string().repeat(if format == "sha256" { 64 } else { 40 });
        let capabilities = if index == 0 {
            format!("\0multi_ack object-format={format}")
        } else {
            String::new()
        };
        bytes.extend(packet(&format!("{sha} {name}{capabilities}\n")));
    }
    bytes.extend_from_slice(b"0000");
    bytes
}

#[test]
fn selects_exact_head_branches_lightweight_and_peeled_tags_for_both_object_formats() {
    for format in ["sha1", "sha256"] {
        let bytes = advertisement(
            &[
                ("HEAD", 'a'),
                ("refs/heads/main", 'a'),
                ("refs/tags/light", 'b'),
                ("refs/tags/release", 'c'),
                ("refs/tags/release^{}", 'd'),
            ],
            format,
        );
        let size = if format == "sha256" { 64 } else { 40 };
        for (selection, character) in [
            ("HEAD", 'a'),
            ("main", 'a'),
            ("refs/heads/main", 'a'),
            ("light", 'b'),
            ("release", 'd'),
            ("refs/tags/release", 'd'),
        ] {
            assert_eq!(resolve(&bytes, selection).unwrap(), character.to_string().repeat(size));
        }
    }
}

#[test]
fn never_guesses_between_a_branch_and_tag_or_accepts_unadvertised_object_ids() {
    let bytes = advertisement(&[("refs/heads/release", 'a'), ("refs/tags/release", 'b')], "sha1");
    assert_eq!(resolve(&bytes, "release"), Err(Error::AmbiguousReference));
    assert_eq!(resolve(&bytes, "refs/heads/release").unwrap(), "a".repeat(40));
    assert_eq!(resolve(&bytes, "missing"), Err(Error::UnknownReference));
    assert_eq!(resolve(&bytes, &"f".repeat(40)), Err(Error::UnknownReference));
    assert_eq!(resolve(&bytes, &"a".repeat(40)).unwrap(), "a".repeat(40));
}

#[test]
fn refuses_truncation_trailing_data_bad_capabilities_duplicates_and_invalid_ids() {
    let valid = advertisement(&[("HEAD", 'a'), ("refs/heads/main", 'b')], "sha1");
    for index in 0..valid.len() {
        assert!(resolve(&valid[..index], "main").is_err(), "truncated at {index}");
    }
    let mut trailing = valid.clone();
    trailing.extend_from_slice(b"0000");
    assert!(resolve(&trailing, "main").is_err());
    let duplicate = advertisement(&[("refs/heads/main", 'a'), ("refs/heads/main", 'a')], "sha1");
    assert_eq!(resolve(&duplicate, "main"), Err(Error::InvalidAdvertisement));
    for invalid in [
        "refs/heads/../main",
        "refs/heads/with space",
        "refs/heads/.hidden",
        "refs/heads/main.lock",
    ] {
        assert!(resolve(&advertisement(&[(invalid, 'a')], "sha1"), "main").is_err());
    }
    let invalid_id = advertisement(&[("refs/heads/main", 'g')], "sha1");
    assert!(resolve(&invalid_id, "main").is_err());
    let mut bad_caps = valid;
    let start = bad_caps.iter().position(|byte| *byte == 0).unwrap();
    bad_caps[start] = b' ';
    assert!(resolve(&bad_caps, "main").is_err());
}

#[test]
fn checks_the_entire_advertisement_before_returning_an_early_matching_reference() {
    let mut bytes = advertisement(&[("HEAD", 'a'), ("refs/heads/main", 'b')], "sha1");
    bytes.truncate(bytes.len() - 4);
    bytes.extend(packet("malformed tail\n"));
    bytes.extend_from_slice(b"0000");
    assert_eq!(resolve(&bytes, "HEAD"), Err(Error::InvalidAdvertisement));
}

#[test]
fn advertised_object_names_are_case_insensitive_and_return_canonical_pins() {
    let bytes = advertisement(&[("refs/heads/main", 'A')], "sha1");
    assert_eq!(resolve(&bytes, "main").unwrap(), "a".repeat(40));
}

mod native_git;

#[test]
fn empty_repository_and_unknown_or_inconsistent_object_formats_are_explicit() {
    let empty = advertisement(&[("capabilities^{}", '0')], "sha1");
    assert_eq!(resolve(&empty, "HEAD"), Err(Error::UnknownReference));
    let unsupported = advertisement(&[("refs/heads/main", 'a')], "future");
    assert_eq!(resolve(&unsupported, "main"), Err(Error::InvalidAdvertisement));
    let valid = advertisement(&[("refs/heads/main", 'a')], "sha1");
    let mut too_large = valid;
    too_large.resize(capsem_proto::ENVIRONMENT_MAX_REPOSITORY_ADVERTISEMENT_BYTES + 1, b'x');
    assert_eq!(resolve(&too_large, "main"), Err(Error::TooLarge));
    let mut orphan = advertisement(&[("refs/tags/release^{}", 'a')], "sha1");
    assert_eq!(resolve(&orphan, "release"), Err(Error::InvalidAdvertisement));
    orphan = advertisement(&[("refs/heads/main", 'a'), ("refs/heads/main^{}", 'b')], "sha1");
    assert_eq!(resolve(&orphan, "main"), Err(Error::InvalidAdvertisement));
}
