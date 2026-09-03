//! Peer-chosen sizes, bounded before anything is allocated for them.
//!
//! A websocket frame header names its own payload length and `/bytes/<n>`
//! names its own body size. Allocating either as declared let a 10-byte frame
//! abort the process and a URL reserve a terabyte. The cap is generous for
//! every fixture (the largest is 10 MiB) and small next to the machine.

use anyhow::{bail, Context, Result};
use hyper::upgrade::Upgraded;
use hyper::StatusCode;
use hyper_util::rt::TokioIo;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Largest websocket payload accepted from a peer.
pub(crate) const MAX_WS_FRAME_BYTES: u64 = 16 * 1024 * 1024;
/// Largest body `/bytes/<n>` and `/gzip/<n>` will generate.
pub(crate) const MAX_GENERATED_BYTES: usize = 16 * 1024 * 1024;
/// RFC 6455 close status: message too big.
const CLOSE_MESSAGE_TOO_BIG: u16 = 1009;

/// A requested generated size, or `None` when it is not a size this server
/// will build.
pub(crate) fn parse_generated_size(size: &str) -> Option<usize> {
    size.parse::<usize>().ok().filter(|len| *len <= MAX_GENERATED_BYTES)
}

/// Why a `/bytes/<n>` or `/gzip/<n>` request produced no body: a number over
/// the cap is refused as too large, anything else is not a route.
pub(crate) fn generated_size_refusal(size: &str) -> StatusCode {
    match size.parse::<u128>() {
        Ok(len) if len > MAX_GENERATED_BYTES as u128 => StatusCode::PAYLOAD_TOO_LARGE,
        _ => StatusCode::NOT_FOUND,
    }
}

pub(crate) async fn read_ws_frame(io: &mut TokioIo<Upgraded>) -> Result<Option<(u8, Vec<u8>)>> {
    let mut header = [0_u8; 2];
    if io.read_exact(&mut header).await.is_err() {
        return Ok(None);
    }
    let opcode = header[0] & 0x0f;
    let masked = header[1] & 0x80 != 0;
    let mut len = u64::from(header[1] & 0x7f);
    if len == 126 {
        let mut bytes = [0_u8; 2];
        io.read_exact(&mut bytes).await?;
        len = u64::from(u16::from_be_bytes(bytes));
    } else if len == 127 {
        let mut bytes = [0_u8; 8];
        io.read_exact(&mut bytes).await?;
        len = u64::from_be_bytes(bytes);
    }
    if len > MAX_WS_FRAME_BYTES {
        tracing::warn!(
            frame_len = len,
            max_frame_len = MAX_WS_FRAME_BYTES,
            "websocket frame declares more than the cap; closing"
        );
        let _ = write_ws_frame(io, 0x8, &CLOSE_MESSAGE_TOO_BIG.to_be_bytes()).await;
        let _ = io.shutdown().await;
        // Closing with unread bytes queued turns the close into a TCP reset
        // that discards the close frame on the peer's side. Drain what has
        // already arrived, bounded in time and buffer so a peer that keeps
        // sending cannot hold the task.
        let _ = tokio::time::timeout(Duration::from_secs(1), async {
            let mut sink = [0_u8; 4096];
            while matches!(io.read(&mut sink).await, Ok(read) if read > 0) {}
        })
        .await;
        bail!("websocket frame of {len} bytes exceeds the {MAX_WS_FRAME_BYTES}-byte cap");
    }
    let mut mask = [0_u8; 4];
    if masked {
        io.read_exact(&mut mask).await?;
    }
    let mut payload = vec![0_u8; usize::try_from(len).context("websocket frame length does not fit usize")?];
    io.read_exact(&mut payload).await?;
    if masked {
        for (idx, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[idx % 4];
        }
    }
    Ok(Some((opcode, payload)))
}

pub(crate) async fn write_ws_frame(io: &mut TokioIo<Upgraded>, opcode: u8, payload: &[u8]) -> Result<()> {
    let mut header = Vec::with_capacity(10);
    header.push(0x80 | opcode);
    if payload.len() < 126 {
        header.push(u8::try_from(payload.len()).expect("len < 126"));
    } else if u16::try_from(payload.len()).is_ok() {
        header.push(126);
        header.extend_from_slice(&u16::try_from(payload.len()).expect("fits").to_be_bytes());
    } else {
        header.push(127);
        header.extend_from_slice(&u64::try_from(payload.len()).expect("fits").to_be_bytes());
    }
    io.write_all(&header).await?;
    io.write_all(payload).await?;
    io.flush().await?;
    Ok(())
}
