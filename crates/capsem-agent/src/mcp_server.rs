// capsem-mcp-server: Guest-side MCP stdio-to-framed-vsock relay.
//
// Bridges JSON-RPC lines from an AI agent's MCP client (stdin/stdout) to the
// MITM MCP endpoint over vsock:5002. The host owns parsing, policy, routing,
// telemetry, and tool execution. This binary only frames requests, carries
// per-frame process attribution, writes responses back to stdout, and
// reconnects after transport loss.

#[path = "vsock_io.rs"]
mod vsock_io;

#[path = "procfs.rs"]
mod procfs;

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::os::unix::io::RawFd;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use capsem_proto::{
    MCP_FRAME_FLAG_NOTIFICATION, MCP_FRAME_HEADER_LEN, MCP_FRAME_MAX_SIZE, VSOCK_PORT_SNI_PROXY,
};
use serde_json::Value;
use vsock_io::{read_exact_fd, vsock_connect_retry, write_all_fd, VSOCK_HOST_CID};

const MCP_TRANSPORT_PORT: u32 = VSOCK_PORT_SNI_PROXY;

#[derive(Clone)]
struct PendingRequests {
    inner: Arc<Mutex<HashMap<u32, PendingRequest>>>,
}

#[derive(Clone, Debug, PartialEq)]
struct PendingRequest {
    json_id: Value,
    method: Option<String>,
}

impl PendingRequests {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn insert(&self, stream_id: u32, request: PendingRequest) {
        self.inner
            .lock()
            .expect("pending MCP requests mutex poisoned")
            .insert(stream_id, request);
    }

    fn remove(&self, stream_id: u32) {
        self.inner
            .lock()
            .expect("pending MCP requests mutex poisoned")
            .remove(&stream_id);
    }

    fn take_all(&self) -> Vec<PendingRequest> {
        self.inner
            .lock()
            .expect("pending MCP requests mutex poisoned")
            .drain()
            .map(|(_, request)| request)
            .collect()
    }
}

struct FramedConnection {
    fd: RawFd,
    alive: Arc<AtomicBool>,
    reader_handle: thread::JoinHandle<()>,
}

#[derive(Debug, PartialEq)]
enum JsonRpcLineKind {
    Request {
        json_id: Option<Value>,
        method: Option<String>,
    },
    Notification,
}

/// Get the parent process name (the AI agent that spawned us).
fn get_parent_process_name() -> String {
    let ppid = nix::unistd::getppid();
    let raw = procfs::process_name_for_pid(ppid.as_raw() as u32);
    sanitize_process_name(&raw)
}

/// Sanitize a process name for use in framed MCP attribution.
/// Replaces control characters (including newlines and NUL) and spaces with
/// underscores, and truncates to 128 chars to match the frame envelope.
fn sanitize_process_name(name: &str) -> String {
    let mut s = name
        .chars()
        .map(|c| if c.is_control() || c == ' ' { '_' } else { c })
        .collect::<String>();
    if s.len() > 128 {
        s.truncate(128);
    }
    s
}

fn main() {
    eprintln!(
        "[capsem-mcp-server] starting framed relay (pid {})",
        process::id()
    );

    let process_name = get_parent_process_name();
    let pending = PendingRequests::new();
    let stdout = Arc::new(Mutex::new(io::stdout()));
    let mut conn = connect_framed(&process_name, pending.clone(), Arc::clone(&stdout));
    let mut next_stream_id = 1u32;

    let stdin = io::stdin();
    let reader = stdin.lock();

    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => break,
        };

        if !conn.alive.load(Ordering::SeqCst) {
            close_broken_connection(conn);
            conn = connect_framed(&process_name, pending.clone(), Arc::clone(&stdout));
            next_stream_id = 1;
        }

        let kind = classify_jsonrpc_line(&line);
        let (stream_id, flags, pending_request) = match kind {
            JsonRpcLineKind::Notification => (0, MCP_FRAME_FLAG_NOTIFICATION, None),
            JsonRpcLineKind::Request { json_id, method } => {
                let id = next_stream_id;
                if id == u32::MAX {
                    eprintln!(
                        "[capsem-mcp-server] framed stream id exhausted; reconnecting before next request"
                    );
                    close_broken_connection(conn);
                    conn = connect_framed(&process_name, pending.clone(), Arc::clone(&stdout));
                    next_stream_id = 1;
                }
                let id = next_stream_id;
                next_stream_id += 1;
                (
                    id,
                    0,
                    json_id.map(|json_id| PendingRequest { json_id, method }),
                )
            }
        };

        let frame = match capsem_proto::encode_mcp_frame(
            stream_id,
            flags,
            &process_name,
            line.as_bytes(),
        ) {
            Ok(frame) => frame,
            Err(e) => {
                eprintln!("[capsem-mcp-server] failed to encode MCP frame: {e:#}");
                if let Some(request) = pending_request {
                    emit_single_disconnect_error(&stdout, request, "frame encode failed");
                }
                continue;
            }
        };

        if let Some(request) = pending_request {
            pending.insert(stream_id, request);
        }

        if write_all_fd(conn.fd, &frame).is_err() {
            conn.alive.store(false, Ordering::SeqCst);
            emit_disconnect_errors(&pending, &stdout, "write failed");
            close_broken_connection(conn);
            conn = connect_framed(&process_name, pending.clone(), Arc::clone(&stdout));
            next_stream_id = 1;
        }
    }

    finish_connection(conn);
}

fn connect_framed(
    process_name: &str,
    pending: PendingRequests,
    stdout: Arc<Mutex<io::Stdout>>,
) -> FramedConnection {
    let fd = vsock_connect_retry(VSOCK_HOST_CID, MCP_TRANSPORT_PORT, "mcp-framed");

    // Keep the established diagnostic metadata prefix so host logs can still
    // attribute the connection. The framed envelope carries authoritative
    // per-request process attribution.
    let meta = format!("\0CAPSEM_META:{}\n", process_name);
    if let Err(e) = write_all_fd(fd, meta.as_bytes()) {
        eprintln!("[capsem-mcp-server] failed to send framed metadata: {e}");
        process::exit(1);
    }

    let alive = Arc::new(AtomicBool::new(true));
    let alive_reader = Arc::clone(&alive);
    let reader_handle = thread::spawn(move || {
        framed_vsock_to_stdout(fd, pending, stdout, alive_reader);
    });

    FramedConnection {
        fd,
        alive,
        reader_handle,
    }
}

fn close_broken_connection(conn: FramedConnection) {
    unsafe {
        nix::libc::shutdown(conn.fd, nix::libc::SHUT_RDWR);
        nix::libc::close(conn.fd);
    }
    let _ = conn.reader_handle.join();
}

fn finish_connection(conn: FramedConnection) {
    unsafe {
        nix::libc::shutdown(conn.fd, nix::libc::SHUT_WR);
    }
    let _ = conn.reader_handle.join();
    unsafe {
        nix::libc::close(conn.fd);
    }
}

fn framed_vsock_to_stdout(
    fd: RawFd,
    pending: PendingRequests,
    stdout: Arc<Mutex<io::Stdout>>,
    alive: Arc<AtomicBool>,
) {
    let dup_fd = unsafe { nix::libc::dup(fd) };

    loop {
        let mut len_buf = [0u8; 4];
        if read_exact_fd(dup_fd, &mut len_buf).is_err() {
            break;
        }
        let total_len = u32::from_be_bytes(len_buf) as usize;
        if !(MCP_FRAME_HEADER_LEN as usize..=MCP_FRAME_MAX_SIZE).contains(&total_len) {
            eprintln!("[capsem-mcp-server] invalid MCP frame length from host: {total_len}");
            break;
        }

        let mut body = vec![0u8; total_len];
        if read_exact_fd(dup_fd, &mut body).is_err() {
            break;
        }
        let frame = match capsem_proto::decode_mcp_frame_body(&body) {
            Ok(frame) => frame,
            Err(e) => {
                eprintln!("[capsem-mcp-server] invalid MCP frame from host: {e:#}");
                break;
            }
        };
        if frame.payload.is_empty() {
            pending.remove(frame.stream_id);
            continue;
        }

        pending.remove(frame.stream_id);
        let mut out = stdout.lock().expect("stdout mutex poisoned");
        if out.write_all(&frame.payload).is_err() {
            break;
        }
        if !frame.payload.ends_with(b"\n") && writeln!(out).is_err() {
            break;
        }
        if out.flush().is_err() {
            break;
        }
    }

    alive.store(false, Ordering::SeqCst);
    // If the transport drops while requests are outstanding, report a
    // terminal JSON-RPC error and reconnect for later requests. We do not
    // replay `tools/call` automatically: after host dispatch, an external
    // tool may have already performed a non-idempotent side effect, and the
    // relay cannot know whether retrying would duplicate it.
    emit_disconnect_errors(&pending, &stdout, "connection closed");

    unsafe {
        nix::libc::close(dup_fd);
    }
}

fn classify_jsonrpc_line(line: &str) -> JsonRpcLineKind {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return JsonRpcLineKind::Request {
            json_id: None,
            method: None,
        };
    };
    let Some(object) = value.as_object() else {
        return JsonRpcLineKind::Request {
            json_id: None,
            method: None,
        };
    };
    let method = object
        .get("method")
        .and_then(|method| method.as_str())
        .map(str::to_string);
    match object.get("id") {
        Some(json_id) => JsonRpcLineKind::Request {
            json_id: Some(json_id.clone()),
            method,
        },
        None => JsonRpcLineKind::Notification,
    }
}

fn emit_disconnect_errors(
    pending: &PendingRequests,
    stdout: &Arc<Mutex<io::Stdout>>,
    reason: &str,
) {
    let requests = pending.take_all();
    if requests.is_empty() {
        return;
    }
    let mut out = stdout.lock().expect("stdout mutex poisoned");
    for request in requests {
        let _ = write_disconnect_error(&mut *out, request, reason);
    }
    let _ = out.flush();
}

fn emit_single_disconnect_error(
    stdout: &Arc<Mutex<io::Stdout>>,
    request: PendingRequest,
    reason: &str,
) {
    let mut out = stdout.lock().expect("stdout mutex poisoned");
    let _ = write_disconnect_error(&mut *out, request, reason);
    let _ = out.flush();
}

fn write_disconnect_error<W: Write>(
    out: &mut W,
    request: PendingRequest,
    reason: &str,
) -> io::Result<()> {
    let method = request.method.as_deref().unwrap_or("request");
    let response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": request.json_id,
        "error": {
            "code": -32001,
            "message": format!("MCP transport disconnected before {method} response: {reason}"),
        }
    });
    serde_json::to_writer(&mut *out, &response)?;
    writeln!(out)
}

#[cfg(test)]
#[path = "mcp_server/tests.rs"]
mod tests;
