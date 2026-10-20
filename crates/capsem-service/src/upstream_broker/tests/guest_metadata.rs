use super::*;
use capsem_proto::upstream_grant::GuestFileTime;
use std::os::unix::fs::{symlink, PermissionsExt as _};

#[tokio::test]
async fn guest_mode_requests_are_confined_to_the_session_share() {
    let session = tempfile::tempdir().unwrap();
    let guest = session.path().join(capsem_core::GUEST_SHARE_DIR);
    std::fs::create_dir_all(guest.join("workspace/nested")).unwrap();
    std::fs::write(guest.join("workspace/nested/tool"), b"tool").unwrap();
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret");
    std::fs::write(&secret, b"secret").unwrap();
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&secret, guest.join("workspace/link")).unwrap();

    let (client, _publisher, authority, task) =
        start_broker_for_session(test_policy("policy-a", None, Vec::new()), session.path().to_path_buf());
    let (response, fds) = client
        .request(&UpstreamGrantRequest::SetGuestMode {
            request_id: 1,
            relative_path: b"workspace/nested/tool".to_vec(),
            mode: 0o751,
        })
        .await;
    assert_eq!(response, UpstreamGrantResponse::GuestModeSet { request_id: 1 });
    assert!(fds.is_empty());
    assert_eq!(
        std::fs::metadata(guest.join("workspace/nested/tool"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o751
    );

    let (response, fds) = client
        .request(&UpstreamGrantRequest::SetGuestMode {
            request_id: 2,
            relative_path: b"workspace/link".to_vec(),
            mode: 0o777,
        })
        .await;
    assert_eq!(
        response,
        UpstreamGrantResponse::Denied {
            request_id: 2,
            reason: UpstreamGrantDenial::NotAllowed,
        }
    );
    assert!(fds.is_empty());
    assert_eq!(std::fs::metadata(secret).unwrap().permissions().mode() & 0o7777, 0o600);
    stop_broker(authority, task).await;
}

#[tokio::test]
async fn guest_timestamp_requests_are_confined_to_the_session_share() {
    let session = tempfile::tempdir().unwrap();
    let guest = session.path().join(capsem_core::GUEST_SHARE_DIR);
    std::fs::create_dir_all(guest.join("workspace/nested")).unwrap();
    let tool = guest.join("workspace/nested/tool");
    std::fs::write(&tool, b"tool").unwrap();
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret");
    std::fs::write(&secret, b"secret").unwrap();
    symlink(&secret, guest.join("workspace/link")).unwrap();

    let (client, _publisher, authority, task) =
        start_broker_for_session(test_policy("policy-a", None, Vec::new()), session.path().to_path_buf());
    let timestamp = 1_700_000_123_i64;
    let (response, fds) = client
        .request(&UpstreamGrantRequest::SetGuestTimes {
            request_id: 1,
            relative_path: b"workspace/nested/tool".to_vec(),
            atime: GuestFileTime::Omit,
            mtime: GuestFileTime::Exact {
                seconds: timestamp,
                nanoseconds: 456_000_000,
            },
        })
        .await;
    assert_eq!(response, UpstreamGrantResponse::GuestTimesSet { request_id: 1 });
    assert!(fds.is_empty());
    assert_eq!(
        std::fs::metadata(&tool)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap(),
        Duration::new(timestamp.cast_unsigned(), 456_000_000)
    );

    let secret_modified = std::fs::metadata(&secret).unwrap().modified().unwrap();
    let (response, fds) = client
        .request(&UpstreamGrantRequest::SetGuestTimes {
            request_id: 2,
            relative_path: b"workspace/link".to_vec(),
            atime: GuestFileTime::Now,
            mtime: GuestFileTime::Now,
        })
        .await;
    assert_eq!(response, UpstreamGrantResponse::GuestTimesSet { request_id: 2 });
    assert!(fds.is_empty());
    assert_eq!(std::fs::metadata(secret).unwrap().modified().unwrap(), secret_modified);
    stop_broker(authority, task).await;
}
