//! End-to-end loopback transfer tests (separate test binary).
//!
//! FEC recovery uses the `test-hooks` feature (simulated chunk loss on the receiver).
//!
//! ```text
//! cargo test -p sct-core --features test-hooks e2e_loopback
//! cargo test -p sct-core --features test-hooks --test e2e_loopback
//! ```

#[cfg(not(feature = "test-hooks"))]
compile_error!("e2e_loopback tests require building sct-core with --features test-hooks");

use sct_core::protocol::CompressionType;
use sct_core::receiver::{FileReceiver, ReceiverConfig};
use sct_core::sender::{FileSender, SenderConfig};
use sct_core::transport::{SctEndpoint, TransportConfig};
use serial_test::serial;
use std::net::SocketAddr;

mod common;

/// 4 Chunks × 64 KiB — ein RS-Block bei `data_shards=4` (Loss-Hint 0.05, niedriger Loopback-RTT).
const CHUNK_SIZE: usize = 64 * 1024;
const TEST_BYTES: usize = 4 * CHUNK_SIZE;
const LOSS_HINT_DATA_SHARDS_4: &str = "0.05";
/// Loss-Hint 0.10 → 64 KiB-Chunks, `data_shards=2` (Loopback-RTT, RS-Recovery-E2E).
const LOSS_HINT_FEC_RECOVERY: &str = "0.10";

struct EnvGuard {
    vars: &'static [&'static str],
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for v in self.vars {
            std::env::remove_var(v);
        }
    }
}

/// Multi-Chunk + Loss-Hint ohne simulierten Verlust (Parity auf dem Wire, alle Daten-Chunks an).
#[tokio::test]
#[serial]
async fn e2e_loopback_transfer_5pct_loss_multi_chunk() {
    common::with_timeout("e2e_loopback_transfer_5pct_loss_multi_chunk", 120, async {
        std::env::set_var("SC_SCT_ADAPTIVE_LOSS_HINT", LOSS_HINT_DATA_SHARDS_4);
        let _env = EnvGuard {
            vars: &["SC_SCT_ADAPTIVE_LOSS_HINT"],
        };

        let data: Vec<u8> = (0..TEST_BYTES).map(|i| (i % 251) as u8).collect();
        let tmp_in = tempfile::NamedTempFile::new().expect("tmp_in");
        std::fs::write(tmp_in.path(), &data).expect("write tmp_in");

        let recv_tmp = tempfile::tempdir().expect("recv tempdir");
        std::env::set_var("HOME", recv_tmp.path());

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
        let server_name = format!("e2e-loopback-mc-{}", addr.port());
        let conn = client.connect(addr, &server_name).await.expect("connect");

        let send_cfg = SenderConfig {
            chunk_size: CHUNK_SIZE,
            compression: CompressionType::None,
            ..Default::default()
        };
        let sender = FileSender::new(conn, send_cfg);
        let src_path = tmp_in.path().to_path_buf();
        let send_task = tokio::spawn(async move { sender.send(&src_path).await });

        let (send_res, recv_res) = tokio::join!(send_task, recv_task);
        send_res.expect("send join").expect("send");
        let out_path = recv_res.expect("recv join");

        let received = tokio::fs::read(&out_path).await.expect("read output");
        assert_eq!(data, received);
    })
    .await;
}

/// Simulierter Verlust von Chunk 1; RS-Recovery (`data_shards=2`, Loss-Hint 0.10).
#[tokio::test]
#[serial]
async fn e2e_loopback_fec_recovery() {
    common::with_timeout("e2e_loopback_fec_recovery", 120, async {
        std::env::set_var("SC_SCT_ADAPTIVE_LOSS_HINT", LOSS_HINT_FEC_RECOVERY);
        std::env::set_var(sct_core::receiver::TEST_SIMULATE_LOST_CHUNK_INDICES_ENV, "1");
        let _env = EnvGuard {
            vars: &[
                "SC_SCT_ADAPTIVE_LOSS_HINT",
                sct_core::receiver::TEST_SIMULATE_LOST_CHUNK_INDICES_ENV,
            ],
        };

        let data: Vec<u8> = (0..TEST_BYTES).map(|i| (i % 251) as u8).collect();
        let tmp_in = tempfile::NamedTempFile::new().expect("tmp_in");
        std::fs::write(tmp_in.path(), &data).expect("write tmp_in");

        let recv_tmp = tempfile::tempdir().expect("recv tempdir");
        std::env::set_var("HOME", recv_tmp.path());

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
        let server_name = format!("e2e-loopback-fec-{}", addr.port());
        let conn = client.connect(addr, &server_name).await.expect("connect");

        let send_cfg = SenderConfig {
            chunk_size: CHUNK_SIZE,
            compression: CompressionType::None,
            ..Default::default()
        };
        let sender = FileSender::new(conn, send_cfg);
        let src_path = tmp_in.path().to_path_buf();
        let send_task = tokio::spawn(async move { sender.send(&src_path).await });

        let (send_res, recv_res) = tokio::join!(send_task, recv_task);
        send_res.expect("send join").expect("send");
        let out_path = recv_res.expect("recv join");

        let received = tokio::fs::read(&out_path).await.expect("read output");
        assert_eq!(
            data, received,
            "FEC recovery: Chunk 1 simuliert verloren, Datei vollständig rekonstruiert"
        );
    })
    .await;
}
