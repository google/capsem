use crate::{models, operations as api, HistoryOptions, LogOptions, PageOptions, Result, TimelineOptions, VM};

impl VM {
    pub async fn log(&self, options: LogOptions) -> Result<models::LogsResponse> {
        let params = api::GetVmLogsParams {
            id: self.resolve().await?,
            grep: options.grep,
            tail: options.tail,
            max_bytes: options.max_bytes,
        };
        api::get_vm_logs(&self.client.transport, &params, self.client.options).await
    }

    pub async fn history(&self, options: HistoryOptions) -> Result<models::HistoryResponse> {
        let params = api::GetVmHistoryParams {
            id: self.resolve().await?,
            limit: options.limit,
            offset: options.offset,
            search: options.search,
            layer: options.layer,
        };
        api::get_vm_history(&self.client.transport, &params, self.client.options).await
    }

    pub async fn list(&self, path: &str, depth: Option<i64>) -> Result<models::FileListResponse> {
        let params = api::ListVmFilesParams {
            id: self.resolve().await?,
            path: (path != "/").then(|| path.to_owned()),
            depth,
        };
        api::list_vm_files(&self.client.transport, &params, self.client.options).await
    }

    pub async fn changes(&self, checkpoint: &str, options: PageOptions) -> Result<models::ChangesResponse> {
        let params = api::GetVmChangesParams {
            id: self.resolve().await?,
            checkpoint: checkpoint.into(),
            limit: options.limit,
            offset: options.offset,
        };
        api::get_vm_changes(&self.client.transport, &params, self.client.options).await
    }

    pub async fn timeline(&self, options: TimelineOptions) -> Result<models::TimelineResponse> {
        let params = api::GetVmTimelineParams {
            id: self.resolve().await?,
            trace_id: options.trace_id,
            since: options.since,
            limit: options.limit,
            layers: options.layers,
        };
        api::get_vm_timeline(&self.client.transport, &params, self.client.options).await
    }
}
