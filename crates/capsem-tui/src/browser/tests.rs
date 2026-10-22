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
