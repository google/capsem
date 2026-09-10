use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A single entry in a workspace file listing.
#[derive(Serialize, Debug, Clone, Deserialize, ToSchema)]
pub struct FileListEntry {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub entry_type: String,
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
pub struct ReadFileRequest {
    pub path: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct ReadFileResponse {
    pub content: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct WriteFileRequest {
    pub path: String,
    pub content: String, // Base64 or plain text? For now let's assume plain text or base64 if we detect it.
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
