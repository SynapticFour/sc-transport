use sct_core::protocol::{
    encode, read_framed, write_framed, ChunkDescriptor, CompressionType, FinalAck, ManifestAck,
    TransferComplete, TransferManifest,
};
use sct_core::receiver::{FileReceiver, ReceiverConfig};
use sct_core::sender::{FileSender, SenderConfig};
use sct_core::transport::{SctConnection, SctEndpoint, TransportConfig};
use serial_test::serial;
use std::collections::HashMap;
use std::net::SocketAddr;
mod common;

fn framed_chunk_payload(index: u64, body: &[u8], chunk_size: usize) -> Vec<u8> {
    let off = index as usize * chunk_size;
    let desc = ChunkDescriptor {
        index,
        offset: off as u64,
        compressed_size: body.len() as u32,
        uncompressed_size: body.len() as u32,
        checksum: *blake3::hash(body).as_bytes(),
        was_compressed: false,
        is_parity: false,
        parity_index: 0,
        fec_group: index,
    };
    let desc_bytes = encode(&desc).expect("encode chunk descriptor");
    let mut payload = Vec::with_capacity(4 + desc_bytes.len() + body.len());
    payload.extend_from_slice(&(desc_bytes.len() as u32).to_be_bytes());
    payload.extend_from_slice(&desc_bytes);
    payload.extend_from_slice(body);
    payload
}

async fn write_chunk_on_stream(conn: &SctConnection, wire: Vec<u8>) {
    let mut stream = conn.open_data_stream().await.expect("open data stream");
    stream.write_all(&wire).await.expect("write chunk");
    stream.finish().expect("finish data stream");
}

#[tokio::test]
#[serial]
async fn duplicate_chunks_are_deduplicated() {
    common::with_timeout("duplicate_chunks_are_deduplicated", 60, async {
        let recv_tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", recv_tmp.path());
        let chunk_size = 64 * 1024;
        let payload = vec![0x42_u8; chunk_size * 2];
        let file_checksum = *blake3::hash(&payload).as_bytes();
        let filename = "dedup.bin";
        let mut transfer_id = [0_u8; 16];
        transfer_id.copy_from_slice(&blake3::hash(filename.as_bytes()).as_bytes()[..16]);
        let hex = transfer_id
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let state_path = recv_tmp.path().join(format!("{filename}.{hex}.state.json"));

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
        let recv_task =
            tokio::spawn(async move { receiver.accept_transfer().await.expect("receive") });

        let client = SctEndpoint::client(TransportConfig {
            bind_addr: "0.0.0.0:0".parse().expect("addr"),
            ..Default::default()
        })
        .expect("client");
        let server_name = format!("dedup-{}", addr.port());
        let conn = client.connect(addr, &server_name).await.expect("connect");

        let manifest = TransferManifest {
            transfer_id,
            filename: filename.to_string(),
            total_size: payload.len() as u64,
            chunk_size: chunk_size as u32,
            num_chunks: 2,
            checksum_algorithm: sct_proto::ChecksumAlg::Blake3,
            file_checksum,
            compression: CompressionType::None,
            metadata: HashMap::new(),
            data_shards: 1,
            parity_shards: 0,
        };
        let (mut ctrl_send, mut ctrl_recv) = conn.open_control_stream().await.expect("control");
        write_framed(&mut ctrl_send, &manifest)
            .await
            .expect("manifest");
        let ack: ManifestAck = read_framed(&mut ctrl_recv).await.expect("manifest ack");
        assert!(ack.accepted);

        let chunk0 = &payload[..chunk_size];
        let chunk1 = &payload[chunk_size..];
        let wire0 = framed_chunk_payload(0, chunk0, chunk_size);
        let wire1 = framed_chunk_payload(1, chunk1, chunk_size);
        write_chunk_on_stream(&conn, wire0.clone()).await;
        write_chunk_on_stream(&conn, wire0).await;

        let mut state_raw = None;
        for _ in 0..100 {
            if state_path.exists() {
                state_raw = Some(tokio::fs::read(&state_path).await.expect("resume state"));
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let state_raw = state_raw.expect("resume state not written after duplicate chunk");
        let state: serde_json::Value = serde_json::from_slice(&state_raw).expect("state json");
        let received = state["received_chunks"]
            .as_array()
            .expect("received_chunks array");
        assert_eq!(
            received.len(),
            1,
            "duplicate index must not be counted twice"
        );

        write_chunk_on_stream(&conn, wire1).await;
        write_framed(
            &mut ctrl_send,
            &TransferComplete {
                transfer_id: manifest.transfer_id,
            },
        )
        .await
        .expect("complete");
        let final_ack: FinalAck = read_framed(&mut ctrl_recv).await.expect("final ack");
        assert!(final_ack.success);

        let out_path = recv_task.await.expect("join");
        let got = tokio::fs::read(out_path).await.expect("read output");
        assert_eq!(got, payload);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn sender_receiver_transfer_smoke() {
    common::with_timeout("sender_receiver_transfer_smoke", 120, async {
        let recv_tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", recv_tmp.path());
        let send_tmp = tempfile::tempdir().expect("tempdir");
        let src_path = send_tmp.path().join("payload.bin");
        let payload = vec![0xAB_u8; 128 * 1024];
        tokio::fs::write(&src_path, &payload)
            .await
            .expect("write src");

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
        let recv_task =
            tokio::spawn(async move { receiver.accept_transfer().await.expect("receive") });

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
    })
    .await;
}

#[tokio::test]
#[serial]
async fn sender_receiver_resume_from_bitmap_state() {
    common::with_timeout("sender_receiver_resume_from_bitmap_state", 120, async {
        let recv_tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", recv_tmp.path());
        let send_tmp = tempfile::tempdir().expect("tempdir");
        let src_path = send_tmp.path().join("resume.bin");
        let payload = vec![0xCD_u8; 256 * 1024];
        tokio::fs::write(&src_path, &payload)
            .await
            .expect("write src");

        let filename = "resume.bin";
        let hash = blake3::hash(filename.as_bytes());
        let mut transfer_id = [0_u8; 16];
        transfer_id.copy_from_slice(&hash.as_bytes()[..16]);
        let hex = transfer_id
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let temp_path = recv_tmp.path().join(format!("{filename}.{hex}.part"));
        let state_path = recv_tmp.path().join(format!("{filename}.{hex}.state.json"));

        // Prepare a partial state with chunk index 0 already received.
        let chunk_size = 64 * 1024;
        let mut pre = vec![0_u8; payload.len()];
        pre[..chunk_size].copy_from_slice(&payload[..chunk_size]);
        tokio::fs::write(&temp_path, &pre)
            .await
            .expect("write part");
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
        let recv_task =
            tokio::spawn(async move { receiver.accept_transfer().await.expect("receive") });

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
    })
    .await;
}

#[tokio::test]
#[serial]
async fn resume_ignores_corrupted_state_file() {
    common::with_timeout("resume_ignores_corrupted_state_file", 120, async {
        let recv_tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", recv_tmp.path());
        let send_tmp = tempfile::tempdir().expect("tempdir");
        let src_path = send_tmp.path().join("corrupt-state.bin");
        let payload = vec![0xEF_u8; 96 * 1024];
        tokio::fs::write(&src_path, &payload)
            .await
            .expect("write src");

        let filename = "corrupt-state.bin";
        let hash = blake3::hash(filename.as_bytes());
        let mut transfer_id = [0_u8; 16];
        transfer_id.copy_from_slice(&hash.as_bytes()[..16]);
        let hex = transfer_id
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let state_path = recv_tmp.path().join(format!("{filename}.{hex}.state.json"));
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
        let recv_task =
            tokio::spawn(async move { receiver.accept_transfer().await.expect("receive") });

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
    })
    .await;
}

#[tokio::test]
#[serial]
async fn missing_final_ack_fails_in_strict_mode() {
    common::with_timeout("missing_final_ack_fails_in_strict_mode", 60, async {
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
            let (mut ctrl_send, mut ctrl_recv) =
                conn.accept_control_stream().await.expect("control");
            let _manifest: TransferManifest = read_framed(&mut ctrl_recv).await.expect("manifest");
            write_framed(
                &mut ctrl_send,
                &ManifestAck {
                    accepted: true,
                    message: None,
                    received_chunks: Vec::new(),
                    chunk_hashes: vec![],
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
    })
    .await;
}

#[test]
fn missing_chunk_list_is_capped_and_sorted() {
    use std::collections::HashSet;
    // Simuliert: manifest.num_chunks=200, received_chunks enthält alle außer
    // 100 zufällig verteilten — Vec muss <= 64 Einträge haben, aufsteigend.
    let num_chunks = 200u64;
    let received: HashSet<u64> = (0..num_chunks).filter(|i| i % 2 == 0).collect();
    let mut missing: Vec<u64> = (0..num_chunks)
        .filter(|i| !received.contains(i))
        .take(64)
        .collect();
    missing.sort_unstable();

    assert!(missing.len() <= 64);
    assert!(missing.windows(2).all(|w| w[0] < w[1]), "nicht sortiert");
    assert_eq!(
        missing[0], 1,
        "kleinster fehlender Chunk muss zuerst kommen"
    );
}

#[test]
fn fec_recovery_with_one_missing_data_shard() {
    // 4 Datashards, 2 Parity — simuliere 1 fehlenden Datashard
    use reed_solomon_erasure::galois_8::ReedSolomon;
    let rs = ReedSolomon::new(4, 2).unwrap();
    let chunk = vec![0xABu8; 256];
    let mut shards: Vec<Vec<u8>> = (0..6).map(|_| chunk.clone()).collect();
    rs.encode(&mut shards).unwrap();
    let mut shards: Vec<Option<Vec<u8>>> = shards.into_iter().map(Some).collect();
    // Simuliere fehlenden Datashard 2
    shards[2] = None;
    // Muss mindestens 4 Shards vorhanden haben: 5 von 6 → Recovery möglich
    let received = shards.iter().filter(|s| s.is_some()).count();
    assert!(received >= 4);
    rs.reconstruct_data(&mut shards).expect("reconstruct_data");
    assert!(shards[2].is_some(), "Datashard 2 muss rekonstruiert sein");
}
