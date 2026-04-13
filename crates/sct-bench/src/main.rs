#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use sct_core::transport::{SctEndpoint, TransportConfig};
use serde::Serialize;
use std::fs::{self, File};
#[cfg(target_os = "linux")]
use std::io::{BufRead, BufReader};
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Parser)]
#[command(name = "sct-bench")]
struct Cli {
    #[command(subcommand)]
    cmd: CommandArg,
}

#[derive(Debug, Subcommand)]
enum CommandArg {
    Synthetic {
        #[arg(long, default_value_t = 10)]
        samples: u32,
        #[arg(long, default_value_t = 64)]
        payload_mib: usize,
        #[arg(long)]
        endpoint: Option<String>,
    },
    NetemMatrix {
        #[arg(long, value_enum, default_value_t = ProfileArg::All)]
        profile: ProfileArg,
        #[arg(long, default_value = "lo")]
        interface: String,
        #[arg(long, default_value = "1,16,256,1024")]
        sizes_mib: String,
        #[arg(long, default_value_t = 7272)]
        base_port: u16,
        #[arg(long, default_value_t = true)]
        use_sudo: bool,
        #[arg(long)]
        output_json: Option<PathBuf>,
        #[arg(long, default_value_t = false)]
        keep_files: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ProfileArg {
    Good,
    Bad,
    VeryBad,
    All,
}

#[derive(Debug, Clone, Copy)]
struct NetemProfile {
    name: &'static str,
    delay_ms: u32,
    jitter_ms: u32,
    loss_pct: f64,
    reorder_pct: f64,
    duplicate_pct: f64,
    corrupt_pct: f64,
    rate_mbit: u32,
}

#[derive(Debug, Serialize)]
struct ScenarioResult {
    profile: String,
    size_mib: usize,
    duration_s: f64,
    throughput_mbps: f64,
    sent_bytes: u64,
    recv_bytes: u64,
    ok: bool,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct NetemRunSummary {
    interface: String,
    sizes_mib: Vec<usize>,
    results: Vec<ScenarioResult>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        CommandArg::Synthetic {
            samples,
            payload_mib,
            endpoint,
        } => {
            if let Some(endpoint) = endpoint.as_deref() {
                let throughput = network_throughput_estimate_mbps(endpoint, samples).await?;
                println!(
                    "sct-bench network summary: endpoint={}, samples={}, estimated_throughput_mbps={:.2}",
                    endpoint, samples, throughput
                );
            } else {
                let throughput = synthetic_throughput_mbps(samples, payload_mib);
                println!(
                    "sct-bench summary: samples={}, payload_mib={}, synthetic_throughput_mbps={:.2}",
                    samples, payload_mib, throughput
                );
            }
        }
        CommandArg::NetemMatrix {
            profile,
            interface,
            sizes_mib,
            base_port,
            use_sudo,
            output_json,
            keep_files,
        } => {
            run_netem_matrix(
                profile,
                &interface,
                parse_sizes(&sizes_mib)?,
                base_port,
                use_sudo,
                output_json.as_deref(),
                keep_files,
            )?;
        }
    }
    Ok(())
}

fn run_netem_matrix(
    profile_arg: ProfileArg,
    interface: &str,
    sizes_mib: Vec<usize>,
    base_port: u16,
    use_sudo: bool,
    output_json: Option<&Path>,
    keep_files: bool,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        ensure_tc_available(use_sudo)?;
        ensure_sct_binary_exists()?;
        let profiles = selected_profiles(profile_arg);
        let run_root = PathBuf::from("target/sct-bench-netem");
        fs::create_dir_all(&run_root)?;

        let mut results = Vec::new();
        let mut port = base_port;
        for profile in profiles {
            apply_profile(interface, profile, use_sudo)
                .with_context(|| format!("failed applying profile {}", profile.name))?;
            for size_mib in &sizes_mib {
                let outcome = run_transfer_case(&run_root, profile.name, *size_mib, port);
                match outcome {
                    Ok((duration_s, sent, recv)) => {
                        let throughput = (sent as f64 * 8.0 / 1_000_000.0) / duration_s.max(0.000_001);
                        println!(
                            "[{}] size={}MiB duration={:.2}s throughput={:.2}Mbps sent={} recv={} status=ok",
                            profile.name, size_mib, duration_s, throughput, sent, recv
                        );
                        results.push(ScenarioResult {
                            profile: profile.name.to_string(),
                            size_mib: *size_mib,
                            duration_s,
                            throughput_mbps: throughput,
                            sent_bytes: sent,
                            recv_bytes: recv,
                            ok: sent == recv,
                            error: (sent != recv).then(|| "sent/recv size mismatch".to_string()),
                        });
                    }
                    Err(err) => {
                        println!("[{}] size={}MiB status=error error={}", profile.name, size_mib, err);
                        results.push(ScenarioResult {
                            profile: profile.name.to_string(),
                            size_mib: *size_mib,
                            duration_s: 0.0,
                            throughput_mbps: 0.0,
                            sent_bytes: 0,
                            recv_bytes: 0,
                            ok: false,
                            error: Some(err.to_string()),
                        });
                    }
                }
                port = port.saturating_add(1);
            }
            clear_profile(interface, use_sudo)?;
        }

        if !keep_files {
            let _ = fs::remove_dir_all(&run_root);
        }

        let summary = NetemRunSummary {
            interface: interface.to_string(),
            sizes_mib,
            results,
        };
        if let Some(path) = output_json {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = File::create(path)?;
            serde_json::to_writer_pretty(&mut file, &summary)?;
            file.write_all(b"\n")?;
            println!("wrote netem report: {}", path.display());
        } else {
            println!("{}", serde_json::to_string_pretty(&summary)?);
        }
        return Ok(());
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (profile_arg, interface, sizes_mib, base_port, use_sudo, output_json, keep_files);
        bail!("netem matrix requires Linux because tc/netem is Linux-only");
    }
}

fn selected_profiles(profile: ProfileArg) -> Vec<NetemProfile> {
    let good = NetemProfile {
        name: "good",
        delay_ms: 8,
        jitter_ms: 2,
        loss_pct: 0.01,
        reorder_pct: 0.01,
        duplicate_pct: 0.0,
        corrupt_pct: 0.0,
        rate_mbit: 950,
    };
    let bad = NetemProfile {
        name: "bad",
        delay_ms: 75,
        jitter_ms: 20,
        loss_pct: 1.0,
        reorder_pct: 0.2,
        duplicate_pct: 0.1,
        corrupt_pct: 0.05,
        rate_mbit: 80,
    };
    let very_bad = NetemProfile {
        name: "very-bad",
        delay_ms: 220,
        jitter_ms: 85,
        loss_pct: 8.0,
        reorder_pct: 3.0,
        duplicate_pct: 1.0,
        corrupt_pct: 0.3,
        rate_mbit: 15,
    };
    match profile {
        ProfileArg::Good => vec![good],
        ProfileArg::Bad => vec![bad],
        ProfileArg::VeryBad => vec![very_bad],
        ProfileArg::All => vec![good, bad, very_bad],
    }
}

fn run_transfer_case(
    root: &Path,
    profile_name: &str,
    size_mib: usize,
    port: u16,
) -> Result<(f64, u64, u64)> {
    let case_dir = root.join(format!("{profile_name}-{size_mib}mib-{port}"));
    fs::create_dir_all(&case_dir)?;
    let send_file = case_dir.join("payload.bin");
    let recv_dir = case_dir.join("recv");
    fs::create_dir_all(&recv_dir)?;
    write_test_file(&send_file, size_mib)?;
    let sent_size = fs::metadata(&send_file)?.len();

    let recv_proc = start_receiver(port, &recv_dir)?;
    thread::sleep(Duration::from_millis(400));
    let started = Instant::now();
    run_sender(&send_file, port)?;
    let duration_s = started.elapsed().as_secs_f64().max(0.000_001);
    wait_receiver(recv_proc)?;
    let recv_size = first_file_size(&recv_dir)?;
    Ok((duration_s, sent_size, recv_size))
}

fn write_test_file(path: &Path, size_mib: usize) -> Result<()> {
    let mut f = File::create(path)?;
    let total_bytes = size_mib * 1024 * 1024;
    let mut remaining = total_bytes;
    let chunk = vec![0x5Au8; 1024 * 1024];
    while remaining > 0 {
        let n = remaining.min(chunk.len());
        f.write_all(&chunk[..n])?;
        remaining -= n;
    }
    Ok(())
}

fn start_receiver(port: u16, output_dir: &Path) -> Result<Child> {
    Command::new("target/debug/sct")
        .arg("recv")
        .arg("--port")
        .arg(port.to_string())
        .arg("--output-dir")
        .arg(output_dir)
        .arg("--once")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to start receiver")
}

fn run_sender(send_file: &Path, port: u16) -> Result<()> {
    let out = Command::new("target/debug/sct")
        .arg("send")
        .arg(send_file)
        .arg(format!("sct://127.0.0.1:{port}"))
        .arg("--quiet")
        .output()?;
    if !out.status.success() {
        bail!(
            "sender failed: {}",
            String::from_utf8_lossy(&out.stderr).trim().to_string()
        );
    }
    Ok(())
}

fn wait_receiver(child: Child) -> Result<()> {
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "receiver failed: {}",
            String::from_utf8_lossy(&out.stderr).trim().to_string()
        );
    }
    Ok(())
}

fn first_file_size(dir: &Path) -> Result<u64> {
    let entries = fs::read_dir(dir)?;
    for entry in entries {
        let path = entry?.path();
        if path.is_file() {
            return Ok(fs::metadata(path)?.len());
        }
    }
    bail!("receiver produced no output file")
}

fn ensure_sct_binary_exists() -> Result<()> {
    let mut build = Command::new("cargo");
    build
        .arg("build")
        .arg("-p")
        .arg("sct-cli")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let out = build.output()?;
    if !out.status.success() {
        bail!(
            "failed to build sct-cli: {}",
            String::from_utf8_lossy(&out.stderr).trim().to_string()
        );
    }
    Ok(())
}

fn parse_sizes(value: &str) -> Result<Vec<usize>> {
    let sizes: Result<Vec<_>> = value
        .split(',')
        .map(|v| {
            v.trim()
                .parse::<usize>()
                .with_context(|| format!("invalid size '{v}'"))
        })
        .collect();
    let sizes = sizes?;
    if sizes.is_empty() {
        bail!("at least one size is required");
    }
    Ok(sizes)
}

#[cfg(target_os = "linux")]
fn ensure_tc_available(use_sudo: bool) -> Result<()> {
    let status = Command::new("sh")
        .arg("-c")
        .arg("command -v tc >/dev/null 2>&1")
        .status()?;
    if !status.success() {
        bail!("tc is required but not found on PATH");
    }
    if is_root() || !use_sudo {
        return Ok(());
    }
    let status = Command::new("sudo").arg("-n").arg("true").status();
    if status.map(|s| s.success()).unwrap_or(false) {
        Ok(())
    } else {
        bail!("need root or passwordless sudo for tc operations")
    }
}

#[cfg(target_os = "linux")]
fn apply_profile(interface: &str, profile: NetemProfile, use_sudo: bool) -> Result<()> {
    clear_profile(interface, use_sudo)?;
    let mut args = vec![
        "qdisc".to_string(),
        "add".to_string(),
        "dev".to_string(),
        interface.to_string(),
        "root".to_string(),
        "netem".to_string(),
        "delay".to_string(),
        format!("{}ms", profile.delay_ms),
        format!("{}ms", profile.jitter_ms),
        "loss".to_string(),
        format!("{:.3}%", profile.loss_pct),
        "reorder".to_string(),
        format!("{:.3}%", profile.reorder_pct),
        "25%".to_string(),
        "duplicate".to_string(),
        format!("{:.3}%", profile.duplicate_pct),
        "corrupt".to_string(),
        format!("{:.3}%", profile.corrupt_pct),
        "rate".to_string(),
        format!("{}mbit", profile.rate_mbit),
    ];
    run_tc(&mut args, use_sudo)
}

#[cfg(target_os = "linux")]
fn clear_profile(interface: &str, use_sudo: bool) -> Result<()> {
    let mut args = vec![
        "qdisc".to_string(),
        "del".to_string(),
        "dev".to_string(),
        interface.to_string(),
        "root".to_string(),
    ];
    let out = run_tc_output(&mut args, use_sudo)?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("No such file or directory") || stderr.contains("Cannot delete qdisc with handle of zero") {
        return Ok(());
    }
    bail!("failed to clear tc qdisc: {}", stderr.trim())
}

#[cfg(target_os = "linux")]
fn run_tc(args: &mut [String], use_sudo: bool) -> Result<()> {
    let out = run_tc_output(args, use_sudo)?;
    if out.status.success() {
        Ok(())
    } else {
        bail!("tc failed: {}", String::from_utf8_lossy(&out.stderr).trim())
    }
}

#[cfg(target_os = "linux")]
fn run_tc_output(args: &mut [String], use_sudo: bool) -> Result<std::process::Output> {
    let mut cmd = if use_sudo && !is_root() {
        let mut c = Command::new("sudo");
        c.arg("-n").arg("tc");
        c
    } else {
        Command::new("tc")
    };
    for arg in args {
        cmd.arg(arg);
    }
    Ok(cmd.output()?)
}

#[cfg(target_os = "linux")]
fn is_root() -> bool {
    if let Ok(file) = File::open("/proc/self/status") {
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if let Some(rest) = line.strip_prefix("Uid:") {
                let uid = rest.split_whitespace().next().unwrap_or_default();
                return uid == "0";
            }
        }
    }
    false
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
    trimmed
        .parse()
        .map_err(|e| anyhow!("invalid endpoint '{trimmed}': {e}"))
}

#[cfg(test)]
mod tests {
    use super::{parse_sizes, synthetic_throughput_mbps};

    #[test]
    fn synthetic_throughput_is_positive() {
        let t = synthetic_throughput_mbps(2, 1);
        assert!(t > 0.0);
    }

    #[test]
    fn parses_sizes_csv() {
        let sizes = parse_sizes("1,16,256").expect("size csv should parse");
        assert_eq!(sizes, vec![1, 16, 256]);
    }
}
