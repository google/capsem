//! Named CLI administration never retries a failed gateway mutation over UDS.
use super::{validate_id, ServiceAdminClient};
use anyhow::Result;

impl ServiceAdminClient {
    pub(super) async fn preserve_doctor_failure(
        &self,
        id: &str,
        authorization: reqwest::header::HeaderValue,
    ) -> Result<capsem_api::VmActionResponse> {
        validate_id(id)?;
        self.request_local(
            "POST",
            &format!("/vms/{id}/preserve-failure"),
            Some(()),
            Some(authorization),
        )
        .await
    }
}
