use super::*;

#[test]
fn publications_are_explicit_loopback_port_pairs() {
    assert_eq!(
        "16379:6379".parse::<PortMapping>().unwrap(),
        PortMapping {
            host: 16379,
            guest: 6379
        }
    );
    assert_eq!("0:6379".parse::<PortMapping>().unwrap().host, 0);
    for invalid in ["6379", "0.0.0.0:6379:6379", "1:0", "65536:6379", "1:2/udp"] {
        assert!(invalid.parse::<PortMapping>().is_err(), "accepted {invalid}");
    }
}

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
