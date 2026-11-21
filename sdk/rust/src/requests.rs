//! Operator inspection of durable requests and their frozen review inputs.
use crate::{client::Client, models, operations as api, Error, Result};

pub struct Requests<'a>(pub(crate) &'a Client);

impl Requests<'_> {
    pub async fn list(&self, query: models::requests::RequestListQuery) -> Result<models::RequestListResponse> {
        let input = api::ListRequestsParams {
            after: query.after,
            limit: query.limit.map(u32::from),
            pending_only: query.pending_only,
        };
        api::list_requests(&self.0.transport, &input, self.0.options).await
    }

    pub async fn info(&self, id: &str) -> Result<models::RequestDetailResponse> {
        let input = api::GetRequestInfoParams { id: id.into() };
        let response = api::get_request_info(&self.0.transport, &input, self.0.options).await?;
        if response.summary.request_id != id {
            return Err(Error::InvalidResponse("review request identity changed"));
        }
        Ok(response)
    }

    pub async fn resolve(
        &self,
        id: &str,
        request: models::RequestResolveRequest,
    ) -> Result<models::EnvironmentMutationResponse> {
        let input = api::ResolveRequestParams {
            id: id.into(),
            body: request,
        };
        let response = api::resolve_request(&self.0.transport, &input, self.0.options).await?;
        if response.request_id != id {
            return Err(Error::InvalidResponse("resolved request identity changed"));
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{test_gateway::Server, Hypervisor};
    use axum::{body::Body, http::Response};
    use serde_json::json;

    #[tokio::test]
    async fn reviews_preserve_the_observation_and_refuse_substituted_request_ids() {
        const ID: &str = "00000000-0000-4000-8000-000000000001";
        let summary = json!({
            "request_id": ID, "kind": "environment_change", "environment_id": ID,
            "base": null, "source": null, "requester": null, "actor": "operator",
            "reason": "test review", "timestamp_unix_ms": 1, "expires_at_unix_ms": 1000,
            "decision": "pending", "application": "pending", "result": null
        });
        let mut server = Server::respond(move |request| {
            let value = if request.uri.path() == "/requests/list" {
                json!({"requests": [summary], "next_cursor": 9})
            } else if request.method == axum::http::Method::POST {
                json!({"request_id": ID, "environment_id": ID, "decision":"denied",
                    "application":"pending", "result":null, "application_error":null})
            } else {
                json!({
                    "summary": summary, "observed_revision": "0123456789ab", "disposition": null,
                    "base": null, "proposed": {"availability": "unavailable", "purged_variables": []},
                    "targets": []
                })
            };
            Response::new(Body::from(value.to_string()))
        })
        .await;
        let hv = Hypervisor::new(&server.url, "operator-token").unwrap();
        let page = hv
            .requests()
            .list(models::requests::RequestListQuery {
                pending_only: Some(true),
                limit: Some(1),
                after: Some(7),
            })
            .await
            .unwrap();
        assert_eq!(page.next_cursor, Some(9));
        assert_eq!(hv.requests().info(ID).await.unwrap().observed_revision, "0123456789ab");
        assert!(matches!(
            hv.requests().info("other").await,
            Err(Error::InvalidResponse(_))
        ));
        let decision = models::RequestResolveRequest {
            observed_revision: "0123456789ab".into(),
            decision: models::requests::RequestDecision::Deny,
            reason: "keep the original configuration".into(),
        };
        let response = hv.requests().resolve(ID, decision.clone()).await.unwrap();
        assert_eq!(response.decision, models::requests::RequestState::Denied);
        let (list, _) = server.received.recv().await.unwrap();
        assert_eq!(list.uri.query(), Some("after=7&limit=1&pending_only=true"));
        assert_eq!(list.headers["authorization"], "Bearer operator-token");
        server.received.recv().await.unwrap();
        server.received.recv().await.unwrap();
        let (resolve, body) = server.received.recv().await.unwrap();
        assert_eq!(resolve.uri.path(), format!("/requests/{ID}/resolve"));
        assert_eq!(
            serde_json::from_slice::<models::RequestResolveRequest>(&body).unwrap(),
            decision
        );
        assert!(server.received.try_recv().is_err());
    }
}
