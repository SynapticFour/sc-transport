//! Minimal QUIC datagram send smoke (localhost server + a few events).
use sc_transport_core::{DeliveryStatus, EventType, TelemetryEvent, Transport};
use sc_transport_datagrams::{QuicDatagramConfig, QuicDatagramTransport};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[tokio::test]
async fn quic_datagram_basic_send_to_local_server() {
    use quinn::ServerConfig;
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("cert");
    let cert_der: CertificateDer<'static> = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
    let server_config =
        ServerConfig::with_single_cert(vec![cert_der], key_der.into()).expect("server cfg");
    let server_endpoint =
        quinn::Endpoint::server(server_config, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .expect("server");
    let addr = server_endpoint.local_addr().expect("addr");
    let recv_count = Arc::new(AtomicU64::new(0));
    let recv_count_task = recv_count.clone();
    let server_task = tokio::spawn(async move {
        let incoming = server_endpoint.accept().await.expect("incoming");
        let conn = incoming.await.expect("conn");
        for _ in 0..8 {
            if conn.read_datagram().await.is_ok() {
                recv_count_task.fetch_add(1, Ordering::Relaxed);
            }
        }
    });

    let transport = QuicDatagramTransport::with_config(QuicDatagramConfig {
        server_addr: addr,
        ..Default::default()
    });
    let run_id = "quic-datagram-basic";
    for i in 0..8_u64 {
        let status = transport
            .send_event(
                run_id,
                TelemetryEvent {
                    run_id: run_id.to_string(),
                    task_id: None,
                    event_type: EventType::Progress,
                    timestamp_ms: i,
                    payload: serde_json::json!({ "i": i }),
                },
            )
            .await
            .expect("send");
        assert!(
            matches!(status, DeliveryStatus::Sent),
            "unexpected status: {status:?}"
        );
    }
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), server_task)
        .await
        .expect("server timeout");
    assert!(
        recv_count.load(Ordering::Relaxed) >= 6,
        "datagrams received"
    );
}
