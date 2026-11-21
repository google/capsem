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
mod tests;
