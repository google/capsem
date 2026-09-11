use super::*;

fn root() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("networks");
    (dir, root)
}

#[tokio::test]
async fn names_are_unique_among_active_networks_and_reuse_makes_a_new_network() {
    let (_dir, root) = root();
    let mut registry = NetworkRegistry::new(root.clone());
    let first = registry.create("shared", 1_000).await.unwrap();
    assert_eq!(first.name, "shared");
    assert_eq!(first.member_count, 0);
    assert_eq!(
        registry.create("shared", 1_001).await,
        Err(NetworkError::NameTaken {
            name: "shared".into(),
            id: first.id
        })
    );
    registry.retire(first.id, 2_000).await.unwrap();
    let second = registry.create("shared", 3_000).await.unwrap();
    assert_ne!(second.id, first.id, "a reused name is a different network");
    assert_eq!(registry.list().len(), 1);
    assert_eq!(registry.find("shared"), Some(second.id));
    assert!(
        network_db_path_in(&root, &first.id.to_string()).exists(),
        "history stays on disk"
    );
}

#[tokio::test]
async fn names_must_be_dns_labels() {
    let (_dir, root) = root();
    let mut registry = NetworkRegistry::new(root);
    for bad in [
        "",
        "-lead",
        "trail-",
        "Upper",
        "under_score",
        "dot.ted",
        &"x".repeat(64),
    ] {
        assert_eq!(
            registry.create(bad, 1).await,
            Err(NetworkError::InvalidName(bad.to_string())),
            "{bad:?}"
        );
    }
    registry.create(&"y".repeat(63), 1).await.unwrap();
}

#[tokio::test]
async fn membership_records_the_vm_and_its_address_and_blocks_retirement() {
    let (_dir, root) = root();
    let mut registry = NetworkRegistry::new(root);
    let net = registry.create("team", 1).await.unwrap();
    let address = Ipv4Addr::new(10, 128, 0, 2);
    registry
        .attach(net.id, "vm-a", address, MembershipState::Declared, 2)
        .await
        .unwrap();
    registry
        .attach(net.id, "vm-a", address, MembershipState::Ready, 3)
        .await
        .unwrap();
    registry
        .attach(net.id, "vm-b", Ipv4Addr::new(10, 128, 0, 3), MembershipState::Ready, 4)
        .await
        .unwrap();
    let members = registry.members(net.id).unwrap();
    assert_eq!(members.len(), 2);
    assert_eq!(
        members[0].state,
        MembershipState::Ready,
        "a second attach updates the state"
    );
    assert_eq!(members[0].updated_unix_ms, 3);
    assert_eq!(registry.memberships_of("vm-a"), vec![net.id]);
    assert_eq!(
        registry.retire(net.id, 5).await,
        Err(NetworkError::HasMembers { id: net.id, members: 2 })
    );
    assert_eq!(
        registry.detach(net.id, "vm-c", 6).await,
        Err(NetworkError::NotAMember {
            id: net.id,
            vm_id: "vm-c".into()
        })
    );
    assert_eq!(registry.detach_everywhere("vm-a", 7).await.unwrap(), vec![net.id]);
    registry.detach(net.id, "vm-b", 8).await.unwrap();
    registry.retire(net.id, 9).await.unwrap();
    assert_eq!(registry.retire(net.id, 10).await, Err(NetworkError::NotFound(net.id)));
}

#[tokio::test]
async fn a_restart_rebuilds_active_networks_and_members_from_their_databases() {
    let (_dir, root) = root();
    let (kept, retired) = {
        let mut registry = NetworkRegistry::new(root.clone());
        let kept = registry.create("kept", 10).await.unwrap();
        let retired = registry.create("gone", 11).await.unwrap();
        registry
            .attach(
                kept.id,
                "vm-a",
                Ipv4Addr::new(10, 128, 0, 2),
                MembershipState::Ready,
                12,
            )
            .await
            .unwrap();
        registry
            .attach(
                kept.id,
                "vm-b",
                Ipv4Addr::new(10, 128, 0, 3),
                MembershipState::Declared,
                13,
            )
            .await
            .unwrap();
        registry.detach(kept.id, "vm-b", 14).await.unwrap();
        registry.retire(retired.id, 15).await.unwrap();
        (kept, retired)
    };
    let registry = NetworkRegistry::load(root).await.unwrap();
    assert_eq!(
        registry.list(),
        vec![NetworkSummary {
            member_count: 1,
            ..kept
        }]
    );
    assert_eq!(
        registry.members(kept.id).unwrap(),
        vec![Member {
            vm_id: "vm-a".into(),
            address: Ipv4Addr::new(10, 128, 0, 2),
            state: MembershipState::Ready,
            updated_unix_ms: 12
        }]
    );
    assert_eq!(
        registry.summary(retired.id),
        None,
        "retired networks are history, not state"
    );
    assert_eq!(registry.find("gone"), None);
}

#[tokio::test]
async fn a_database_that_cannot_be_read_fails_startup_loudly() {
    let (_dir, root) = root();
    let id = Uuid::new_v4();
    let path = network_db_path_in(&root, &id.to_string());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"not a database").unwrap();
    let error = NetworkRegistry::load(root).await.unwrap_err();
    assert!(
        matches!(error, NetworkError::Database { path: ref failed, .. } if *failed == path),
        "{error}"
    );
}

#[tokio::test]
async fn an_empty_or_missing_root_is_an_empty_registry() {
    let (_dir, root) = root();
    assert!(NetworkRegistry::load(root.clone()).await.unwrap().list().is_empty());
    std::fs::create_dir_all(root.join("not-a-uuid")).unwrap();
    assert!(NetworkRegistry::load(root).await.unwrap().list().is_empty());
}
