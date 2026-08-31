use super::*;

#[tokio::test]
async fn local_http_recorder_captures_request_shape() {
    let recorder = spawn_http_recorder().await.unwrap();
    let response = reqwest::Client::new()
        .post(format!("{}/credential/capture", recorder.base_url))
        .header("Authorization", "Bearer local-secret")
        .body("payload")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let requests = recorder.state.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(requests[0].uri.path(), "/credential/capture");
    assert_eq!(requests[0].header("authorization"), Some("Bearer local-secret"));
    assert_eq!(requests[0].body, b"payload");
}
