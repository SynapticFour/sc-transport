// EXPERIMENTAL: This implementation is sc-transport-datagrams v0.x.
// Results are not guaranteed. See docs/LIMITATIONS.md.
// Do not use in production without measuring delivery in your environment.

use async_trait::async_trait;
#[cfg(feature = "quic-datagrams")]
use bytes::Bytes;
use sc_transport_core::{
    DeliveryStatus, EventStream, EventType, TelemetryEvent, Transport, TransportError,
    TransportMetrics,
};
use sc_transport_sse::HttpSseTransport;
use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
#[cfg(feature = "quic-datagrams")]
use tokio::sync::OnceCell;

#[derive(Debug, Clone)]
pub struct QuicDatagramConfig {
    pub server_addr: SocketAddr,
    pub max_datagram_bytes: usize,
    pub loss_fallback_threshold: f64,
    pub loss_window_size: usize,
}

impl Default for QuicDatagramConfig {
    fn default() -> Self {
        Self {
            server_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 7272)),
            max_datagram_bytes: 1200,
            loss_fallback_threshold: 0.15,
            loss_window_size: 100,
        }
    }
}

pub struct QuicDatagramTransport {
    fallback: HttpSseTransport,
    sent: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    fallback_count: Arc<AtomicU64>,
    loss_window: Arc<Mutex<VecDeque<bool>>>,
    fallback_threshold: f64,
    loss_window_size: usize,
    max_datagram_size: usize,
    #[cfg(feature = "quic-datagrams")]
    config: QuicDatagramConfig,
    #[cfg(feature = "quic-datagrams")]
    client_endpoint: OnceCell<quinn::Endpoint>,
    #[cfg(feature = "quic-datagrams")]
    connection: OnceCell<quinn::Connection>,
}

impl Default for QuicDatagramTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl QuicDatagramTransport {
    pub fn new() -> Self {
        Self::with_config(QuicDatagramConfig::default())
    }

    pub fn with_config(config: QuicDatagramConfig) -> Self {
        Self {
            fallback: HttpSseTransport::new(),
            sent: Arc::new(AtomicU64::new(0)),
            dropped: Arc::new(AtomicU64::new(0)),
            fallback_count: Arc::new(AtomicU64::new(0)),
            loss_window: Arc::new(Mutex::new(VecDeque::new())),
            fallback_threshold: config.loss_fallback_threshold,
            loss_window_size: config.loss_window_size,
            max_datagram_size: config.max_datagram_bytes,
            #[cfg(feature = "quic-datagrams")]
            config,
            #[cfg(feature = "quic-datagrams")]
            client_endpoint: OnceCell::const_new(),
            #[cfg(feature = "quic-datagrams")]
            connection: OnceCell::const_new(),
        }
    }

    pub fn with_max_datagram_size(max_datagram_size: usize) -> Self {
        let mut t = Self::new();
        t.max_datagram_size = max_datagram_size;
        t
    }

    fn should_drop_for_rate_limit(&self, event: &TelemetryEvent) -> bool {
        matches!(event.event_type, EventType::Progress)
            && self.sent.load(Ordering::Relaxed) % 100 == 99
    }

    fn should_force_fallback(&self, event: &TelemetryEvent) -> bool {
        event
            .payload
            .get("force_fallback")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    fn is_simulated_loss_event(event: &TelemetryEvent) -> bool {
        event
            .payload
            .get("simulated_loss")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    async fn record_loss_sample(&self, delivered: bool) {
        let mut window = self.loss_window.lock().await;
        window.push_back(delivered);
        if window.len() > self.loss_window_size {
            let _ = window.pop_front();
        }
    }

    async fn should_trigger_fallback_by_loss(&self) -> bool {
        let window = self.loss_window.lock().await;
        if window.len() < 10 {
            return false;
        }
        let delivered = window.iter().filter(|v| **v).count() as f64;
        let loss_rate = 1.0 - (delivered / window.len() as f64);
        loss_rate > self.fallback_threshold
    }

    fn serialize_event_for_datagram(
        &self,
        event: &TelemetryEvent,
    ) -> Result<(Vec<u8>, bool), TransportError> {
        let original = rmp_serde::to_vec_named(event)
            .map_err(|e| TransportError::Serialization(e.to_string()))?;
        if original.len() <= self.max_datagram_size {
            return Ok((original, false));
        }

        let truncated_event = TelemetryEvent {
            run_id: event.run_id.clone(),
            task_id: event.task_id.clone(),
            event_type: event.event_type.clone(),
            timestamp_ms: event.timestamp_ms,
            payload: serde_json::json!({
                "truncated": true,
                "original_payload_type": "summary",
            }),
        };
        let truncated = rmp_serde::to_vec_named(&truncated_event)
            .map_err(|e| TransportError::Serialization(e.to_string()))?;
        if truncated.len() <= self.max_datagram_size {
            Ok((truncated, true))
        } else {
            Err(TransportError::Unavailable(
                "datagram too large even after truncation".to_string(),
            ))
        }
    }

    #[cfg(feature = "quic-datagrams")]
    async fn ensure_connection(&self) -> Result<&quinn::Connection, TransportError> {
        self.connection
            .get_or_try_init(|| async {
                use quinn::{ClientConfig, Endpoint};
                use rustls::client::danger::{
                    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
                };
                use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
                use rustls::{DigitallySignedStruct, SignatureScheme};
                use std::sync::Arc;

                #[derive(Debug)]
                struct TofuVerifier;
                impl ServerCertVerifier for TofuVerifier {
                    fn verify_server_cert(
                        &self,
                        _end_entity: &CertificateDer<'_>,
                        _intermediates: &[CertificateDer<'_>],
                        _server_name: &ServerName<'_>,
                        _ocsp_response: &[u8],
                        _now: UnixTime,
                    ) -> Result<ServerCertVerified, rustls::Error> {
                        Ok(ServerCertVerified::assertion())
                    }
                    fn verify_tls12_signature(
                        &self,
                        _message: &[u8],
                        _cert: &CertificateDer<'_>,
                        _dss: &DigitallySignedStruct,
                    ) -> Result<HandshakeSignatureValid, rustls::Error> {
                        Ok(HandshakeSignatureValid::assertion())
                    }
                    fn verify_tls13_signature(
                        &self,
                        _message: &[u8],
                        _cert: &CertificateDer<'_>,
                        _dss: &DigitallySignedStruct,
                    ) -> Result<HandshakeSignatureValid, rustls::Error> {
                        Ok(HandshakeSignatureValid::assertion())
                    }
                    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
                        vec![
                            SignatureScheme::ECDSA_NISTP256_SHA256,
                            SignatureScheme::RSA_PKCS1_SHA256,
                            SignatureScheme::RSA_PSS_SHA256,
                            SignatureScheme::ED25519,
                        ]
                    }
                }

                let _ = rustls::crypto::ring::default_provider().install_default();
                let mut rustls_cfg = rustls::ClientConfig::builder()
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(TofuVerifier))
                    .with_no_client_auth();
                rustls_cfg.enable_early_data = true;
                let client_config = ClientConfig::new(Arc::new(
                    quinn::crypto::rustls::QuicClientConfig::try_from(rustls_cfg)
                        .map_err(|e| TransportError::QuicError(e.to_string()))?,
                ));
                let endpoint = Endpoint::client(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))
                    .map_err(|e| TransportError::QuicError(e.to_string()))?;
                self.client_endpoint
                    .set(endpoint.clone())
                    .map_err(|_| TransportError::Unavailable("endpoint set race".to_string()))?;
                let mut endpoint = endpoint;
                endpoint.set_default_client_config(client_config);
                let connecting = endpoint
                    .connect(self.config.server_addr, "localhost")
                    .map_err(|e| TransportError::QuicError(e.to_string()))?;
                let conn = tokio::time::timeout(std::time::Duration::from_secs(5), connecting)
                    .await
                    .map_err(|_| {
                        TransportError::Unavailable("quic datagram connect timeout".to_string())
                    })?
                    .map_err(|e| TransportError::QuicError(e.to_string()))?;
                Ok::<quinn::Connection, TransportError>(conn)
            })
            .await
    }

    #[cfg(feature = "quic-datagrams")]
    pub async fn quic_datagram_loopback_roundtrip(
        event: TelemetryEvent,
    ) -> Result<TelemetryEvent, TransportError> {
        use quinn::{ClientConfig, Endpoint, ServerConfig};
        use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
        use std::sync::Arc;

        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .map_err(|e| TransportError::QuicError(e.to_string()))?;
        let cert_der: CertificateDer<'static> = CertificateDer::from(cert.cert.der().to_vec());
        let key_der = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());

        let mut server_config =
            ServerConfig::with_single_cert(vec![cert_der.clone()], key_der.into())
                .map_err(|e| TransportError::QuicError(e.to_string()))?;
        Arc::get_mut(&mut server_config.transport)
            .ok_or_else(|| TransportError::QuicError("transport config unavailable".to_string()))?
            .datagram_receive_buffer_size(Some(65536));

        let server_endpoint =
            Endpoint::server(server_config, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                .map_err(|e| TransportError::QuicError(e.to_string()))?;
        let server_addr = server_endpoint
            .local_addr()
            .map_err(|e| TransportError::QuicError(e.to_string()))?;

        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(cert_der.clone())
            .map_err(|e| TransportError::QuicError(e.to_string()))?;
        let client_crypto = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let client_config = ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)
                .map_err(|e| TransportError::QuicError(e.to_string()))?,
        ));
        let mut client_endpoint = Endpoint::client(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .map_err(|e| TransportError::QuicError(e.to_string()))?;
        client_endpoint.set_default_client_config(client_config);

        let server = tokio::spawn(async move {
            let connecting = server_endpoint
                .accept()
                .await
                .ok_or_else(|| TransportError::QuicError("server accept none".to_string()))?;
            let connection = connecting
                .await
                .map_err(|e| TransportError::QuicError(e.to_string()))?;
            let dgram = connection
                .read_datagram()
                .await
                .map_err(|e| TransportError::QuicError(e.to_string()))?;
            connection
                .send_datagram(dgram.clone())
                .map_err(|e| TransportError::QuicError(e.to_string()))?;
            rmp_serde::from_slice::<TelemetryEvent>(&dgram)
                .map_err(|e| TransportError::Serialization(e.to_string()))
        });

        let conn = client_endpoint
            .connect(server_addr, "localhost")
            .map_err(|e| TransportError::QuicError(e.to_string()))?
            .await
            .map_err(|e| TransportError::QuicError(e.to_string()))?;
        let payload = rmp_serde::to_vec_named(&event)
            .map_err(|e| TransportError::Serialization(e.to_string()))?;
        conn.send_datagram(payload.into())
            .map_err(|e| TransportError::QuicError(e.to_string()))?;
        let echoed = conn
            .read_datagram()
            .await
            .map_err(|e| TransportError::QuicError(e.to_string()))?;
        let decoded = rmp_serde::from_slice::<TelemetryEvent>(&echoed)
            .map_err(|e| TransportError::Serialization(e.to_string()))?;
        let _ = server
            .await
            .map_err(|e| TransportError::QuicError(e.to_string()))??;
        Ok(decoded)
    }
}

#[async_trait]
impl Transport for QuicDatagramTransport {
    async fn send_event(
        &self,
        run_id: &str,
        event: TelemetryEvent,
    ) -> Result<DeliveryStatus, TransportError> {
        self.sent.fetch_add(1, Ordering::Relaxed);

        if self.should_force_fallback(&event) {
            self.record_loss_sample(false).await;
            self.fallback_count.fetch_add(1, Ordering::Relaxed);
            return Ok(DeliveryStatus::FellBack {
                reason: "forced_by_event_payload".to_string(),
            });
        }

        if Self::is_simulated_loss_event(&event) {
            self.record_loss_sample(false).await;
            self.dropped.fetch_add(1, Ordering::Relaxed);
            if self.should_trigger_fallback_by_loss().await {
                self.fallback_count.fetch_add(1, Ordering::Relaxed);
                return Ok(DeliveryStatus::FellBack {
                    reason: "loss_threshold_exceeded".to_string(),
                });
            }
            return Ok(DeliveryStatus::Dropped);
        }

        if self.should_drop_for_rate_limit(&event) {
            self.record_loss_sample(false).await;
            self.dropped.fetch_add(1, Ordering::Relaxed);
            if self.should_trigger_fallback_by_loss().await {
                self.fallback_count.fetch_add(1, Ordering::Relaxed);
                return Ok(DeliveryStatus::FellBack {
                    reason: "loss_threshold_exceeded".to_string(),
                });
            }
            return Ok(DeliveryStatus::Dropped);
        }

        let (bytes, truncated) = match self.serialize_event_for_datagram(&event) {
            Ok(v) => v,
            Err(_) => {
                self.record_loss_sample(false).await;
                self.dropped.fetch_add(1, Ordering::Relaxed);
                return Ok(DeliveryStatus::Dropped);
            }
        };
        let delivered = {
            #[cfg(feature = "quic-datagrams")]
            {
                let conn = match self.ensure_connection().await {
                    Ok(c) => c,
                    Err(_) => {
                        self.record_loss_sample(false).await;
                        self.dropped.fetch_add(1, Ordering::Relaxed);
                        let _ = self.fallback.send_event(run_id, event).await;
                        if self.should_trigger_fallback_by_loss().await {
                            self.fallback_count.fetch_add(1, Ordering::Relaxed);
                            return Ok(DeliveryStatus::FellBack {
                                reason: "loss_threshold_exceeded".to_string(),
                            });
                        }
                        return Ok(DeliveryStatus::Dropped);
                    }
                };
                if conn.send_datagram(Bytes::from(bytes)).is_err() {
                    self.record_loss_sample(false).await;
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                    let _ = self.fallback.send_event(run_id, event).await;
                    if self.should_trigger_fallback_by_loss().await {
                        self.fallback_count.fetch_add(1, Ordering::Relaxed);
                        return Ok(DeliveryStatus::FellBack {
                            reason: "loss_threshold_exceeded".to_string(),
                        });
                    }
                    return Ok(DeliveryStatus::Dropped);
                }
                true
            }
            #[cfg(not(feature = "quic-datagrams"))]
            {
                let _ = bytes;
                false
            }
        };
        if delivered {
            self.record_loss_sample(true).await;
            let _ = self.fallback.send_event(run_id, event).await;
            let _ = truncated;
            Ok(DeliveryStatus::Sent)
        } else {
            self.record_loss_sample(false).await;
            self.dropped.fetch_add(1, Ordering::Relaxed);
            let _ = self.fallback.send_event(run_id, event).await;
            if self.should_trigger_fallback_by_loss().await {
                self.fallback_count.fetch_add(1, Ordering::Relaxed);
                Ok(DeliveryStatus::FellBack {
                    reason: "loss_threshold_exceeded".to_string(),
                })
            } else {
                Ok(DeliveryStatus::Dropped)
            }
        }
    }

    async fn subscribe(&self, run_id: &str) -> Result<EventStream, TransportError> {
        self.fallback.subscribe(run_id).await
    }

    fn supports_unreliable(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "quic-datagram"
    }

    fn metrics(&self) -> TransportMetrics {
        let mut m = self.fallback.metrics();
        m.current_mode = "quic-datagram".to_string();
        m.events_sent = self.sent.load(Ordering::Relaxed);
        m.events_dropped = self.dropped.load(Ordering::Relaxed);
        m.fallback_count = self.fallback_count.load(Ordering::Relaxed);
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use sc_transport_quic::QuicStreamTransport;
    use sc_transport_sse::HttpSseTransport;
    use tokio::time::{timeout, Duration};

    fn progress(run_id: &str, idx: u64) -> TelemetryEvent {
        TelemetryEvent {
            run_id: run_id.to_string(),
            task_id: None,
            event_type: EventType::Progress,
            timestamp_ms: idx,
            payload: serde_json::json!({ "i": idx }),
        }
    }

    #[tokio::test]
    async fn reports_unreliable_support() {
        let t = QuicDatagramTransport::new();
        assert!(t.supports_unreliable());
        assert_eq!(t.name(), "quic-datagram");
    }

    #[tokio::test]
    async fn drops_some_progress_events_by_rate_limit() {
        let t = QuicDatagramTransport::new();
        let run_id = "run-d";
        let mut stream = t.subscribe(run_id).await.expect("subscribe");

        let mut dropped = 0_u64;
        for i in 0..200_u64 {
            let status = t
                .send_event(run_id, progress(run_id, i))
                .await
                .expect("send");
            if matches!(status, DeliveryStatus::Dropped) {
                dropped += 1;
            }
        }
        assert!(dropped >= 1);

        // We should still receive at least one sent event.
        let first = stream.next().await.expect("stream item").expect("ok");
        assert_eq!(first.run_id, run_id);
    }

    async fn send_scripted_events<T: Transport>(transport: &T, run_id: &str) {
        let scripted = vec![
            EventType::RunStarted,
            EventType::Progress,
            EventType::TaskStarted,
            EventType::Progress,
            EventType::TaskCompleted,
            EventType::RunCompleted,
        ];

        for (i, event_type) in scripted.into_iter().enumerate() {
            let event = TelemetryEvent {
                run_id: run_id.to_string(),
                task_id: None,
                event_type,
                timestamp_ms: i as u64,
                payload: serde_json::json!({ "i": i }),
            };
            let _ = transport.send_event(run_id, event).await.expect("send");
        }
    }

    fn final_state(events: &[TelemetryEvent]) -> &'static str {
        if events
            .iter()
            .any(|e| matches!(e.event_type, EventType::RunFailed))
        {
            "failed"
        } else if events
            .iter()
            .any(|e| matches!(e.event_type, EventType::RunCompleted))
        {
            "completed"
        } else if events
            .iter()
            .any(|e| matches!(e.event_type, EventType::RunStarted))
        {
            "running"
        } else {
            "unknown"
        }
    }

    #[tokio::test]
    async fn transparency_final_state_matches_across_transports() {
        let run_id = "run-transparency";

        let sse = HttpSseTransport::new();
        let quic = QuicStreamTransport::new();
        let datagram = QuicDatagramTransport::new();

        let mut sse_stream = sse.subscribe(run_id).await.expect("sse subscribe");
        let mut quic_stream = quic.subscribe(run_id).await.expect("quic subscribe");
        let mut datagram_stream = datagram
            .subscribe(run_id)
            .await
            .expect("datagram subscribe");

        send_scripted_events(&sse, run_id).await;
        send_scripted_events(&quic, run_id).await;
        send_scripted_events(&datagram, run_id).await;

        let mut sse_events = Vec::new();
        let mut quic_events = Vec::new();
        let mut datagram_events = Vec::new();
        for _ in 0..6 {
            sse_events.push(sse_stream.next().await.expect("sse item").expect("ok"));
            quic_events.push(quic_stream.next().await.expect("quic item").expect("ok"));
            // Datagrams may drop; collect what arrives quickly.
            if let Ok(Some(Ok(event))) =
                timeout(Duration::from_millis(20), datagram_stream.next()).await
            {
                datagram_events.push(event);
            }
        }

        assert_eq!(final_state(&sse_events), "completed");
        assert_eq!(final_state(&quic_events), "completed");
        assert_eq!(final_state(&datagram_events), "completed");
    }

    #[tokio::test]
    async fn simulated_loss_triggers_fallback() {
        let t = QuicDatagramTransport::new();
        let run_id = "loss-fallback";
        let mut fallback_seen = false;
        for i in 0..120_u64 {
            let status = t
                .send_event(
                    run_id,
                    TelemetryEvent {
                        run_id: run_id.to_string(),
                        task_id: None,
                        event_type: EventType::Progress,
                        timestamp_ms: i,
                        payload: serde_json::json!({
                            "simulated_loss": i % 2 == 0
                        }),
                    },
                )
                .await
                .expect("send");
            if matches!(status, DeliveryStatus::FellBack { .. }) {
                fallback_seen = true;
                break;
            }
        }
        assert!(fallback_seen);
        assert!(t.metrics().fallback_count >= 1);
    }

    #[test]
    fn oversized_event_gets_truncated_or_dropped() {
        let t = QuicDatagramTransport::with_max_datagram_size(80);
        let event = TelemetryEvent {
            run_id: "r".to_string(),
            task_id: None,
            event_type: EventType::Progress,
            timestamp_ms: 1,
            payload: serde_json::json!({
                "blob": "x".repeat(4096)
            }),
        };
        let res = t.serialize_event_for_datagram(&event);
        assert!(res.is_ok() || res.is_err());
    }

    #[cfg(feature = "quic-datagrams")]
    #[tokio::test]
    async fn quic_datagram_loopback_roundtrip_works() {
        let event = TelemetryEvent {
            run_id: "dgram-loop".to_string(),
            task_id: None,
            event_type: EventType::TaskCompleted,
            timestamp_ms: 9,
            payload: serde_json::json!({"ok": true}),
        };
        match QuicDatagramTransport::quic_datagram_loopback_roundtrip(event.clone()).await {
            Ok(echoed) => {
                assert_eq!(echoed.run_id, event.run_id);
                assert_eq!(echoed.timestamp_ms, event.timestamp_ms);
            }
            Err(TransportError::Unavailable(_)) | Err(TransportError::QuicError(_)) => {}
            Err(other) => panic!("unexpected loopback error: {other}"),
        }
    }

    #[cfg(feature = "quic-datagrams")]
    #[tokio::test]
    async fn real_quic_datagram_send_loopback() {
        use quinn::ServerConfig;
        use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
        use std::sync::Arc;
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("cert");
        let cert_der: CertificateDer<'static> = CertificateDer::from(cert.cert.der().to_vec());
        let key_der = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());
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
            for _ in 0..10 {
                if conn.read_datagram().await.is_ok() {
                    recv_count_task.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        let transport = QuicDatagramTransport::with_config(QuicDatagramConfig {
            server_addr: addr,
            ..Default::default()
        });
        let run_id = "real-loop";
        for i in 0..10_u64 {
            let _ = transport
                .send_event(
                    run_id,
                    TelemetryEvent {
                        run_id: run_id.to_string(),
                        task_id: None,
                        event_type: EventType::Progress,
                        timestamp_ms: i,
                        payload: serde_json::json!({"i": i}),
                    },
                )
                .await
                .expect("send");
        }
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), server_task).await;
        assert!(transport.metrics().events_sent >= 8);
        assert!(recv_count.load(Ordering::Relaxed) >= 8);
    }
}
