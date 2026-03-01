pub mod builtin_tools;
pub mod gateway;
pub mod policy;
pub mod server_manager;
pub mod stdio_bridge;
pub mod types;

use std::collections::HashMap;
use std::path::Path;

use tracing::debug;

use crate::mcp::types::McpServerDef;

/// Read MCP server definitions from the user's existing AI CLI configs.
/// Scans ~/.claude/settings.json and ~/.gemini/settings.json for mcpServers.
pub fn detect_host_mcp_servers() -> Vec<McpServerDef> {
    let home = match dirs_home() {
        Some(h) => h,
        None => return Vec::new(),
    };

    let mut servers = Vec::new();

    // Claude Code: ~/.claude/settings.json
    let claude_path = home.join(".claude").join("settings.json");
    if let Some(mut defs) = parse_mcp_servers_from_file(&claude_path, "claude") {
        servers.append(&mut defs);
    }

    // Gemini CLI: ~/.gemini/settings.json
    let gemini_path = home.join(".gemini").join("settings.json");
    if let Some(mut defs) = parse_mcp_servers_from_file(&gemini_path, "gemini") {
        servers.append(&mut defs);
    }

    // Deduplicate by name (first occurrence wins)
    let mut seen = std::collections::HashSet::new();
    servers.retain(|s| seen.insert(s.name.clone()));

    debug!(count = servers.len(), "auto-detected MCP servers");
    servers
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

/// Parse mcpServers from a settings.json file.
/// Returns None if the file doesn't exist or can't be parsed.
fn parse_mcp_servers_from_file(path: &Path, source: &str) -> Option<Vec<McpServerDef>> {
    let content = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;

    let servers_obj = json.get("mcpServers")?.as_object()?;
    let mut defs = Vec::new();

    for (name, config) in servers_obj {
        // Skip the capsem server itself (we inject that)
        if name == "capsem" {
            continue;
        }

        let command = match config.get("command").and_then(|v| v.as_str()) {
            Some(c) => c.to_string(),
            None => continue, // Not an stdio server
        };

        let args: Vec<String> = config
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let env: HashMap<String, String> = config
            .get("env")
            .and_then(|v| v.as_object())
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        debug!(name, source, "detected MCP server");
        defs.push(McpServerDef {
            name: name.clone(),
            command,
            args,
            env,
            enabled: true,
            source: source.to_string(),
        });
    }

    Some(defs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parse_claude_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(
            f,
            r#"{{
            "mcpServers": {{
                "github": {{
                    "command": "npx",
                    "args": ["-y", "@github/mcp-server"],
                    "env": {{"GITHUB_TOKEN": "ghp_secret"}}
                }},
                "capsem": {{
                    "command": "/run/capsem-mcp-server"
                }}
            }}
        }}"#
        )
        .unwrap();

        let defs = parse_mcp_servers_from_file(&path, "claude").unwrap();
        assert_eq!(defs.len(), 1); // capsem filtered out
        assert_eq!(defs[0].name, "github");
        assert_eq!(defs[0].command, "npx");
        assert_eq!(defs[0].args, vec!["-y", "@github/mcp-server"]);
        assert_eq!(defs[0].env.get("GITHUB_TOKEN").unwrap(), "ghp_secret");
        assert_eq!(defs[0].source, "claude");
    }

    #[test]
    fn parse_missing_file_returns_none() {
        let result = parse_mcp_servers_from_file(Path::new("/nonexistent/settings.json"), "test");
        assert!(result.is_none());
    }

    #[test]
    fn parse_no_mcp_servers_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"other": "stuff"}"#).unwrap();
        let result = parse_mcp_servers_from_file(&path, "test");
        assert!(result.is_none());
    }

    #[test]
    fn parse_server_without_command_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"mcpServers": {"bad": {"url": "http://localhost"}}}"#,
        )
        .unwrap();
        let defs = parse_mcp_servers_from_file(&path, "test").unwrap();
        assert_eq!(defs.len(), 0);
    }
}
