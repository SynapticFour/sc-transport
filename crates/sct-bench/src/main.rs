use anyhow::Result;
use clap::Parser;
use sct_core::transport::{SctEndpoint, TransportConfig};
use std::net::SocketAddr;
use std::time::Instant;

#[derive(Debug, Parser)]
#[command(name = "sct-bench")]
struct Cli {
    #[arg(long, default_value_t = 10)]
    samples: u32,
    #[arg(long, default_value_t = 64)]
    payload_mib: usize,
    #[arg(long)]
    endpoint: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(endpoint) = cli.endpoint.as_deref() {
        let throughput = network_throughput_estimate_mbps(endpoint, cli.samples).await?;
        println!(
            "sct-bench network summary: endpoint={}, samples={}, estimated_throughput_mbps={:.2}",
            endpoint, cli.samples, throughput
        );
    } else {
        let throughput = synthetic_throughput_mbps(cli.samples, cli.payload_mib);
        println!(
            "sct-bench summary: samples={}, payload_mib={}, synthetic_throughput_mbps={:.2}",
            cli.samples, cli.payload_mib, throughput
        );
    }
    Ok(())
}

fn synthetic_throughput_mbps(samples: u32, payload_mib: usize) -> f64 {
    let mut vals = Vec::new();
    for _ in 0..samples.max(1) {
        let payload = vec![0x42u8; payload_mib * 1024 * 1024];
        let start = Instant::now();
        let _hash = blake3::hash(&payload);
        let elapsed = start.elapsed().as_secs_f64().max(0.000_001);
        vals.push((payload.len() as f64 * 8.0 / 1_000_000.0) / elapsed);
    }
    vals.iter().sum::<f64>() / vals.len() as f64
}

async fn network_throughput_estimate_mbps(endpoint: &str, samples: u32) -> Result<f64> {
    let addr = parse_endpoint(endpoint)?;
    let ep = SctEndpoint::client(TransportConfig {
        bind_addr: "0.0.0.0:0".parse()?,
        ..Default::default()
    })?;
    let mut vals = Vec::new();
    for _ in 0..samples.max(1) {
        let conn = ep.connect(addr, "localhost").await?;
        let rtt_s = conn.rtt().as_secs_f64().max(0.000_001);
        vals.push((conn.congestion_window() as f64 * 8.0 / 1_000_000.0) / rtt_s);
    }
    Ok(vals.iter().sum::<f64>() / vals.len() as f64)
}

fn parse_endpoint(value: &str) -> Result<SocketAddr> {
    let trimmed = value.strip_prefix("sct://").unwrap_or(value);
    Ok(trimmed.parse()?)
}

#[cfg(test)]
mod tests {
    use super::synthetic_throughput_mbps;

    #[test]
    fn synthetic_throughput_is_positive() {
        let t = synthetic_throughput_mbps(2, 1);
        assert!(t > 0.0);
    }
}
