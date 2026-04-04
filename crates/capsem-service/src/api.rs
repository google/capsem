use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct ProvisionRequest {
    pub name: Option<String>,
    pub ram_mb: u64,
    pub cpus: u32,
    #[serde(default)]
    pub auto_remove: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ProvisionResponse {
    pub id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SandboxInfo {
    pub id: String,
    pub pid: u32,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ram_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpus: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ListResponse {
    pub sandboxes: Vec<SandboxInfo>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExecRequest {
    pub command: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_timeout() -> u64 { 30 }

#[derive(Serialize, Deserialize, Debug)]
pub struct ExecResponse {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct WriteFileRequest {
    pub path: String,
    pub content: String, // Base64 or plain text? For now let's assume plain text or base64 if we detect it.
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ReadFileRequest {
    pub path: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ReadFileResponse {
    pub content: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LogsResponse {
    pub logs: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_logs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_logs: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct InspectRequest {
    pub sql: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[allow(dead_code)]
pub struct InspectResponse {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -----------------------------------------------------------------------
    // ProvisionRequest / ProvisionResponse
    // -----------------------------------------------------------------------

    #[test]
    fn provision_request_with_name() {
        let json = json!({"name": "my-vm", "ram_mb": 4096, "cpus": 4});
        let r: ProvisionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.name, Some("my-vm".into()));
        assert_eq!(r.ram_mb, 4096);
        assert_eq!(r.cpus, 4);
    }

    #[test]
    fn provision_request_without_name() {
        let json = json!({"ram_mb": 2048, "cpus": 2});
        let r: ProvisionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.name, None);
    }

    #[test]
    fn provision_response_roundtrip() {
        let r = ProvisionResponse { id: "vm-123".into() };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ProvisionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.id, "vm-123");
    }

    // -----------------------------------------------------------------------
    // ListResponse
    // -----------------------------------------------------------------------

    #[test]
    fn list_response_empty() {
        let r = ListResponse { sandboxes: vec![] };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ListResponse = serde_json::from_str(&json).unwrap();
        assert!(r2.sandboxes.is_empty());
    }

    #[test]
    fn list_response_multiple() {
        let r = ListResponse {
            sandboxes: vec![
                SandboxInfo { id: "a".into(), pid: 100, status: "Running".into(), ram_mb: Some(2048), cpus: Some(2), version: None },
                SandboxInfo { id: "b".into(), pid: 200, status: "Running".into(), ram_mb: None, cpus: None, version: None },
            ],
        };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ListResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.sandboxes.len(), 2);
        assert_eq!(r2.sandboxes[0].id, "a");
        assert_eq!(r2.sandboxes[1].id, "b");
    }

    #[test]
    fn sandbox_info_optional_fields_omitted() {
        let s = SandboxInfo { id: "x".into(), pid: 1, status: "Running".into(), ram_mb: None, cpus: None, version: None };
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("ram_mb"));
        assert!(!json.contains("cpus"));
    }

    // -----------------------------------------------------------------------
    // ExecRequest / ExecResponse
    // -----------------------------------------------------------------------

    #[test]
    fn exec_request_default_timeout() {
        let json = json!({"command": "echo hi"});
        let r: ExecRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.command, "echo hi");
        assert_eq!(r.timeout_secs, 30);
    }

    #[test]
    fn exec_request_custom_timeout() {
        let json = json!({"command": "sleep 10", "timeout_secs": 5});
        let r: ExecRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.timeout_secs, 5);
    }

    #[test]
    fn exec_response_roundtrip() {
        let r = ExecResponse { stdout: "hello\n".into(), stderr: "".into(), exit_code: 0 };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ExecResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.stdout, "hello\n");
        assert_eq!(r2.exit_code, 0);
    }

    // -----------------------------------------------------------------------
    // File I/O
    // -----------------------------------------------------------------------

    #[test]
    fn write_file_request_roundtrip() {
        let json = json!({"path": "/tmp/f.txt", "content": "data"});
        let r: WriteFileRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.path, "/tmp/f.txt");
        assert_eq!(r.content, "data");
    }

    #[test]
    fn read_file_response_roundtrip() {
        let r = ReadFileResponse { content: "file contents".into() };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ReadFileResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.content, "file contents");
    }

    // -----------------------------------------------------------------------
    // Inspect
    // -----------------------------------------------------------------------

    #[test]
    fn inspect_request_roundtrip() {
        let json = json!({"sql": "SELECT count(*) FROM net_events"});
        let r: InspectRequest = serde_json::from_value(json).unwrap();
        assert_eq!(r.sql, "SELECT count(*) FROM net_events");
    }

    #[test]
    fn inspect_response_roundtrip() {
        let r = InspectResponse {
            columns: vec!["name".into(), "count".into()],
            rows: vec![vec![json!("net_events"), json!(42)]],
        };
        let json = serde_json::to_string(&r).unwrap();
        let r2: InspectResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(r2.columns.len(), 2);
        assert_eq!(r2.rows[0][1], json!(42));
    }

    // -----------------------------------------------------------------------
    // Logs / Error
    // -----------------------------------------------------------------------

    #[test]
    fn logs_response_roundtrip() {
        let r = LogsResponse { logs: "Linux boot...\n".into() };
        let json = serde_json::to_string(&r).unwrap();
        let r2: LogsResponse = serde_json::from_str(&json).unwrap();
        assert!(r2.logs.contains("Linux"));
    }

    #[test]
    fn error_response_roundtrip() {
        let r = ErrorResponse { error: "sandbox not found".into() };
        let json = serde_json::to_string(&r).unwrap();
        let r2: ErrorResponse = serde_json::from_str(&json).unwrap();
        assert!(r2.error.contains("not found"));
    }
}
