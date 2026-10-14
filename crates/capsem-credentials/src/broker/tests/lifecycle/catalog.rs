use super::*;

#[tokio::test]
async fn connection_catalog_pages_are_finite_complete_and_survive_deleted_cursors() {
    let (_fixture, connection) = verified().await;
    let provider = Arc::new(Provider::default());
    let broker = ConnectionBroker::open(
        BrokerStorage::Memory,
        vec![profile(Arc::clone(&provider))],
        authority(),
        limits(),
    )
    .await
    .unwrap();
    let empty = broker
        .list_page(&api::ConnectionListQuery {
            limit: Some(1),
            cursor: None,
        })
        .await
        .unwrap();
    assert!(empty.connections.is_empty());
    assert!(empty.next_cursor.is_none());
    for _ in 0..5 {
        let sender = provider.queue();
        let started = broker.authorize(consent()).await.unwrap();
        sender.send(connection.clone()).unwrap();
        joined(&broker, &started.transaction.id).await;
    }
    let complete = broker.list().await.unwrap();
    let expected: Vec<_> = complete.connections.iter().map(|entry| entry.id.clone()).collect();
    let mut query = api::ConnectionListQuery {
        limit: Some(2),
        cursor: None,
    };
    let mut actual = vec![];
    loop {
        let page = broker.list_page(&query).await.unwrap();
        assert!(page.connections.len() <= 2);
        assert_eq!(page.scope_profiles, complete.scope_profiles);
        actual.extend(page.connections.iter().map(|entry| entry.id.clone()));
        if page.next_cursor.is_none() {
            break;
        }
        assert_eq!(
            page.next_cursor.as_ref(),
            page.connections.last().map(|entry| &entry.id)
        );
        query.cursor = page.next_cursor;
    }
    assert_eq!(actual, expected);
    let cursor = expected[1].clone();
    broker.0.state.lock().await.connections.remove(&cursor);
    let page = broker
        .list_page(&api::ConnectionListQuery {
            limit: Some(8),
            cursor: Some(cursor),
        })
        .await
        .unwrap();
    assert_eq!(
        page.connections
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>(),
        expected[2..]
    );
    for limit in [0, 9, u32::MAX] {
        assert_eq!(
            broker
                .list_page(&api::ConnectionListQuery {
                    limit: Some(limit),
                    cursor: None
                })
                .await
                .unwrap_err(),
            api::CredentialErrorCode::RequestConflict
        );
    }
    for cursor in [
        "",
        "private-query",
        "00000000-0000-0000-0000-000000000000",
        "12345678-1234-4234-8234-123456789ABC",
    ] {
        assert_eq!(
            broker
                .list_page(&api::ConnectionListQuery {
                    limit: None,
                    cursor: Some(cursor.into())
                })
                .await
                .unwrap_err(),
            api::CredentialErrorCode::RequestConflict
        );
    }
}
