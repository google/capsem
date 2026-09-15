use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Largest request body and workspace file the HTTP API accepts, in bytes.
///
/// One value for every layer: the gateway refuses larger bodies, the service
/// router's body limit and its file transfer limit use it too. They were three
/// literals, and the service router's unset limit fell back to axum's 2 MiB,
/// refusing uploads the other two allowed.
pub const MAX_REQUEST_BODY_BYTES: usize = 10 * 1024 * 1024;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileEntryType {
    File,
    Directory,
}

/// A single entry in a workspace file listing.
#[derive(Serialize, Debug, Clone, Deserialize, ToSchema)]
pub struct FileListEntry {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub entry_type: FileEntryType,
    pub size: u64,
    pub mtime: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_text: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(no_recursion)]
    pub children: Option<Vec<FileListEntry>>,
}

/// Response for GET /vms/{id}/files/list.
#[derive(Serialize, Debug, Deserialize, Clone, ToSchema)]
pub struct FileListResponse {
    pub entries: Vec<FileListEntry>,
}

/// Response for POST /vms/{id}/files/content.
#[derive(Serialize, Debug, Deserialize, Clone, ToSchema)]
pub struct UploadResponse {
    pub success: bool,
    pub size: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct LogsResponse {
    pub logs: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_logs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_logs: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
}
