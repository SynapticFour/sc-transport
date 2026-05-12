//! Loopback transfer benchmark: prints one JSON object to stdout.
//!
//! Same wiring as `sct-core` integration tests: ephemeral `HOME` (TLS paths via
//! `dirs::home_dir`), `127.0.0.1:0` loopback, `FileReceiver` + `FileSender`, default
//! `SenderConfig` / `ReceiverConfig`.
//!
//! ```text
//! SC_SCT_BENCH_ITERATIONS=32 SC_SCT_BENCH_BYTES=1048576 \
//!   cargo run -p sct-core --example bench-transfer --release
//! ```
//!
//! Optional: `SC_SCT_BENCH_TIMEOUT_SECS` (default 180), `SC_SCT_COMPLETION_FIRST=1`, etc.

use anyhow::{Context, Result};
use serde::Serialize;
use sct_core::receiver::{FileReceiver, ReceiverConfig};
use sct_core::sender::{FileSender, SenderConfig};
use sct_core::transport::{SctEndpoint, TransportConfig};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::time::timeout;

#[derive(Serialize)]
struct BenchReport {
    sct_core_version: &'static str,
    iterations: usize,
    payload_bytes: usize,
    /// `iterations * payload_bytes` — total application bytes moved (same as one file of that size).
    aggregate_payload_bytes: usize,
    timeout_secs: u64,
    elapsed_secs: Vec<f64>,
    min_elapsed_secs: f64,
    max_elapsed_secs: f64,
    mean_elapsed_secs: f64,
    p50_elapsed_secs: f64,
    p95_elapsed_secs: f64,
    goodput_mbps_mean: f64,
    goodput_mbps_p50: f64,
    /// Wall seconds for all iterations (`sum(elapsed_secs)`).
    aggregate_wall_secs: f64,
    /// Goodput over the whole run: `(iterations * payload_bytes) * 8 / 1e6 / aggregate_wall_secs`.
    aggregate_goodput_mbps: f64,
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p * sorted.len() as f64).ceil() as usize).saturating_sub(1);
    sorted[idx.min(sorted.len() - 1)]
}

async fn one_transfer(payload_bytes: usize, port_hint: u16) -> Result<f64> {
    let prev_home = std::env::var_os("HOME");

    let recv_tmp = tempfile::tempdir().context("recv tempdir")?;
    std::env::set_var("HOME", recv_tmp.path());

    let inner = async {
        let send_tmp = tempfile::tempdir().context("send tempdir")?;
        let src = send_tmp.path().join("bench.bin");
        tokio::fs::write(&src, vec![0xB5_u8; payload_bytes])
            .await
            .context("write payload")?;

        let server = SctEndpoint::server(TransportConfig {
            bind_addr: "127.0.0.1:0".parse().expect("bind"),
            ..Default::default()
        })
        .context("server endpoint")?;
        let addr: SocketAddr = server.local_addr().context("local_addr")?;

        let out_dir = recv_tmp.path().to_path_buf();
        let receiver = FileReceiver::new(
            server,
            out_dir.clone(),
            ReceiverConfig {
                resume_partial: false,
                temp_dir: Some(out_dir),
                ..Default::default()
            },
        );
        let recv_task = tokio::spawn(async move { receiver.accept_transfer().await });

        let client = SctEndpoint::client(TransportConfig {
            bind_addr: "0.0.0.0:0".parse().expect("client bind"),
            ..Default::default()
        })
        .context("client endpoint")?;
        let server_name = format!("bench-{port_hint}-{}", addr.port());
        let conn = client
            .connect(addr, &server_name)
            .await
            .context("connect")?;
        let sender = FileSender::new(conn, SenderConfig::default());

        let started = Instant::now();
        sender.send(&src).await.context("send")?;
        let elapsed = started.elapsed().as_secs_f64();

        let out_path = recv_task.await.context("recv join")??;
        let got = tokio::fs::read(&out_path).await.context("read output")?;
        anyhow::ensure!(
            got.len() == payload_bytes,
            "size mismatch: want {} got {}",
            payload_bytes,
            got.len()
        );

        Ok(elapsed.max(1e-9))
    };

    let result = inner.await;

    match prev_home {
        Some(h) => std::env::set_var("HOME", h),
        None => {
            let _ = std::env::remove_var("HOME");
        }
    }

    result
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> Result<()> {
    let iterations = env_usize("SC_SCT_BENCH_ITERATIONS", 32).max(1);
    // Default 1 MiB per iteration: the library loopback harness stays stable; use many
    // iterations so aggregate bytes dwarf connect/handshake overhead (see NETWORK-EMULATION.md).
    let payload_bytes = env_usize("SC_SCT_BENCH_BYTES", 1024 * 1024).max(1024);
    let timeout_secs = env_u64("SC_SCT_BENCH_TIMEOUT_SECS", 180).max(30);

    let mut elapsed_secs = Vec::with_capacity(iterations);
    for i in 0..iterations {
        let port_hint = (i as u16).wrapping_add(1);
        let dur = timeout(
            Duration::from_secs(timeout_secs),
            one_transfer(payload_bytes, port_hint),
        )
        .await
        .with_context(|| format!("iteration {i} exceeded {timeout_secs}s"))??;
        elapsed_secs.push(dur);
    }

    let mut sorted = elapsed_secs.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mean = elapsed_secs.iter().sum::<f64>() / elapsed_secs.len() as f64;
    let min_e = *sorted.first().unwrap_or(&0.0);
    let max_e = *sorted.last().unwrap_or(&0.0);
    let p50 = percentile(&sorted, 0.50);
    let p95 = percentile(&sorted, 0.95);
    let b = payload_bytes as f64;
    let goodput_mean = (b * 8.0 / 1_000_000.0) / mean;
    let goodput_p50 = (b * 8.0 / 1_000_000.0) / p50.max(1e-9);
    let aggregate_wall: f64 = elapsed_secs.iter().sum();
    let agg_bytes = iterations.saturating_mul(payload_bytes) as f64;
    let aggregate_goodput = (agg_bytes * 8.0 / 1_000_000.0) / aggregate_wall.max(1e-9);

    let report = BenchReport {
        sct_core_version: env!("CARGO_PKG_VERSION"),
        iterations,
        payload_bytes,
        aggregate_payload_bytes: iterations.saturating_mul(payload_bytes),
        timeout_secs,
        elapsed_secs,
        min_elapsed_secs: min_e,
        max_elapsed_secs: max_e,
        mean_elapsed_secs: mean,
        p50_elapsed_secs: p50,
        p95_elapsed_secs: p95,
        goodput_mbps_mean: goodput_mean,
        goodput_mbps_p50: goodput_p50,
        aggregate_wall_secs: aggregate_wall,
        aggregate_goodput_mbps: aggregate_goodput,
    };

    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
