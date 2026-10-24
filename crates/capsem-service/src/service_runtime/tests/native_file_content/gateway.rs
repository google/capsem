use super::*;

pub(super) struct Gateway {
    client: reqwest::Client,
    base: String,
    token: String,
}

impl Gateway {
    pub(super) async fn ready(run: &StdPath) -> Self {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !gateway_runtime_ready(
                &run.join("gateway.token"),
                &run.join("gateway.port"),
                &run.join("preview.port"),
            ) {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        let gateway = Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(90))
                .build()
                .unwrap(),
            base: format!(
                "http://127.0.0.1:{}",
                std::fs::read_to_string(run.join("gateway.port")).unwrap().trim()
            ),
            token: std::fs::read_to_string(run.join("gateway.token"))
                .unwrap()
                .trim()
                .into(),
        };
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if gateway
                    .client
                    .get(format!("{}/vms/list", gateway.base))
                    .bearer_auth(&gateway.token)
                    .send()
                    .await
                    .is_ok_and(|response| response.status().is_success())
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        gateway
    }

    pub(super) async fn post(&self, route: &str, body: Value) -> Value {
        let response = self
            .client
            .post(format!("{}{route}", self.base))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body = response.text().await.unwrap();
        assert!(status.is_success(), "{route}: {status}: {body}");
        serde_json::from_str(&body).unwrap()
    }

    async fn list(&self, id: &str) -> Value {
        self.client
            .get(format!("{}/vms/{id}/security/asks", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    pub(super) async fn pending(&self, id: &str, original: &mut ReadTask) -> Value {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let listing = self.list(id).await;
                if original.is_finished() {
                    let summary = match original.await {
                        Ok(result) => read_summary(&result),
                        Err(_) => "original_task_failed".into(),
                    };
                    panic!("original returned before approval: {summary}; {listing}");
                }
                if listing["pending_count"] == 1 {
                    return listing;
                }
                assert_eq!(listing["pending_count"], 0);
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap()
    }

    pub(super) async fn cancelled(&self, id: &str, ask: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let listing = self.list(id).await;
                if listing["pending_count"] == 0 {
                    let row = listing["requests"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|row| row["ask_id"] == ask)
                        .unwrap();
                    assert_eq!(row["status"], "cancelled");
                    return;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
    }

    pub(super) async fn rejected(&self, route: &str, body: &Value, expected: u16, auth: bool) {
        let request = self.client.post(format!("{}{route}", self.base)).json(body);
        let response = if auth {
            request.bearer_auth(&self.token)
        } else {
            request
        }
        .send()
        .await
        .unwrap();
        assert_eq!(
            response.status().as_u16(),
            expected,
            "{route}: {}",
            response.text().await.unwrap()
        );
    }

    pub(super) async fn seed(&self, id: &str) {
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let response = self
                    .client
                    .post(format!("{}/vms/{id}/exec", self.base))
                    .bearer_auth(&self.token)
                    .json(&json!({"command": "true", "target": "vm", "timeout_secs": 5}))
                    .send()
                    .await
                    .unwrap();
                if response.status().is_success() {
                    let value: Value = response.json().await.unwrap();
                    assert_eq!(value["exit_code"], 0, "{value}");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap();
        let seeded = self
            .post(
                &format!("/vms/{id}/exec"),
                json!({
                "command": format!("printf original-private-guest-file-content > {}", guest_file_path()),
                        "target": "vm", "timeout_secs": 5
                    }),
            )
            .await;
        assert_eq!(seeded["exit_code"], 0, "{seeded}");
    }
}

fn read_summary(result: &Result<ProcessToService, crate::vm_files::ipc_command::IpcCommandError>) -> String {
    use crate::vm_files::ipc_command::IpcCommandError;
    match result {
        Ok(ProcessToService::ReadFileResult { data, error, .. }) => {
            let outcome = match error.as_deref() {
                Some(error) if error.starts_with("FileRead rejected:") => "guest_path_refused",
                Some(error) if error.starts_with("file export refused:") => "security_boundary_refused",
                Some(_) => "read_refused",
                None => "read_completed",
            };
            format!("{outcome}; data_bytes={:?}", data.as_ref().map(Vec::len))
        }
        Ok(_) => "unexpected_reply".into(),
        Err(IpcCommandError::Timeout { .. }) => "ipc_timeout".into(),
        Err(IpcCommandError::Failed(_)) => "ipc_failure".into(),
    }
}

#[cfg(test)]
mod tests;
