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

async fn write_events(registry: &NetworkRegistry, id: Uuid, count: usize, first_ms: i64) {
    use capsem_logger::{TransportEvent, TransportEventKind, WriteOp};
    let entry = registry.networks.get(&id).unwrap();
    // Ids derive from the timestamp too: the ledger keeps one row per event
    // id, so a second batch with the same ids would be silently dropped.
    for n in (first_ms as usize)..(first_ms as usize + count) {
        let facts = serde_json::json!({
            "network": {"context": "flow", "source": {"vm": {"id": format!("vm-{}", n % 2)}}, "destination": {"vm": {"id": "vm-9"}}},
            "decision": {"effective": if n % 3 == 0 { "block" } else { "allow" }},
        });
        let event = TransportEvent::new(
            format!("{:012x}", 0xabc000 + n),
            n as i64,
            if n % 2 == 0 {
                TransportEventKind::Connect
            } else {
                TransportEventKind::Close
            },
            Some(id),
            Some(Uuid::from_u128(1000 + n as u128)),
            &facts,
        )
        .unwrap();
        entry.handle.write(WriteOp::TransportEvent(event)).await.unwrap();
    }
    entry.handle.flush().await.unwrap();
}

#[tokio::test]
async fn logs_page_oldest_first_and_the_cursor_continues_across_appends() {
    let (_dir, root) = root();
    let mut registry = NetworkRegistry::new(root);
    let net = registry.create("audited", 1).await.unwrap();
    write_events(&registry, net.id, 7, 100).await;
    let first = registry
        .logs(
            net.id,
            &LogQuery {
                limit: Some(3),
                ..LogQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        first.events.iter().map(|e| e.sequence).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert!(first.next_cursor.is_some(), "a full page has more");
    write_events(&registry, net.id, 2, 200).await;
    let second = registry
        .logs(
            net.id,
            &LogQuery {
                cursor: first.next_cursor.clone(),
                limit: Some(100),
                ..LogQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        second.events.iter().map(|e| e.sequence).collect::<Vec<_>>(),
        vec![4, 5, 6, 7, 8, 9]
    );
    assert!(
        second.next_cursor.is_none(),
        "not a full page: everything so far was seen"
    );
    let idle = registry
        .logs(
            net.id,
            &LogQuery {
                cursor: Some(second.cursor.clone()),
                ..LogQuery::default()
            },
        )
        .await
        .unwrap();
    assert!(idle.events.is_empty());
    assert_eq!(idle.cursor, second.cursor, "an empty poll keeps the same cursor");
}

#[tokio::test]
async fn log_filters_select_by_vm_connection_type_decision_and_time() {
    let (_dir, root) = root();
    let mut registry = NetworkRegistry::new(root);
    let net = registry.create("audited", 1).await.unwrap();
    write_events(&registry, net.id, 6, 100).await;
    let by_vm = registry
        .logs(
            net.id,
            &LogQuery {
                vm: Some("vm-1".into()),
                ..LogQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        by_vm.events.iter().map(|e| e.sequence).collect::<Vec<_>>(),
        vec![2, 4, 6]
    );
    let by_destination = registry
        .logs(
            net.id,
            &LogQuery {
                vm: Some("vm-9".into()),
                ..LogQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(by_destination.events.len(), 6, "either endpoint matches");
    let by_type = registry
        .logs(
            net.id,
            &LogQuery {
                event_type: Some("network.close".into()),
                ..LogQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(by_type.events.len(), 3);
    let blocked = registry
        .logs(
            net.id,
            &LogQuery {
                decision: Some("block".into()),
                ..LogQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        blocked.events.iter().map(|e| e.sequence).collect::<Vec<_>>(),
        vec![3, 6]
    );
    let window = registry
        .logs(
            net.id,
            &LogQuery {
                since_unix_ms: Some(102),
                until_unix_ms: Some(104),
                ..LogQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        window.events.iter().map(|e| e.timestamp_unix_ms).collect::<Vec<_>>(),
        vec![102, 103, 104]
    );
    let connection = Uuid::from_u128(1102).to_string();
    let by_connection = registry
        .logs(
            net.id,
            &LogQuery {
                connection: Some(connection.clone()),
                ..LogQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(by_connection.events.len(), 1);
    assert_eq!(
        by_connection.events[0].connection_id.as_deref(),
        Some(connection.as_str())
    );
}

#[tokio::test]
async fn cursors_are_bound_to_their_network_and_filters_and_limits_are_bounded() {
    let (_dir, root) = root();
    let mut registry = NetworkRegistry::new(root);
    let a = registry.create("alpha", 1).await.unwrap();
    let b = registry.create("beta", 1).await.unwrap();
    write_events(&registry, a.id, 2, 100).await;
    let page = registry.logs(a.id, &LogQuery::default()).await.unwrap();
    let other = registry
        .logs(
            b.id,
            &LogQuery {
                cursor: Some(page.cursor.clone()),
                ..LogQuery::default()
            },
        )
        .await;
    assert!(
        matches!(other, Err(NetworkError::Cursor(ref reason)) if reason.contains("another network")),
        "{other:?}"
    );
    let refiltered = registry
        .logs(
            a.id,
            &LogQuery {
                cursor: Some(page.cursor.clone()),
                vm: Some("vm-0".into()),
                ..LogQuery::default()
            },
        )
        .await;
    assert!(
        matches!(refiltered, Err(NetworkError::Cursor(ref reason)) if reason.contains("different filters")),
        "{refiltered:?}"
    );
    for bad in ["nope", "x.y.z", &format!("{}.0.-1", a.id)] {
        let result = registry
            .logs(
                a.id,
                &LogQuery {
                    cursor: Some(bad.to_string()),
                    ..LogQuery::default()
                },
            )
            .await;
        assert!(matches!(result, Err(NetworkError::Cursor(_))), "{bad}: {result:?}");
    }
    for limit in [0, MAX_LOG_LIMIT + 1] {
        let result = registry
            .logs(
                a.id,
                &LogQuery {
                    limit: Some(limit),
                    ..LogQuery::default()
                },
            )
            .await;
        assert!(matches!(result, Err(NetworkError::Cursor(_))), "limit {limit}");
    }
}

#[tokio::test]
async fn a_retired_network_keeps_its_history_and_a_new_same_name_network_starts_empty() {
    let (_dir, root) = root();
    let mut registry = NetworkRegistry::new(root.clone());
    let old = registry.create("team", 1).await.unwrap();
    write_events(&registry, old.id, 3, 100).await;
    registry.retire(old.id, 2).await.unwrap();
    let new = registry.create("team", 3).await.unwrap();
    let mut reloaded = NetworkRegistry::load(root).await.unwrap();
    let history = reloaded.logs(old.id, &LogQuery::default()).await.unwrap();
    assert_eq!(
        history.events.len(),
        3,
        "retired history is read through a lazily opened reader"
    );
    let fresh = reloaded.logs(new.id, &LogQuery::default()).await.unwrap();
    assert!(fresh.events.is_empty());
    let unknown = Uuid::new_v4();
    assert_eq!(
        reloaded.logs(unknown, &LogQuery::default()).await.unwrap_err(),
        NetworkError::NotFound(unknown)
    );
}

#[tokio::test]
async fn paging_while_a_writer_appends_neither_repeats_nor_skips_a_row() {
    // The cursor is the last row id seen. Appends land above it, so a reader
    // racing the writer sees every row exactly once even when its pages
    // straddle a flush. This is also the reader-contention evidence: the
    // pages complete while the writer is at full tilt, and the read path
    // never calls `flush` -- it waits on `ready` and issues one SELECT.
    let (_dir, root) = root();
    let mut registry = NetworkRegistry::new(root);
    let net = registry.create("busy", 1).await.unwrap();
    let handle = registry.reader(net.id).unwrap();
    const TOTAL: usize = 2_000;
    let writer = tokio::spawn({
        let handle = std::sync::Arc::clone(&handle);
        async move {
            use capsem_logger::{TransportEvent, TransportEventKind, WriteOp};
            for n in 0..TOTAL {
                let event = TransportEvent::new(
                    format!("{:012x}", 0xd00000 + n),
                    1_000 + n as i64,
                    TransportEventKind::Connect,
                    Some(net.id),
                    Some(Uuid::from_u128(5_000 + n as u128)),
                    &serde_json::json!({ "n": n }),
                )
                .unwrap();
                handle.write(WriteOp::TransportEvent(event)).await.unwrap();
                if n % 250 == 0 {
                    handle.flush().await.unwrap();
                }
            }
            handle.flush().await.unwrap();
        }
    });
    let started = std::time::Instant::now();
    let mut seen: Vec<i64> = Vec::new();
    let mut cursor = None;
    let mut pages = 0usize;
    while seen.len() < TOTAL {
        let page = registry
            .logs(
                net.id,
                &LogQuery {
                    cursor: cursor.clone(),
                    limit: Some(100),
                    ..LogQuery::default()
                },
            )
            .await
            .unwrap();
        pages += 1;
        seen.extend(page.events.iter().map(|event| event.sequence));
        cursor = Some(page.cursor);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(60),
            "paging stalled at {}",
            seen.len()
        );
        if page.events.is_empty() {
            tokio::task::yield_now().await;
        }
    }
    writer.await.unwrap();
    assert_eq!(seen, (1..=TOTAL as i64).collect::<Vec<_>>(), "every row once, in order");
    eprintln!(
        "paged {TOTAL} rows in {pages} pages while the writer appended: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_network_database_without_its_ledger_table_fails_loudly() {
    // Missing table means broken schema, not empty history.
    let (_dir, root) = root();
    let mut registry = NetworkRegistry::new(root.clone());
    let net = registry.create("broken", 1).await.unwrap();
    registry.retire(net.id, 2).await.unwrap();
    let path = capsem_foundation::paths::network_db_path_in(&root, &net.id.to_string());
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("DROP TABLE transport_events")
        .unwrap();
    let history = registry.logs(net.id, &LogQuery::default()).await;
    assert!(
        matches!(history, Err(NetworkError::Database { .. })),
        "a dropped ledger table must not read as empty: {history:?}"
    );
    // Nor does the service start over it as if the network never existed.
    let reloaded = NetworkRegistry::load(root).await;
    assert!(
        matches!(reloaded, Err(NetworkError::Database { .. })),
        "loading over a broken network database must refuse: {reloaded:?}"
    );
}
