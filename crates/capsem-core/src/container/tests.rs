use super::*;

#[test]
fn image_names_skip_existing_sessions_without_reusing_them() {
    assert_eq!(available_name("redis", &[]).unwrap(), "redis");
    assert_eq!(
        available_name("redis", &["Redis".into(), "redis-2".into()]).unwrap(),
        "redis-3"
    );
}

#[test]
fn qualified_images_derive_names_without_registry_tags_or_digest() {
    for reference in [
        "docker://redis:7-alpine",
        "ghcr.io/team/redis:latest",
        "registry.example:5443/team/redis",
    ] {
        assert_eq!(image_name(reference).unwrap().as_deref(), Some("redis"));
    }
}

#[test]
fn shell_commands_remain_commands_and_bad_image_urls_fail_closed() {
    for command in [
        "echo hello",
        "true",
        "/usr/bin/id",
        "python -c 'print(1)'",
        "./script.sh",
    ] {
        assert!(image_name(command).unwrap().is_none());
    }
    for image in [
        "docker://",
        "docker://redis;echo",
        "ghcr.io/team/redis@sha256:bad",
        "https://example.com/image",
    ] {
        assert!(
            image_name(image).is_err(),
            "invalid image became a shell command: {image}"
        );
    }
}
