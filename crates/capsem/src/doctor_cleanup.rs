use crate::client::{ApiResponse, GatewayClient};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DoctorSessionCleanup {
    Completed,
    Failed,
    Interrupted,
}

impl DoctorSessionCleanup {
    pub(crate) fn route(self, vm_id: &str) -> String {
        let action = match self {
            Self::Failed => "preserve-failure",
            Self::Completed | Self::Interrupted => "delete",
        };
        format!("/vms/{vm_id}/{action}")
    }
}

pub(crate) async fn cleanup_doctor_vm(client: &GatewayClient, vm_id: &str, cleanup: DoctorSessionCleanup) {
    let route = cleanup.route(vm_id);
    let result: Result<capsem_api::VmActionResponse, _> = match cleanup {
        DoctorSessionCleanup::Failed => client.preserve_doctor_failure(vm_id).await,
        DoctorSessionCleanup::Completed | DoctorSessionCleanup::Interrupted => client
            .delete::<ApiResponse<capsem_api::VmActionResponse>>(&route)
            .await
            .and_then(ApiResponse::into_result),
    };
    if let Err(error) = result.and_then(|response| {
        anyhow::ensure!(response.success, "doctor cleanup was not acknowledged");
        Ok(())
    }) {
        eprintln!("warning: doctor session cleanup failed: {error:#}");
    }
}
