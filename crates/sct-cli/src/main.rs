use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};
use sct_core::congestion::{CongestionOracle, LinkType};
use sct_core::protocol::CompressionType;
use sct_core::receiver::{FileReceiver, ReceiverConfig};
use sct_core::sender::{FileSender, SenderConfig};
use sct_core::transport::{SctEndpoint, TransportConfig};
use serde_json::json;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Parser)]
#[command(name = "sct")]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Send {
        file_or_dir: PathBuf,
        endpoint: String,
        #[arg(long, value_enum, default_value_t = CompressionArg::Auto)]
        compression: CompressionArg,
        #[arg(long, default_value_t = 16)]
        parallel: usize,
        #[arg(long)]
        bandwidth: Option<u64>,
        #[arg(long, default_value_t = false)]
        resume: bool,
        #[arg(long, default_value_t = true)]
        verify: bool,
        #[arg(long, default_value_t = false)]
        json: bool,
        #[arg(long, default_value_t = false)]
        quiet: bool,
    },
    Recv {
        #[arg(long, default_value_t = 7272)]
        port: u16,
        #[arg(long, default_value = ".")]
        output_dir: PathBuf,
        #[arg(long, default_value_t = false)]
        once: bool,
        #[arg(long, default_value_t = false)]
        daemon: bool,
        #[arg(long, default_value_t = false)]
        resume: bool,
        #[arg(long, default_value_t = true)]
        verify: bool,
    },
    Bench {
        endpoint: String,
        #[arg(long, default_value_t = 5)]
        samples: u32,
    },
    Probe {
        endpoint: String,
        #[arg(long, default_value_t = 5)]
        samples: u32,
        #[arg(long, default_value_t = false)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CompressionArg {
    None,
    Zstd,
    Auto,
}

#[derive(Debug, Clone)]
struct ParsedEndpoint {
    addr: SocketAddr,
    server_name: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cli = Cli::parse();
    match cli.cmd {
        Command::Send {
            file_or_dir,
            endpoint,
            compression,
            parallel,
            bandwidth,
            resume,
            verify: _verify,
            json: as_json,
            quiet,
        } => {
            let parsed = parse_endpoint(&endpoint)?;
            let bar = ProgressBar::new_spinner();
            if !quiet && !as_json {
                bar.set_style(ProgressStyle::default_spinner().template("{spinner:.green} {msg}")?);
                bar.enable_steady_tick(std::time::Duration::from_millis(120));
                bar.set_message("connecting and sending...");
            }
            let ep = SctEndpoint::client(TransportConfig {
                bind_addr: "0.0.0.0:0".parse()?,
                target_bandwidth_mbps: bandwidth,
                ..Default::default()
            })?;
            let conn = ep.connect(parsed.addr, &parsed.server_name).await?;
            let selected = select_compression(&file_or_dir, compression)?;
            let sender = FileSender::new(
                conn,
                SenderConfig {
                    max_parallel_chunks: parallel,
                    compression: selected,
                    progress_callback: Some(Arc::new(move |p| {
                        if as_json {
                            println!(
                                "{}",
                                json!({
                                    "bytes_sent": p.bytes_sent,
                                    "bytes_total": p.bytes_total,
                                    "throughput_mbps": p.throughput_mbps,
                                    "elapsed_ms": p.elapsed.as_millis()
                                })
                            );
                        }
                    })),
                    ..Default::default()
                },
            );
            if resume {
                eprintln!("resume requested: sender will use receiver bitmap acknowledgements");
            }
            sender.send(&file_or_dir).await?;
            if !quiet && !as_json {
                bar.finish_with_message("transfer completed");
            }
        }
        Command::Recv {
            port,
            output_dir,
            once,
            daemon,
            resume,
            verify,
        } => {
            if daemon {
                eprintln!("daemon mode active; running receive loop");
            }
            let ep = SctEndpoint::server(TransportConfig {
                bind_addr: SocketAddr::from(([0, 0, 0, 0], port)),
                ..Default::default()
            })?;
            let receiver = FileReceiver::new(
                ep,
                output_dir,
                ReceiverConfig {
                    resume_partial: resume,
                    verify_checksums: verify,
                    ..Default::default()
                },
            );
            if once || !daemon {
                let out = receiver.accept_transfer().await?;
                println!("received: {}", out.display());
            } else {
                loop {
                    let out = receiver.accept_transfer().await?;
                    println!("received: {}", out.display());
                }
            }
        }
        Command::Bench { endpoint, samples } => {
            let parsed = parse_endpoint(&endpoint)?;
            let ep = SctEndpoint::client(TransportConfig {
                bind_addr: "0.0.0.0:0".parse()?,
                ..Default::default()
            })?;
            let mut throughput_estimates_mbps = Vec::new();
            for _ in 0..samples {
                let conn = ep.connect(parsed.addr, &parsed.server_name).await?;
                let rtt_s = conn.rtt().as_secs_f64();
                let cwnd_bytes = conn.congestion_window() as f64;
                if rtt_s > 0.0 {
                    let est_mbps = (cwnd_bytes * 8.0 / 1_000_000.0) / rtt_s;
                    throughput_estimates_mbps.push(est_mbps);
                }
            }
            let avg = if throughput_estimates_mbps.is_empty() {
                0.0
            } else {
                throughput_estimates_mbps.iter().sum::<f64>()
                    / throughput_estimates_mbps.len() as f64
            };
            println!(
                "bench summary: target={}, samples={}, est_throughput_mbps={:.2}",
                parsed.addr,
                throughput_estimates_mbps.len(),
                avg
            );
        }
        Command::Probe {
            endpoint,
            samples,
            json: as_json,
        } => {
            let parsed = parse_endpoint(&endpoint)?;
            let ep = SctEndpoint::client(TransportConfig {
                bind_addr: "0.0.0.0:0".parse()?,
                ..Default::default()
            })?;
            let mut samples_ms = Vec::new();
            for _ in 0..samples {
                let started = Instant::now();
                let conn = ep.connect(parsed.addr, &parsed.server_name).await?;
                let elapsed = started.elapsed();
                let rtt = conn.rtt();
                samples_ms.push(rtt.as_secs_f64() * 1000.0);
                println!(
                    "probe sample: connect_ms={:.2}, rtt_ms={:.2}, cwnd_bytes={}",
                    elapsed.as_secs_f64() * 1000.0,
                    rtt.as_secs_f64() * 1000.0,
                    conn.congestion_window()
                );
            }
            samples_ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let min = samples_ms.first().copied().unwrap_or_default();
            let avg = if samples_ms.is_empty() {
                0.0
            } else {
                samples_ms.iter().sum::<f64>() / samples_ms.len() as f64
            };
            println!(
                "network profile estimate: min_rtt_ms={:.2}, avg_rtt_ms={:.2}, samples={}",
                min,
                avg,
                samples_ms.len()
            );
            let profile = CongestionOracle::probe(&ep, parsed.addr).await;
            let link_type = match profile.link_type {
                LinkType::Datacenter => "datacenter",
                LinkType::Continental => "continental",
                LinkType::Intercontinental => "intercontinental",
            };
            if as_json {
                println!(
                    "{}",
                    json!({
                        "target": parsed.addr,
                        "samples": samples_ms.len(),
                        "min_rtt_ms": profile.min_rtt.as_secs_f64() * 1000.0,
                        "avg_rtt_ms": profile.avg_rtt.as_secs_f64() * 1000.0,
                        "jitter_ms": profile.jitter.as_secs_f64() * 1000.0,
                        "estimated_bandwidth_mbps": profile.estimated_bandwidth_mbps,
                        "loss_rate": profile.loss_rate,
                        "suggested_initial_cwnd": profile.suggested_initial_cwnd,
                        "link_type": link_type
                    })
                );
            } else {
                println!(
                    "oracle profile: link_type={}, min_rtt_ms={:.2}, avg_rtt_ms={:.2}, jitter_ms={:.2}, est_bw_mbps={:.2}, loss_rate={:.2}",
                    link_type,
                    profile.min_rtt.as_secs_f64() * 1000.0,
                    profile.avg_rtt.as_secs_f64() * 1000.0,
                    profile.jitter.as_secs_f64() * 1000.0,
                    profile.estimated_bandwidth_mbps,
                    profile.loss_rate
                );
            }
        }
    }
    Ok(())
}

fn parse_endpoint(value: &str) -> Result<ParsedEndpoint> {
    let trimmed = value.strip_prefix("sct://").unwrap_or(value);
    let addr: SocketAddr = trimmed.parse()?;
    let server_name = if addr.ip().is_loopback() {
        "localhost".to_string()
    } else {
        addr.ip().to_string()
    };
    Ok(ParsedEndpoint { addr, server_name })
}

fn select_compression(path: &PathBuf, mode: CompressionArg) -> Result<CompressionType> {
    let selected = match mode {
        CompressionArg::None => CompressionType::None,
        CompressionArg::Zstd => CompressionType::Zstd { level: 3 },
        CompressionArg::Auto => {
            let size = fs::metadata(path)?.len();
            if size > 8 * 1024 * 1024 {
                CompressionType::Zstd { level: 1 }
            } else {
                CompressionType::None
            }
        }
    };
    Ok(selected)
}
