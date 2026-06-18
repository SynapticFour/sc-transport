//! Daemon REST smoke: receive path (`sct send` → daemon) and push path (daemon → `sct recv`).

#[path = "../harness/smoke.rs"]
mod smoke;

use serial_test::serial;
use smoke::{file_bytes, sct_bin, sct_daemon_bin, with_timeout, ChildGuard};
use std::net::TcpListener;
use tokio::process::Command;

#[tokio::test]
#[serial]
async fn daemon_receive_via_rest_and_cli_send() {
    with_timeout("daemon_receive_via_rest_and_cli_send", 180, async {
        let quic_listener = TcpListener::bind("127.0.0.1:0").expect("bind quic");
        let quic_port = quic_listener.local_addr().expect("addr").port();
        drop(quic_listener);
        let api_listener = TcpListener::bind("127.0.0.1:0").expect("bind api");
        let api_port = api_listener.local_addr().expect("addr").port();
        drop(api_listener);

        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", tmp.path().to_str().expect("utf8"));
        std::env::set_var(
            "SCT_DAEMON_STATE_PATH",
            tmp.path().join("transfers.json").to_str().expect("utf8"),
        );
        std::env::set_var(
            "SCT_DAEMON_EVENTS_PATH",
            tmp.path().join("events.jsonl").to_str().expect("utf8"),
        );

        let data_dir = tmp.path();
        let out_dir = data_dir.join("incoming");
        tokio::fs::create_dir_all(&out_dir).await.expect("mkdir");

        let cfg = format!(
            r#"[server]
listen_port = {quic_port}
api_port = {api_port}
max_concurrent_transfers = 2
output_base_dir = "{data}"

[transfer]
default_chunk_size_mb = 1
default_parallel_streams = 4
default_compression = "none"
enable_resumption = true

[auth]
mode = "tofu"
known_hosts_file = "~/.sct/known_hosts"

[metrics]
enabled = false
prometheus_port = 9090

[logging]
level = "warn"
format = "text"
"#,
            data = data_dir.display()
        );
        let cfg_path = tmp.path().join("sct.toml");
        tokio::fs::write(&cfg_path, cfg)
            .await
            .expect("write config");

        let src = tmp.path().join("daemon-payload.bin");
        let payload: Vec<u8> = (0..48 * 1024).map(|i| (i % 200) as u8).collect();
        tokio::fs::write(&src, &payload).await.expect("write src");

        let mut daemon_cmd = Command::new(sct_daemon_bin());
        daemon_cmd
            .current_dir(tmp.path())
            .env("RUST_LOG", "warn")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit());
        let daemon_child = daemon_cmd.spawn().expect("spawn daemon");
        smoke::wait_for_http_ok(&format!("http://127.0.0.1:{api_port}/v1/health"), 30).await;

        let client = reqwest::Client::new();

        let submit_body = serde_json::json!({
            "source": format!("sct://127.0.0.1:{quic_port}"),
            "destination": "incoming/",
            "priority": 5
        });
        let submit = client
            .post(format!("http://127.0.0.1:{api_port}/v1/transfer"))
            .json(&submit_body)
            .send()
            .await
            .expect("submit");
        assert_eq!(submit.status(), reqwest::StatusCode::ACCEPTED);
        let submit_json: serde_json::Value = submit.json().await.expect("submit json");
        let transfer_id = submit_json["transfer_id"].as_str().expect("transfer_id");

        let endpoint = format!("sct://127.0.0.1:{quic_port}");
        let mut send_cmd = Command::new(sct_bin());
        send_cmd.args([
            "send",
            src.to_str().expect("utf8"),
            &endpoint,
            "--quiet",
            "--compression",
            "none",
        ]);
        let send_task = tokio::spawn(async move {
            let mut cmd = send_cmd;
            let status = cmd.status().await.expect("send status");
            assert!(status.success(), "sct send failed: {status}");
        });

        let mut completed = false;
        for _ in 0..240 {
            let row = client
                .get(format!(
                    "http://127.0.0.1:{api_port}/v1/transfer/{transfer_id}"
                ))
                .send()
                .await
                .expect("get transfer");
            if row.status().is_success() {
                let body: serde_json::Value = row.json().await.expect("row json");
                if body["status"].as_str() == Some("completed") {
                    completed = true;
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        send_task.await.expect("send join");
        assert!(completed, "transfer did not reach Completed status");

        let got_path = out_dir.join("daemon-payload.bin");
        let got = file_bytes(&got_path).await;
        assert_eq!(got, payload);

        let _daemon_guard = ChildGuard(daemon_child);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn daemon_push_via_rest_and_cli_recv() {
    with_timeout("daemon_push_via_rest_and_cli_recv", 180, async {
        let recv_listener = TcpListener::bind("127.0.0.1:0").expect("bind recv");
        let recv_port = recv_listener.local_addr().expect("addr").port();
        drop(recv_listener);
        let api_listener = TcpListener::bind("127.0.0.1:0").expect("bind api");
        let api_port = api_listener.local_addr().expect("addr").port();
        drop(api_listener);

        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", tmp.path().to_str().expect("utf8"));
        std::env::set_var(
            "SCT_DAEMON_STATE_PATH",
            tmp.path().join("transfers.json").to_str().expect("utf8"),
        );
        std::env::set_var(
            "SCT_DAEMON_EVENTS_PATH",
            tmp.path().join("events.jsonl").to_str().expect("utf8"),
        );

        let data_dir = tmp.path();
        let recv_out = data_dir.join("recv-out");
        tokio::fs::create_dir_all(&recv_out)
            .await
            .expect("mkdir recv-out");

        let cfg = format!(
            r#"[server]
listen_port = 17272
api_port = {api_port}
max_concurrent_transfers = 2
output_base_dir = "{data}"

[transfer]
default_chunk_size_mb = 1
default_parallel_streams = 4
default_compression = "none"
enable_resumption = true

[auth]
mode = "tofu"
known_hosts_file = "~/.sct/known_hosts"

[metrics]
enabled = false
prometheus_port = 9090

[logging]
level = "warn"
format = "text"
"#,
            data = data_dir.display()
        );
        tokio::fs::write(tmp.path().join("sct.toml"), cfg)
            .await
            .expect("write config");

        let src = tmp.path().join("push-payload.bin");
        let payload: Vec<u8> = (0..32 * 1024).map(|i| (i % 173) as u8).collect();
        tokio::fs::write(&src, &payload).await.expect("write src");

        let sct = sct_bin();
        let mut recv_cmd = Command::new(&sct);
        recv_cmd.args([
            "recv",
            "--port",
            &recv_port.to_string(),
            "--output-dir",
            recv_out.to_str().expect("utf8"),
            "--once",
        ]);
        recv_cmd.stdout(std::process::Stdio::null());
        let mut recv_child = recv_cmd.spawn().expect("spawn recv");
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;

        let mut daemon_cmd = Command::new(sct_daemon_bin());
        daemon_cmd
            .current_dir(tmp.path())
            .env("RUST_LOG", "warn")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit());
        let daemon_child = daemon_cmd.spawn().expect("spawn daemon");
        smoke::wait_for_http_ok(&format!("http://127.0.0.1:{api_port}/v1/health"), 30).await;

        let client = reqwest::Client::new();
        let submit_body = serde_json::json!({
            "source": src.to_str().expect("utf8"),
            "destination": format!("sct://127.0.0.1:{recv_port}"),
            "priority": 5
        });
        let submit = client
            .post(format!("http://127.0.0.1:{api_port}/v1/transfer"))
            .json(&submit_body)
            .send()
            .await
            .expect("submit");
        assert_eq!(submit.status(), reqwest::StatusCode::ACCEPTED);
        let submit_json: serde_json::Value = submit.json().await.expect("submit json");
        let transfer_id = submit_json["transfer_id"].as_str().expect("transfer_id");

        let mut completed = false;
        for _ in 0..240 {
            let row = client
                .get(format!(
                    "http://127.0.0.1:{api_port}/v1/transfer/{transfer_id}"
                ))
                .send()
                .await
                .expect("get transfer");
            if row.status().is_success() {
                let body: serde_json::Value = row.json().await.expect("row json");
                if body["status"].as_str() == Some("completed") {
                    completed = true;
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        assert!(completed, "push transfer did not reach completed status");

        let _ = recv_child.wait().await;
        let got_path = recv_out.join("push-payload.bin");
        let got = file_bytes(&got_path).await;
        assert_eq!(got, payload);

        let _daemon_guard = ChildGuard(daemon_child);
        let _recv_guard = ChildGuard(recv_child);
    })
    .await;
}
