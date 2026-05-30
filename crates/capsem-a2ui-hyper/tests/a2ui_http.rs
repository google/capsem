use capsem_a2ui_hyper::{serve, A2uiRenderResponse};
use capsem_ui_catalog::ui::validate_messages;
use serde_json::json;
use tokio::{net::TcpListener, sync::oneshot};

#[tokio::test]
async fn hyper_render_outputs_conformant_warning_alert() {
    let server = TestServer::start().await;
    let response = server
        .post(json!({
            "calls": [
                { "tool": "ui.surface.create", "args": { "id": "agent-draft" } },
                {
                    "tool": "ui.alert",
                    "args": {
                        "surfaceId": "agent-draft",
                        "id": "base-warning",
                        "message": "all your base beling to us",
                        "variant": "soft",
                        "tone": "warning"
                    }
                },
                { "tool": "ui.surface.validate", "args": { "surfaceId": "agent-draft" } },
                { "tool": "ui.surface.preview", "args": { "surfaceId": "agent-draft" } }
            ]
        }))
        .await;

    assert!(response.ok, "{response:#?}");
    assert!(response.conformance.ok, "{response:#?}");
    let surface = response.surfaces.first().expect("surface exists");
    validate_messages(&surface.messages).expect("A2UI messages conform");
    let recipe = surface.recipe.as_ref().expect("recipe exists");
    assert_eq!(recipe.component, "alert");
    assert_eq!(recipe.variant, "soft");
    assert_eq!(recipe.tone.as_deref(), Some("warning"));
}

#[tokio::test]
async fn hyper_render_preserves_card_link_contract() {
    let server = TestServer::start().await;
    let response = server
        .post(json!({
            "calls": [
                { "tool": "ui.surface.create", "args": { "id": "agent-draft" } },
                {
                    "tool": "ui.card",
                    "args": {
                        "surfaceId": "agent-draft",
                        "id": "model-card",
                        "title": "model",
                        "description": "Gemini",
                        "linkLabel": "visit homepage",
                        "linkHref": "https://gemini.google.com/"
                    }
                },
                { "tool": "ui.surface.validate", "args": { "surfaceId": "agent-draft" } },
                { "tool": "ui.surface.preview", "args": { "surfaceId": "agent-draft" } }
            ]
        }))
        .await;

    assert!(response.ok, "{response:#?}");
    let surface = response.surfaces.first().expect("surface exists");
    validate_messages(&surface.messages).expect("A2UI messages conform");
    let recipe = surface.recipe.as_ref().expect("recipe exists");
    let link = recipe.link.as_ref().expect("link survives contract");
    assert_eq!(recipe.component, "card");
    assert_eq!(link.label, "visit homepage");
    assert_eq!(link.href, "https://gemini.google.com/");
}

#[tokio::test]
async fn hyper_render_outputs_conformant_table() {
    let server = TestServer::start().await;
    let response = server
        .post(json!({
            "calls": [
                { "tool": "ui.surface.create", "args": { "id": "agent-draft" } },
                {
                    "tool": "ui.table",
                    "args": {
                        "surfaceId": "agent-draft",
                        "id": "houses",
                        "title": "Great Houses of Westeros",
                        "columns": ["House", "Motto", "Arms"],
                        "rows": [
                            ["Stark", "Winter Is Coming", "Direwolf"],
                            ["Lannister", "Hear Me Roar!", "Golden lion"],
                            ["Targaryen", "Fire and Blood", "Three-headed dragon"]
                        ]
                    }
                },
                { "tool": "ui.surface.validate", "args": { "surfaceId": "agent-draft" } },
                { "tool": "ui.surface.preview", "args": { "surfaceId": "agent-draft" } }
            ]
        }))
        .await;

    assert!(response.ok, "{response:#?}");
    assert!(response.conformance.ok, "{response:#?}");
    let surface = response.surfaces.first().expect("surface exists");
    validate_messages(&surface.messages).expect("A2UI messages conform");
    let recipe = surface.recipe.as_ref().expect("recipe exists");
    assert_eq!(recipe.component, "table");
    assert_eq!(recipe.variant, "basic");
    let serialized = serde_json::to_string(&surface.messages).unwrap();
    assert!(serialized.contains("Winter Is Coming"));
    assert!(serialized.contains("Three-headed dragon"));
}

#[tokio::test]
async fn hyper_render_rejects_bad_paths_and_bad_json() {
    let server = TestServer::start().await;
    let missing = reqwest::get(format!("{}/missing", server.base_url))
        .await
        .expect("request succeeds");
    assert_eq!(missing.status(), 404);

    let bad_json = reqwest::Client::new()
        .post(format!("{}/a2ui/render", server.base_url))
        .header("content-type", "application/json")
        .body("{")
        .send()
        .await
        .expect("request succeeds");
    assert_eq!(bad_json.status(), 400);
}

struct TestServer {
    base_url: String,
    shutdown: Option<oneshot::Sender<()>>,
}

impl TestServer {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("test listener addr");
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        tokio::spawn(async move {
            serve(listener, shutdown_rx)
                .await
                .expect("hyper server exits");
        });
        Self {
            base_url: format!("http://{addr}"),
            shutdown: Some(shutdown_tx),
        }
    }

    async fn post(&self, body: serde_json::Value) -> A2uiRenderResponse {
        let response = reqwest::Client::new()
            .post(format!("{}/a2ui/render", self.base_url))
            .json(&body)
            .send()
            .await
            .expect("request succeeds");
        assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
        response.json().await.expect("response parses")
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}
