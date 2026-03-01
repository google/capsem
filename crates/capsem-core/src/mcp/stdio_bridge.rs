//! NDJSON framing over child stdin/stdout for MCP stdio transport.
//!
//! Host-side communication with real MCP server processes uses newline-delimited
//! JSON (one JSON-RPC message per line).

use std::io;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};

use super::types::{JsonRpcRequest, JsonRpcResponse};

/// Write a JSON-RPC request as a single NDJSON line to the child's stdin.
pub async fn write_request(stdin: &mut ChildStdin, req: &JsonRpcRequest) -> io::Result<()> {
    let mut line = serde_json::to_vec(req)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    line.push(b'\n');
    stdin.write_all(&line).await?;
    stdin.flush().await?;
    Ok(())
}

/// Read a single JSON-RPC response line from the child's stdout.
/// Returns None on EOF.
pub async fn read_response(
    reader: &mut BufReader<ChildStdout>,
) -> io::Result<Option<JsonRpcResponse>> {
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Ok(None); // EOF
    }
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let resp: JsonRpcResponse = serde_json::from_str(trimmed)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(resp))
}

/// Read a single JSON-RPC request line (for bidirectional MCP servers that
/// send requests to the host, e.g. sampling).
pub async fn read_request(
    reader: &mut BufReader<ChildStdout>,
) -> io::Result<Option<JsonRpcRequest>> {
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Ok(None);
    }
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let req: JsonRpcRequest = serde_json::from_str(trimmed)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(req))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::process::Command;

    #[tokio::test]
    async fn roundtrip_via_cat() {
        // Use `cat` as an echo server for NDJSON lines.
        let mut child = Command::new("cat")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("failed to spawn cat");

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);

        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(serde_json::json!(42)),
            method: "tools/list".into(),
            params: None,
        };

        write_request(&mut stdin, &req).await.unwrap();
        // cat echoes the line back, but as a "response" shape
        // We read it as raw JSON -- it won't have result/error fields
        // but serde with skip_serializing_if will handle missing fields
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert!(line.contains("tools/list"));
        assert!(line.contains("42"));

        // Close stdin to make cat exit
        drop(stdin);
        child.wait().await.unwrap();
    }

    #[tokio::test]
    async fn eof_returns_none() {
        let mut child = Command::new("true")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("failed to spawn true");

        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);

        child.wait().await.unwrap();
        let result = read_response(&mut reader).await.unwrap();
        assert!(result.is_none());
    }
}
