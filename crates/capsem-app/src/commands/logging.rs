use serde::Serialize;

/// Info about a session that has a capsem.log file.
#[derive(Serialize)]
pub struct LogSessionInfo {
    pub session_id: String,
    pub entry_count: usize,
}

/// Load a session's capsem.log file as parsed log entries.
#[tauri::command]
pub async fn load_session_log(session_id: String) -> Result<Vec<capsem_core::log_layer::LogEvent>, String> {
    tokio::task::spawn_blocking(move || {
        let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
        let path = std::path::PathBuf::from(home)
            .join(".capsem")
            .join("sessions")
            .join(&session_id)
            .join("capsem.log");

        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;

        let mut entries = Vec::new();
        for line in content.lines() {
            if line.is_empty() {
                continue;
            }
            if let Ok(event) = serde_json::from_str::<capsem_core::log_layer::LogEvent>(line) {
                entries.push(event);
            }
        }
        Ok(entries)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

/// List sessions that have a capsem.log file.
#[tauri::command]
pub async fn list_log_sessions() -> Result<Vec<LogSessionInfo>, String> {
    tokio::task::spawn_blocking(|| {
        let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
        let sessions_dir = std::path::PathBuf::from(home)
            .join(".capsem")
            .join("sessions");

        let mut sessions = Vec::new();
        let entries = std::fs::read_dir(&sessions_dir)
            .map_err(|e| format!("failed to read sessions dir: {e}"))?;

        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let log_path = dir.join("capsem.log");
            if log_path.exists() {
                let session_id = dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();

                // Count lines for entry_count
                let entry_count = std::fs::read_to_string(&log_path)
                    .map(|c| c.lines().filter(|l| !l.is_empty()).count())
                    .unwrap_or(0);

                sessions.push(LogSessionInfo {
                    session_id,
                    entry_count,
                });
            }
        }
        Ok(sessions)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_session_info_serializes() {
        let info = LogSessionInfo {
            session_id: "20260317-100530-a1b2".into(),
            entry_count: 42,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"session_id\":\"20260317-100530-a1b2\""));
        assert!(json.contains("\"entry_count\":42"));
    }

    #[test]
    fn log_event_parsed_from_jsonl() {
        let jsonl = r#"{"timestamp":"2026-03-17T10:05:32.000Z","level":"INFO","target":"capsem::vm::boot","message":"kernel loaded"}"#;
        let event: capsem_core::log_layer::LogEvent = serde_json::from_str(jsonl).unwrap();
        assert_eq!(event.level, "INFO");
        assert_eq!(event.message, "kernel loaded");
    }

    #[test]
    fn log_event_malformed_line_skipped() {
        let content = "not json\n{\"timestamp\":\"t\",\"level\":\"INFO\",\"target\":\"t\",\"message\":\"ok\"}\n";
        let mut entries = Vec::new();
        for line in content.lines() {
            if let Ok(event) = serde_json::from_str::<capsem_core::log_layer::LogEvent>(line) {
                entries.push(event);
            }
        }
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "ok");
    }
}
