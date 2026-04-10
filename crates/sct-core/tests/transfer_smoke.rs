use sct_core::receiver::{FileReceiver, ReceiverConfig};
use sct_core::protocol::{read_framed, write_framed, ManifestAck, TransferComplete, TransferManifest};
use sct_core::sender::{FileSender, SenderConfig};
use sct_core::transport::{SctEndpoint, TransportConfig};
use std::net::SocketAddr;

#[tokio::test]
async fn sender_receiver_transfer_smoke() {
    let recv_tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HOME", recv_tmp.path());
    let send_tmp = tempfile::tempdir().expect("tempdir");
    let src_path = send_tmp.path().join("payload.bin");
    let payload = vec![0xAB_u8; 128 * 1024];
    tokio::fs::write(&src_path, &payload).await.expect("write src");

    let server = SctEndpoint::server(TransportConfig {
        bind_addr: "127.0.0.1:0".parse().expect("addr"),
        ..Default::default()
    })
    .expect("server");
    let addr: SocketAddr = server.local_addr().expect("local addr");

    let receiver = FileReceiver::new(
        server,
        recv_tmp.path().to_path_buf(),
        ReceiverConfig::default(),
    );
    let recv_task = tokio::spawn(async move { receiver.accept_transfer().await.expect("receive") });

    let client = SctEndpoint::client(TransportConfig {
        bind_addr: "0.0.0.0:0".parse().expect("addr"),
        ..Default::default()
    })
    .expect("client");
    let server_name = format!("smoke-{}", addr.port());
    let conn = client.connect(addr, &server_name).await.expect("connect");
    let sender = FileSender::new(conn, SenderConfig::default());
    let send_res = sender.send(&src_path).await;

    let recv_res = recv_task.await.expect("join");
    if let Err(e) = &send_res {
        panic!("send failed: {e:?}; recv={recv_res:?}");
    }
    let out_path = recv_res;
    let got = tokio::fs::read(out_path).await.expect("read output");
    assert_eq!(got, payload);
}

#[tokio::test]
async fn sender_receiver_resume_from_bitmap_state() {
    let recv_tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HOME", recv_tmp.path());
    let send_tmp = tempfile::tempdir().expect("tempdir");
    let src_path = send_tmp.path().join("resume.bin");
    let payload = vec![0xCD_u8; 256 * 1024];
    tokio::fs::write(&src_path, &payload).await.expect("write src");

    let filename = "resume.bin";
    let hash = blake3::hash(filename.as_bytes());
    let mut transfer_id = [0_u8; 16];
    transfer_id.copy_from_slice(&hash.as_bytes()[..16]);
    let hex = transfer_id
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let temp_path = recv_tmp.path().join(format!("{filename}.{hex}.part"));
    let state_path = recv_tmp
        .path()
        .join(format!("{filename}.{hex}.state.json"));

    // Prepare a partial state with chunk index 0 already received.
    let chunk_size = 64 * 1024;
    let mut pre = vec![0_u8; payload.len()];
    pre[..chunk_size].copy_from_slice(&payload[..chunk_size]);
    tokio::fs::write(&temp_path, &pre).await.expect("write part");
    tokio::fs::write(&state_path, br#"{"received_chunks":[0]}"#)
        .await
        .expect("write state");

    let server = SctEndpoint::server(TransportConfig {
        bind_addr: "127.0.0.1:0".parse().expect("addr"),
        ..Default::default()
    })
    .expect("server");
    let addr: SocketAddr = server.local_addr().expect("local addr");

    let receiver = FileReceiver::new(
        server,
        recv_tmp.path().to_path_buf(),
        ReceiverConfig {
            resume_partial: true,
            temp_dir: Some(recv_tmp.path().to_path_buf()),
            ..Default::default()
        },
    );
    let recv_task = tokio::spawn(async move { receiver.accept_transfer().await.expect("receive") });

    let client = SctEndpoint::client(TransportConfig {
        bind_addr: "0.0.0.0:0".parse().expect("addr"),
        ..Default::default()
    })
    .expect("client");
    let server_name = format!("resume-{}", addr.port());
    let conn = client.connect(addr, &server_name).await.expect("connect");
    let sender = FileSender::new(
        conn,
        SenderConfig {
            chunk_size,
            ..Default::default()
        },
    );
    sender.send(&src_path).await.expect("send");

    let out_path = recv_task.await.expect("join");
    let got = tokio::fs::read(out_path).await.expect("read output");
    assert_eq!(got, payload);
}

#[tokio::test]
async fn resume_ignores_corrupted_state_file() {
    let recv_tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HOME", recv_tmp.path());
    let send_tmp = tempfile::tempdir().expect("tempdir");
    let src_path = send_tmp.path().join("corrupt-state.bin");
    let payload = vec![0xEF_u8; 96 * 1024];
    tokio::fs::write(&src_path, &payload).await.expect("write src");

    let filename = "corrupt-state.bin";
    let hash = blake3::hash(filename.as_bytes());
    let mut transfer_id = [0_u8; 16];
    transfer_id.copy_from_slice(&hash.as_bytes()[..16]);
    let hex = transfer_id.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let state_path = recv_tmp
        .path()
        .join(format!("{filename}.{hex}.state.json"));
    tokio::fs::write(&state_path, b"{not-valid-json")
        .await
        .expect("write corrupted state");

    let server = SctEndpoint::server(TransportConfig {
        bind_addr: "127.0.0.1:0".parse().expect("addr"),
        ..Default::default()
    })
    .expect("server");
    let addr: SocketAddr = server.local_addr().expect("local addr");
    let receiver = FileReceiver::new(
        server,
        recv_tmp.path().to_path_buf(),
        ReceiverConfig {
            resume_partial: true,
            temp_dir: Some(recv_tmp.path().to_path_buf()),
            ..Default::default()
        },
    );
    let recv_task = tokio::spawn(async move { receiver.accept_transfer().await.expect("receive") });

    let client = SctEndpoint::client(TransportConfig {
        bind_addr: "0.0.0.0:0".parse().expect("addr"),
        ..Default::default()
    })
    .expect("client");
    let server_name = format!("corrupt-{}", addr.port());
    let conn = client.connect(addr, &server_name).await.expect("connect");
    let sender = FileSender::new(conn, SenderConfig::default());
    sender.send(&src_path).await.expect("send");

    let out_path = recv_task.await.expect("join");
    let got = tokio::fs::read(out_path).await.expect("read output");
    assert_eq!(got, payload);
}

#[tokio::test]
async fn missing_final_ack_fails_in_strict_mode() {
    let send_tmp = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HOME", send_tmp.path());
    let src_path = send_tmp.path().join("no-final-ack.bin");
    tokio::fs::write(&src_path, vec![0xAA_u8; 64 * 1024])
        .await
        .expect("write src");

    let server = SctEndpoint::server(TransportConfig {
        bind_addr: "127.0.0.1:0".parse().expect("addr"),
        ..Default::default()
    })
    .expect("server");
    let addr: SocketAddr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        let conn = server.accept().await.expect("incoming").expect("conn");
        let (mut ctrl_send, mut ctrl_recv) = conn.accept_control_stream().await.expect("control");
        let _manifest: TransferManifest = read_framed(&mut ctrl_recv).await.expect("manifest");
        write_framed(
            &mut ctrl_send,
            &ManifestAck {
                accepted: true,
                message: None,
                received_chunks: Vec::new(),
            },
        )
        .await
        .expect("manifest ack");
        // Consume one data stream if present.
        if let Ok(mut data) = conn.accept_data_stream().await {
            let _ = tokio::io::copy(&mut data, &mut tokio::io::sink()).await;
        }
        let _complete: TransferComplete = read_framed(&mut ctrl_recv).await.expect("complete");
        // Intentionally do not send FinalAck.
    });

    let client = SctEndpoint::client(TransportConfig {
        bind_addr: "0.0.0.0:0".parse().expect("addr"),
        ..Default::default()
    })
    .expect("client");
    let server_name = format!("noack-{}", addr.port());
    let conn = client.connect(addr, &server_name).await.expect("connect");
    let sender = FileSender::new(
        conn,
        SenderConfig {
            chunk_size: 128 * 1024,
            require_final_ack: true,
            ..Default::default()
        },
    );
    let err = tokio::time::timeout(std::time::Duration::from_secs(10), sender.send(&src_path))
        .await
        .expect("timeout")
        .expect_err("must fail without final ack");
    assert!(err.to_string().contains("missing final ack"));
    let _ = server_task.await;
}
