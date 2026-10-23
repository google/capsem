use super::*;

#[tokio::test]
async fn successful_opener_retries_only_metadata_and_failed_opener_never_acknowledges() {
    use crate::tests::{read_http_request, write_response};
    for opener_succeeds in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let vm = capsem_sdk::VM::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            "synthetic",
            capsem_sdk::VmSelector::Id("vm-1".into()),
        )
        .unwrap();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = calls.clone();
        let request = BrowserRequest {
            owner_generation: Some("18f59e33-8af6-4992-81d9-71e10873af9a".into()),
            id: 7,
            url: "https://example.test/?private=not-in-ack".into(),
        };
        let server = if opener_succeeds {
            Some(tokio::spawn(async move {
                for index in 0..2 {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let incoming = read_http_request(&mut stream).await;
                    let (path, body) = incoming.split_once("\r\n\r\n").unwrap();
                    assert!(path.starts_with("POST /vms/vm-1/browser/acknowledge "));
                    assert!(!body.contains("private"));
                    assert_eq!(
                        serde_json::from_str::<serde_json::Value>(body).unwrap()["request_id"],
                        7
                    );
                    if index == 0 {
                        write_response(&mut stream, "503 Service Unavailable", r#"{"error":"retry"}"#).await;
                    } else {
                        write_response(&mut stream, "200 OK", r#"{"vm_id":"vm-1","owner_generation":"18f59e33-8af6-4992-81d9-71e10873af9a","request_id":7}"#).await;
                    }
                }
            }))
        } else {
            None
        };
        let result = open_with(&request, &vm, &Callbacks::default(), move |_| async move {
            count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            anyhow::ensure!(opener_succeeds, "synthetic opener failure");
            Ok(())
        })
        .await;
        assert_eq!(result.is_ok(), opener_succeeds);
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        if let Some(server) = server {
            server.await.unwrap();
        }
    }
}

#[test]
fn native_opener_receives_one_validated_argument_without_shell_interpolation() {
    let request = BrowserRequest {
        owner_generation: Some("18f59e33-8af6-4992-81d9-71e10873af9a".into()),
        id: 1,
        url: "https://example.org/?state=$(touch%20/tmp/no)&x=1".into(),
    };
    let command = command(&request).unwrap();
    assert_eq!(
        command.get_program(),
        if cfg!(target_os = "macos") { "open" } else { "xdg-open" }
    );
    assert_eq!(command.get_args().collect::<Vec<_>>(), [request.url.as_str()]);
    assert!(super::command(&BrowserRequest {
        owner_generation: Some("18f59e33-8af6-4992-81d9-71e10873af9a".into()),
        id: 2,
        url: "file:///etc/passwd".into()
    })
    .is_err());
}

#[tokio::test]
async fn browser_callback_failure_prevents_launching_the_opener() {
    let vm = capsem_sdk::VM::new(
        "https://remote.example",
        "synthetic",
        capsem_sdk::VmSelector::Id("vm-1".into()),
    )
    .unwrap();
    let request = BrowserRequest {
        owner_generation: Some("18f59e33-8af6-4992-81d9-71e10873af9a".into()),
        id: 1,
        url: "https://login.example/?redirect_uri=http%3A%2F%2Flocalhost%3A45000%2Fcallback&state=synthetic-state-1234"
            .into(),
    };
    let callbacks = Callbacks::default();
    assert!(open(&request, &vm, &callbacks)
        .await
        .unwrap_err()
        .to_string()
        .contains("callback"));
}
