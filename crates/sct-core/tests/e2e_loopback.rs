//! End-to-end loopback transfer tests (separate test binary).
//!
//! ```text
//! cargo test -p sct-core e2e_loopback
//! cargo test -p sct-core --test e2e_loopback
//! ```

use sct_core::protocol::CompressionType;
use sct_core::receiver::{FileReceiver, ReceiverConfig};
use sct_core::sender::{FileSender, SenderConfig};
use sct_core::transport::{SctEndpoint, TransportConfig};
use serial_test::serial;
use std::net::SocketAddr;

mod common;

/// Payload- und Chunk-Größe: **ein Chunk** vermeidet den aktuell noch instabilen Multi-Chunk-FEC-Pfad
/// (mehrere parallele Uni-Streams / Parity). Vier MiByte in einem Stream hängt in diesem Setup
/// oft an Flow-Control; für große Last eher `chunk_size` auf Teilgrößen setzen, sobald FEC stabil ist.
const TEST_BYTES: usize = 512 * 1024;

struct LossHintGuard;
impl Drop for LossHintGuard {
    fn drop(&mut self) {
        std::env::remove_var("SC_SCT_ADAPTIVE_LOSS_HINT");
    }
}

/// Loopback-Transfer mit **5 % Adaptive-Loss-Hint** (`SC_SCT_ADAPTIVE_LOSS_HINT`).
///
/// Echter zufälliger Paketverlust auf QUIC ist hier nicht simuliert — nur der Sender-Loss-Hint für
/// FEC/Congestion. API: echtes `SctEndpoint` + `FileSender`/`FileReceiver` wie in `transfer_smoke`.
#[tokio::test]
#[serial]
async fn e2e_loopback_transfer_5pct_loss() {
    common::with_timeout("e2e_loopback_transfer_5pct_loss", 120, async {
        std::env::set_var("SC_SCT_ADAPTIVE_LOSS_HINT", "0.05");
        let _loss_hint = LossHintGuard;

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
        let server_name = format!("e2e-loopback-{}", addr.port());
        let conn = client.connect(addr, &server_name).await.expect("connect");

        let send_cfg = SenderConfig {
            chunk_size: TEST_BYTES,
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
        assert_eq!(data, received, "Daten nach Loopback + 5% Loss-Hint");
    })
    .await;
}
