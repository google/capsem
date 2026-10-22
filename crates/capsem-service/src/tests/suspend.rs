use super::*;

#[tokio::test]
async fn rolled_back_suspend_retains_only_the_current_running_owner() {
    for replaced in [false, true] {
        let (state, _directory) = make_test_state_with_tempdir();
        let id = capsem_crypto::uuid_v4().to_string();
        let session = state.run_dir.join("sessions").join(&id);
        std::fs::create_dir_all(&session).unwrap();
        insert_fake_instance_with_session_dir(&state, &id, std::process::id(), session);
        let socket = state.instance_socket_path(&id).unwrap();
        std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
        let (socket, original_generation) = {
            let mut instances = state.instances.lock().unwrap();
            let instance = instances.get_mut(&id).unwrap();
            instance.persistent = true;
            instance.uds_path = socket;
            let result = (instance.uds_path.clone(), instance.generation);
            drop(instances);
            result
        };
        let replacement = capsem_crypto::uuid_v4();
        let owner = {
            let state = Arc::clone(&state);
            let id = id.clone();
            spawn_fake_process(&socket, 2, move |message| {
                let reply = match message {
                    ServiceToProcess::Suspend => {
                        if replaced {
                            state.instances.lock().unwrap().get_mut(&id).unwrap().generation = replacement;
                        }
                        ProcessToService::SuspendRolledBack {
                            id: id.clone(),
                            error: "terminal input is busy".into(),
                        }
                    }
                    ServiceToProcess::Ping => ProcessToService::Pong,
                    other => panic!("unexpected rollback-test request: {other:?}"),
                };
                Box::pin(async move { Some(reply) })
            })
        };
        let router = build_service_router(Arc::clone(&state));
        let (status, error) = route_request(
            router.clone(),
            axum::http::Method::POST,
            &format!("/vms/{id}/pause"),
            Some(json!({})),
        )
        .await;
        if replaced {
            assert_eq!(status, StatusCode::BAD_GATEWAY, "{error}");
            assert!(error["error"].as_str().unwrap().contains("owner changed"));
        } else {
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{error}");
            assert_eq!(
                error["error"],
                "suspend failed; session remains running: terminal input is busy"
            );
        }
        {
            let instances = state.instances.lock().unwrap();
            let instance = instances
                .get(&id)
                .expect("a live current/replacement owner must not be evicted");
            assert_eq!(instance.pid, std::process::id());
            assert_eq!(
                instance.generation,
                if replaced { replacement } else { original_generation }
            );
            drop(instances);
        }
        let (status, inventory) = route_request(router, axum::http::Method::GET, "/vms/list", None).await;
        assert_eq!(status, StatusCode::OK, "{inventory}");
        let row = inventory["sandboxes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == id)
            .unwrap();
        assert_eq!(row["status"], "Running");
        assert!(matches!(
            send_ipc_command(&state, &socket, ServiceToProcess::Ping, Some(2))
                .await
                .unwrap(),
            ProcessToService::Pong
        ));
        assert_eq!(owner.await.unwrap().len(), 2);
        std::fs::remove_file(&socket).unwrap();
        std::fs::remove_file(socket.with_extension("ready")).unwrap();
    }
}
