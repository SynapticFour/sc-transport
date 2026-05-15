//! Shared subprocess helpers for CLI/daemon integration tests.
//! Each `[[test]]` binary uses only a subset; allow dead_code under `-D warnings`.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::timeout;

fn smoke_bin(env_key: &str, bin_name: &str) -> PathBuf {
    if let Ok(path) = std::env::var(env_key) {
        return PathBuf::from(path);
    }
    if let Ok(path) = std::env::var(format!("CARGO_BIN_EXE_{bin_name}")) {
        return PathBuf::from(path);
    }
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    target.join(profile).join(bin_name)
}

pub fn sct_bin() -> PathBuf {
    smoke_bin("SCT_SMOKE_BIN", "sct")
}

pub fn sct_daemon_bin() -> PathBuf {
    smoke_bin("SCT_SMOKE_DAEMON_BIN", "sct-daemon")
}

pub async fn with_timeout<T>(
    label: &'static str,
    secs: u64,
    fut: impl std::future::Future<Output = T>,
) -> T {
    timeout(Duration::from_secs(secs), fut)
        .await
        .unwrap_or_else(|_| {
            panic!("{label}: exceeded {secs}s. Run with --test-threads=1 if ports contend.")
        })
}

pub struct ChildGuard(pub Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.start_kill();
    }
}

pub async fn spawn_and_wait_ok(cmd: &mut Command, label: &str) -> ChildGuard {
    let mut child = cmd.spawn().unwrap_or_else(|e| panic!("{label} spawn: {e}"));
    let status = timeout(Duration::from_secs(90), child.wait())
        .await
        .unwrap_or_else(|_| panic!("{label} timed out"))
        .unwrap_or_else(|e| panic!("{label} wait: {e}"));
    assert!(status.success(), "{label} exited with {status}");
    ChildGuard(child)
}

pub async fn wait_for_log_line(child: &mut Child, needle: &str, secs: u64) {
    let stdout = child.stdout.take().expect("stdout piped");
    let mut lines = BufReader::new(stdout).lines();
    let found = timeout(Duration::from_secs(secs), async {
        while let Ok(Some(line)) = lines.next_line().await {
            if line.contains(needle) {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(
        found,
        "expected log line containing `{needle}` within {secs}s"
    );
}

pub async fn file_bytes(path: &Path) -> Vec<u8> {
    tokio::fs::read(path).await.expect("read file")
}

pub async fn wait_for_http_ok(url: &str, attempts: u32) {
    let client = reqwest::Client::new();
    for _ in 0..attempts {
        if let Ok(resp) = client.get(url).send().await {
            if resp.status().is_success() {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("GET {url} did not succeed within {attempts} attempts");
}
