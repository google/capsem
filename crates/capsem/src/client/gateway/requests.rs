use super::{gateway_error, CallOptions, GatewayClient};
use anyhow::Result;
use capsem_api::{
    requests::{RequestDetailResponse, RequestListResponse, RequestResolveRequest},
    validate_managed_request_id, EnvironmentMutationResponse,
};
use capsem_sdk::operations as api;
use std::time::Duration;

const READ_OPTIONS: CallOptions = CallOptions {
    timeout: Some(Duration::from_secs(30)),
};

impl GatewayClient {
    pub(crate) async fn list_requests(&self, input: &api::ListRequestsParams) -> Result<RequestListResponse> {
        api::list_requests(self.transport().await?, input, READ_OPTIONS)
            .await
            .map_err(gateway_error)
    }

    pub(crate) async fn get_request_info(&self, id: &str) -> Result<RequestDetailResponse> {
        validate_managed_request_id(id).map_err(anyhow::Error::msg)?;
        let input = api::GetRequestInfoParams { id: id.into() };
        let response = api::get_request_info(self.transport().await?, &input, READ_OPTIONS)
            .await
            .map_err(gateway_error)?;
        anyhow::ensure!(response.summary.request_id == id, "review request identity changed");
        Ok(response)
    }

    pub(crate) async fn resolve_request(
        &self,
        id: &str,
        body: RequestResolveRequest,
    ) -> Result<EnvironmentMutationResponse> {
        validate_managed_request_id(id).map_err(anyhow::Error::msg)?;
        body.validate().map_err(anyhow::Error::msg)?;
        let input = api::ResolveRequestParams { id: id.into(), body };
        let response = api::resolve_request(self.transport().await?, &input, CallOptions::default())
            .await
            .map_err(gateway_error)?;
        anyhow::ensure!(response.request_id == id, "resolved request identity changed");
        Ok(response)
    }
}
