use super::*;

#[tokio::test]
async fn opener_is_not_called_until_callback_preparation_succeeds() {
    let context = Context {
        gateway_url: "https://remote.example".into(),
        token: "synthetic".into(),
        vm_id: "vm-1".into(),
    };
    let result = open(
        &Callbacks::default(),
        "https://login.example/?redirect_uri=http%3A%2F%2Flocalhost%3A45000%2Fcallback&state=synthetic-state-1234",
        Some(context),
        || panic!("browser must not open with an unavailable callback"),
    )
    .await;
    assert_eq!(result.unwrap_err(), "could not prepare browser callback");
}

#[tokio::test]
async fn ordinary_links_use_the_existing_opener_without_callback_admission() {
    assert_eq!(
        open(&Callbacks::default(), "https://docs.example/", None, || Err(
            "synthetic opener refusal".into()
        ))
        .await
        .unwrap_err(),
        "synthetic opener refusal"
    );
    let context = Context {
        gateway_url: "http://127.0.0.1:1".into(),
        token: "synthetic".into(),
        vm_id: "vm-1".into(),
    };
    open(&Callbacks::default(), "https://docs.example/", Some(context), || Ok(()))
        .await
        .unwrap();
}
