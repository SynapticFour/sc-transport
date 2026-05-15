//! Subprocess smoke: `sct recv --once` + `sct send` over loopback QUIC.

#[path = "../harness/smoke.rs"]
mod smoke;

use serial_test::serial;
use smoke::{file_bytes, sct_bin, spawn_and_wait_ok, with_timeout, ChildGuard};
use std::net::TcpListener;
use tokio::process::Command;

#[tokio::test]
#[serial]
async fn cli_send_recv_loopback_smoke() {
    with_timeout("cli_send_recv_loopback_smoke", 120, async {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);

        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", tmp.path().to_str().expect("utf8"));

        let src = tmp.path().join("payload.bin");
        let payload: Vec<u8> = (0..32 * 1024).map(|i| (i % 251) as u8).collect();
        tokio::fs::write(&src, &payload).await.expect("write src");

        let out_dir = tmp.path().join("recv");
        tokio::fs::create_dir_all(&out_dir).await.expect("mkdir recv");

        let sct = sct_bin();
        let endpoint = format!("sct://127.0.0.1:{port}");

        let mut recv_cmd = Command::new(&sct);
        recv_cmd
            .args([
                "recv",
                "--port",
                &port.to_string(),
                "--output-dir",
                out_dir.to_str().expect("utf8"),
                "--once",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());
        let mut recv_child = recv_cmd
            .spawn()
            .expect("spawn recv");
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        let mut send_cmd = Command::new(&sct);
        send_cmd.args(["send", src.to_str().expect("utf8"), &endpoint, "--quiet"]);
        let _send_guard = spawn_and_wait_ok(&mut send_cmd, "sct send").await;

        smoke::wait_for_log_line(&mut recv_child, "received:", 60).await;
        let _recv_guard = ChildGuard(recv_child);

        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&out_dir).await.expect("read_dir");
        while let Some(entry) = read_dir.next_entry().await.expect("read_dir entry") {
            entries.push(entry);
        }
        assert_eq!(entries.len(), 1, "expected one received file");
        let got = file_bytes(&entries[0].path()).await;
        assert_eq!(got, payload);
    })
    .await;
}

