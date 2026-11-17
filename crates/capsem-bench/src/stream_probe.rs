//! Product SDK stream measurements; terminal projections are not raw PTY bytes.
use crate::{
    collector::{Collected, RawMetric},
    schema::Unit,
};
use anyhow::{bail, ensure, Context, Result};
use capsem_sdk::{
    models::stream::{StreamControl, StreamKind},
    transport::{CallOptions, GatewayStream, StreamEvent, Transport},
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const EXEC_BYTES: usize = 100 * 1024 * 1024;
const TERMINAL_BYTES: usize = 16 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(300);

#[derive(clap::Args, Debug)]
pub(crate) struct Args {
    #[arg(long)]
    gateway_url: String,
    #[arg(long)]
    vm_id: String,
    #[arg(long)]
    quick: bool,
}

pub(crate) async fn run(args: Args) -> Result<Collected> {
    let token =
        std::env::var("CAPSEM_BENCH_GATEWAY_BEARER").context("explicit benchmark gateway credential missing")?;
    let transport = Transport::new(&args.gateway_url, &token, Duration::from_secs(30))?;
    let mut metrics = BTreeMap::new();
    let mut terminal = open(&transport, &args.vm_id, StreamKind::Terminal, None).await?;
    // Only command output contains the contiguous marker. The command's echo
    // contains the printf format and its separate arguments instead.
    terminal.send_stdin(b"printf '%s%s\\n' BENCH_ READY\n").await?;
    until_marker(&mut terminal, b"BENCH_READY").await?;
    for index in 0..if args.quick { 50 } else { 400 } {
        let marker = format!("B{index:04}END");
        let mut input = Vec::with_capacity(marker.len() + 1);
        input.push(0x15); // kill the preceding editable line
        input.extend_from_slice(marker.as_bytes());
        let started = Instant::now();
        terminal.send_stdin(&input).await?;
        until_marker(&mut terminal, marker.as_bytes()).await?;
        record(
            &mut metrics,
            "terminal.edit_projection_ms",
            Unit::Milliseconds,
            started.elapsed().as_secs_f64() * 1000.0,
        );
    }
    terminal.send_stdin(&[0x15]).await?;
    drop(terminal);
    for index in 0..if args.quick { 1 } else { 3 } {
        let mut terminal = open(&transport, &args.vm_id, StreamKind::Terminal, None).await?;
        let marker = format!("BENCH_COMPLETE_{index:04}");
        let command =
            format!("\x15head -c {TERMINAL_BYTES} /dev/zero | base64; printf '%s%s\\n' BENCH_COMPLETE_ {index:04}\n");
        let started = Instant::now();
        terminal.send_stdin(command.as_bytes()).await?;
        let projected = until_marker(&mut terminal, marker.as_bytes()).await?;
        record(
            &mut metrics,
            "terminal.output_completion_seconds",
            Unit::Seconds,
            started.elapsed().as_secs_f64(),
        );
        record(&mut metrics, "terminal.projection_bytes", Unit::Bytes, projected as f64);
        drop(terminal);
        let command = format!("head -c {EXEC_BYTES} /dev/zero");
        let mut execution = open(&transport, &args.vm_id, StreamKind::Exec, Some(command)).await?;
        let started = Instant::now();
        let mut transfer = ExecTransfer::default();
        tokio::time::timeout(DEADLINE, async {
            while !transfer.observe(execution.next().await?)? {}
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("exec transfer deadline exceeded")??;
        let seconds = started.elapsed().as_secs_f64();
        record(
            &mut metrics,
            "exec.output_mbps",
            Unit::MegabitsPerSecond,
            EXEC_BYTES as f64 * 8.0 / 1_000_000.0 / seconds,
        );
        record(&mut metrics, "exec.output_100mib_seconds", Unit::Seconds, seconds);
    }
    Ok(Collected { metrics, sidecar: None })
}

fn record(metrics: &mut BTreeMap<String, RawMetric>, key: &str, unit: Unit, sample: f64) {
    metrics
        .entry(key.into())
        .or_insert_with(|| RawMetric { unit, samples: vec![] })
        .samples
        .push(sample);
}

async fn open(transport: &Transport, id: &str, kind: StreamKind, command: Option<String>) -> Result<GatewayStream> {
    Ok(transport
        .open_stream(id, StreamControl::Start { kind, command }, CallOptions::default())
        .await?)
}

async fn until_marker(stream: &mut GatewayStream, needle: &[u8]) -> Result<usize> {
    tokio::time::timeout(DEADLINE, async {
        let mut marker = Marker::new(needle);
        let mut projected = 0;
        loop {
            match stream.next().await? {
                StreamEvent::Output(bytes) => {
                    projected += bytes.len();
                    if marker.observe(&bytes) {
                        return Ok(projected);
                    }
                }
                StreamEvent::Terminal { .. } => {}
                _ => bail!("terminal ended before its expected projection"),
            }
        }
    })
    .await
    .context("terminal projection deadline exceeded")?
}

#[derive(Default)]
struct ExecTransfer {
    received: usize,
}
impl ExecTransfer {
    fn observe(&mut self, event: StreamEvent) -> Result<bool> {
        match event {
            StreamEvent::Output(bytes) => {
                self.received = self
                    .received
                    .checked_add(bytes.len())
                    .context("exec byte count overflow")?;
                ensure!(
                    self.received <= EXEC_BYTES && bytes.iter().all(|byte| *byte == 0),
                    "exec output differs from the requested zero bytes"
                );
                Ok(false)
            }
            StreamEvent::Exit { code, truncated } => {
                ensure!(
                    code == 0 && !truncated && self.received == EXEC_BYTES,
                    "exec transfer did not finish exactly"
                );
                Ok(true)
            }
            _ => bail!("unexpected non-stdout exec event"),
        }
    }
}

/// Bounded streaming prefix matcher: projections may split the marker across
/// frames. No accumulated terminal history or copied output payload is needed.
struct Marker<'a> {
    needle: &'a [u8],
    prefix: Vec<usize>,
    matched: usize,
}
impl<'a> Marker<'a> {
    fn new(needle: &'a [u8]) -> Self {
        assert!(!needle.is_empty());
        let mut prefix = vec![0; needle.len()];
        let mut matched = 0;
        for index in 1..needle.len() {
            while matched > 0 && needle[index] != needle[matched] {
                matched = prefix[matched - 1];
            }
            if needle[index] == needle[matched] {
                matched += 1;
            }
            prefix[index] = matched;
        }
        Self {
            needle,
            prefix,
            matched: 0,
        }
    }
    fn observe(&mut self, bytes: &[u8]) -> bool {
        for byte in bytes {
            while self.matched > 0 && *byte != self.needle[self.matched] {
                self.matched = self.prefix[self.matched - 1];
            }
            if *byte == self.needle[self.matched] {
                self.matched += 1;
            }
            if self.matched == self.needle.len() {
                self.matched = self.prefix[self.matched - 1];
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests;
