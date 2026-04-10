use sct_core::protocol::CompressionType;
use sct_core::receiver::{FileReceiver, ReceiverConfig};
use sct_core::sender::{FileSender, SenderConfig};
use sct_core::transport::{SctEndpoint, TransportConfig};
use serial_test::serial;
use std::net::SocketAddr;
use std::time::Instant;

async fn start_receiver(
    output: std::path::PathBuf,
    resume: bool,
) -> (SocketAddr, tokio::task::JoinHandle<std::path::PathBuf>) {
    let server = SctEndpoint::server(TransportConfig {
        bind_addr: "127.0.0.1:0".parse().expect("addr"),
        ..Default::default()
    })
    .expect("server");
    let addr = server.local_addr().expect("addr");
    let receiver = FileReceiver::new(
        server,
        output.clone(),
        ReceiverConfig {
            resume_partial: resume,
            temp_dir: Some(output),
            ..Default::default()
        },
    );
    let task = tokio::spawn(async move { receiver.accept_transfer().await.expect("receive") });
    (addr, task)
}

#[tokio::test]
#[serial]
async fn loopback_throughput_is_nonzero() {
    let recv_tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HOME", recv_tmp.path());
    let send_tmp = tempfile::tempdir().expect("tempdir");
    let src = send_tmp.path().join("throughput.bin");
    tokio::fs::write(&src, vec![0x33_u8; 2 * 1024 * 1024])
        .await
        .expect("write");
    let (addr, recv_task) = start_receiver(recv_tmp.path().to_path_buf(), false).await;
    let client = SctEndpoint::client(TransportConfig {
        bind_addr: "0.0.0.0:0".parse().expect("addr"),
        ..Default::default()
    })
    .expect("client");
    let server_name = format!("prompt5-{}", addr.port());
    let conn = client.connect(addr, &server_name).await.expect("connect");
    let sender = FileSender::new(conn, SenderConfig::default());
    let started = Instant::now();
    sender.send(&src).await.expect("send");
    let elapsed = started.elapsed().as_secs_f64().max(0.000_001);
    let throughput_mbps = ((2 * 1024 * 1024) as f64 * 8.0 / 1_000_000.0) / elapsed;
    assert!(throughput_mbps > 0.1);
    let out = recv_task.await.expect("join");
    assert!(out.exists());
}

#[tokio::test]
#[serial]
async fn compression_effectiveness_for_redundant_payload() {
    let recv_tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HOME", recv_tmp.path());
    let send_tmp = tempfile::tempdir().expect("tempdir");
    let src = send_tmp.path().join("compressible.bin");
    tokio::fs::write(&src, vec![0x00_u8; 1024 * 1024])
        .await
        .expect("write");
    let (addr, recv_task) = start_receiver(recv_tmp.path().to_path_buf(), false).await;
    let client = SctEndpoint::client(TransportConfig {
        bind_addr: "0.0.0.0:0".parse().expect("addr"),
        ..Default::default()
    })
    .expect("client");
    let server_name = format!("prompt5-{}", addr.port());
    let conn = client.connect(addr, &server_name).await.expect("connect");
    let sender = FileSender::new(
        conn,
        SenderConfig {
            compression: CompressionType::Zstd { level: 3 },
            ..Default::default()
        },
    );
    sender.send(&src).await.expect("send");
    let out = recv_task.await.expect("join");
    let got = tokio::fs::read(out).await.expect("read");
    assert_eq!(got.len(), 1024 * 1024);
    assert!(got.iter().all(|b| *b == 0));
}

#[tokio::test]
#[serial]
async fn resume_stress_multiple_iterations() {
    for _ in 0..3 {
        let recv_tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", recv_tmp.path());
        let send_tmp = tempfile::tempdir().expect("tempdir");
        let src = send_tmp.path().join("resume-stress.bin");
        tokio::fs::write(&src, vec![0xA5_u8; 512 * 1024])
            .await
            .expect("write");
        let (addr, recv_task) = start_receiver(recv_tmp.path().to_path_buf(), true).await;
        let client = SctEndpoint::client(TransportConfig {
            bind_addr: "0.0.0.0:0".parse().expect("addr"),
            ..Default::default()
        })
        .expect("client");
        let server_name = format!("prompt5-{}", addr.port());
        let conn = client.connect(addr, &server_name).await.expect("connect");
        let sender = FileSender::new(
            conn,
            SenderConfig {
                chunk_size: 64 * 1024,
                ..Default::default()
            },
        );
        sender.send(&src).await.expect("send");
        let out = recv_task.await.expect("join");
        let got = tokio::fs::read(out).await.expect("read");
        assert_eq!(got.len(), 512 * 1024);
    }
}

#[tokio::test]
#[serial]
async fn multi_file_dataset_transfers() {
    let recv_tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HOME", recv_tmp.path());
    let send_tmp = tempfile::tempdir().expect("tempdir");
    let files = vec![
        ("a.bin", vec![0x11_u8; 32 * 1024]),
        ("b.bin", vec![0x22_u8; 64 * 1024]),
        ("c.bin", vec![0x33_u8; 128 * 1024]),
    ];
    for (name, data) in &files {
        let src = send_tmp.path().join(name);
        tokio::fs::write(&src, data).await.expect("write");
        let (addr, recv_task) = start_receiver(recv_tmp.path().to_path_buf(), false).await;
        let client = SctEndpoint::client(TransportConfig {
            bind_addr: "0.0.0.0:0".parse().expect("addr"),
            ..Default::default()
        })
        .expect("client");
        let server_name = format!("prompt5-{}", addr.port());
        let conn = client.connect(addr, &server_name).await.expect("connect");
        let sender = FileSender::new(conn, SenderConfig::default());
        sender.send(&src).await.expect("send");
        let out = recv_task.await.expect("join");
        let got = tokio::fs::read(out).await.expect("read");
        assert_eq!(&got, data);
    }
}
