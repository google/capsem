use crate::{models, operations as api, Result, VM};

pub struct Copy<'a>(pub(crate) &'a VM);
pub struct Snapshots<'a>(pub(crate) &'a VM);
pub struct Stats<'a>(pub(crate) &'a VM);

impl Copy<'_> {
    pub async fn from_vm(&self, path: &str) -> Result<Vec<u8>> {
        let params = api::DownloadVmFileParams {
            id: self.0.resolve().await?,
            path: path.into(),
        };
        api::download_vm_file(&self.0.client.transport, &params, self.0.client.options).await
    }

    pub async fn to_vm(&self, path: &str, data: Vec<u8>) -> Result<models::UploadResponse> {
        let params = api::UploadVmFileParams {
            id: self.0.resolve().await?,
            path: path.into(),
            body: data,
        };
        api::upload_vm_file(&self.0.client.transport, &params, self.0.client.options).await
    }
}

impl Snapshots<'_> {
    pub async fn list(&self) -> Result<models::SnapshotsList> {
        api::list_vm_snapshots(
            &self.0.client.transport,
            &api::ListVmSnapshotsParams {
                id: self.0.resolve().await?,
            },
            self.0.client.options,
        )
        .await
    }

    pub async fn status(&self) -> Result<models::SnapshotsStatus> {
        api::get_vm_snapshots_status(
            &self.0.client.transport,
            &api::GetVmSnapshotsStatusParams {
                id: self.0.resolve().await?,
            },
            self.0.client.options,
        )
        .await
    }
}

impl Stats<'_> {
    pub async fn summary(&self) -> Result<models::VmStatsSummaryResponse> {
        api::get_vm_stats_summary(
            &self.0.client.transport,
            &api::GetVmStatsSummaryParams {
                id: self.0.resolve().await?,
            },
            self.0.client.options,
        )
        .await
    }

    pub async fn details(&self) -> Result<models::VmStatsDetailResponse> {
        api::get_vm_stats_detail(
            &self.0.client.transport,
            &api::GetVmStatsDetailParams {
                id: self.0.resolve().await?,
            },
            self.0.client.options,
        )
        .await
    }
}
