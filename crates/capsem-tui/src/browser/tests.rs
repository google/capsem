use super::*;

#[test]
fn native_opener_receives_one_validated_argument_without_shell_interpolation() {
    let request = BrowserRequest {
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
