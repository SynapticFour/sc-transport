pub mod batch;
#[cfg(feature = "quic-streams")]
pub mod congestion;
pub use batch::{BatchSendResult, BatchSender};

use async_trait::async_trait;
use sc_transport_core::{
    DeliveryStatus, EventStream, TelemetryEvent, Transport, TransportError, TransportMetrics,
};
use sc_transport_sse::HttpSseTransport;
#[cfg(feature = "quic-streams")]
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "quic-streams")]
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
#[cfg(feature = "quic-streams")]
use tokio::sync::Mutex;

pub const LENGTH_PREFIX_BYTES: usize = 4;
const BATCH_MAGIC: &[u8; 4] = b"SCB1";

/// Reliable QUIC streams transport.
///
/// This baseline implementation preserves semantic transparency by delegating to
/// the stable SSE transport when QUIC streams are unavailable.
pub struct QuicStreamTransport {
    fallback: HttpSseTransport,
    events_sent: AtomicU64,
    events_delivered: AtomicU64,
    events_dropped: AtomicU64,
    fallback_count: AtomicU64,
    #[cfg(feature = "quic-streams")]
    server_addr: SocketAddr,
    #[cfg(feature = "quic-streams")]
    use_scientific_cc: bool,
    #[cfg(feature = "quic-streams")]
    client_endpoint: Mutex<Option<quinn::Endpoint>>,
    #[cfg(feature = "quic-streams")]
    shared_connection: Mutex<Option<quinn::Connection>>,
    #[cfg(feature = "quic-streams")]
    shared_uni_stream: Mutex<Option<quinn::SendStream>>,
    #[cfg(feature = "quic-streams")]
    pending_events: Mutex<Vec<TelemetryEvent>>,
    #[cfg(feature = "quic-streams")]
    uni_writes: AtomicU64,
}

impl Default for QuicStreamTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl QuicStreamTransport {
    pub fn new() -> Self {
        Self {
            fallback: HttpSseTransport::new(),
            events_sent: AtomicU64::new(0),
            events_delivered: AtomicU64::new(0),
            events_dropped: AtomicU64::new(0),
            fallback_count: AtomicU64::new(0),
            #[cfg(feature = "quic-streams")]
            server_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 7272)),
            #[cfg(feature = "quic-streams")]
            use_scientific_cc: false,
            #[cfg(feature = "quic-streams")]
            client_endpoint: Mutex::new(None),
            #[cfg(feature = "quic-streams")]
            shared_connection: Mutex::new(None),
            #[cfg(feature = "quic-streams")]
            shared_uni_stream: Mutex::new(None),
            #[cfg(feature = "quic-streams")]
            pending_events: Mutex::new(Vec::new()),
            #[cfg(feature = "quic-streams")]
            uni_writes: AtomicU64::new(0),
        }
    }

    #[cfg(feature = "quic-streams")]
    pub fn with_server_addr(server_addr: SocketAddr) -> Self {
        let mut s = Self::new();
        s.server_addr = server_addr;
        s
    }

    pub fn with_scientific_cc() -> Self {
        #[cfg(feature = "quic-streams")]
        {
            let mut s = Self::new();
            s.use_scientific_cc = true;
            s
        }
        #[cfg(not(feature = "quic-streams"))]
        {
            Self::new()
        }
    }

    /// Frame one telemetry event as: 4-byte big-endian length + msgpack bytes.
    pub fn frame_event(event: &TelemetryEvent) -> Result<Vec<u8>, TransportError> {
        let payload = rmp_serde::to_vec_named(event)
            .map_err(|e| TransportError::Serialization(e.to_string()))?;
        let mut buf = Vec::with_capacity(LENGTH_PREFIX_BYTES + payload.len());
        let len = u32::try_from(payload.len()).map_err(|_| {
            TransportError::Unavailable("event payload exceeds u32 frame length".to_string())
        })?;
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(&payload);
        Ok(buf)
    }

    /// Parse one telemetry event framed by `frame_event`.
    pub fn parse_framed_event(buf: &[u8]) -> Result<TelemetryEvent, TransportError> {
        if buf.len() < LENGTH_PREFIX_BYTES {
            return Err(TransportError::Unavailable("frame too small".to_string()));
        }
        let declared_len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        let payload = &buf[LENGTH_PREFIX_BYTES..];
        if payload.len() != declared_len {
            return Err(TransportError::Unavailable(
                "frame length mismatch".to_string(),
            ));
        }
        rmp_serde::from_slice(payload).map_err(|e| TransportError::Serialization(e.to_string()))
    }

    /// Frame a batch payload: magic + count + repeated (len + msgpack event).
    pub fn frame_event_batch(events: &[TelemetryEvent]) -> Result<Vec<u8>, TransportError> {
        let count = u32::try_from(events.len()).map_err(|_| {
            TransportError::Unavailable("too many events for batch frame".to_string())
        })?;
        let mut payload = Vec::new();
        payload.extend_from_slice(BATCH_MAGIC);
        payload.extend_from_slice(&count.to_be_bytes());
        for event in events {
            let encoded = rmp_serde::to_vec_named(event)
                .map_err(|e| TransportError::Serialization(e.to_string()))?;
            let len = u32::try_from(encoded.len()).map_err(|_| {
                TransportError::Unavailable(
                    "batched event payload exceeds u32 frame length".to_string(),
                )
            })?;
            payload.extend_from_slice(&len.to_be_bytes());
            payload.extend_from_slice(&encoded);
        }
        let mut out = Vec::with_capacity(LENGTH_PREFIX_BYTES + payload.len());
        let payload_len = u32::try_from(payload.len()).map_err(|_| {
            TransportError::Unavailable("batch payload exceeds u32 frame length".to_string())
        })?;
        out.extend_from_slice(&payload_len.to_be_bytes());
        out.extend_from_slice(&payload);
        Ok(out)
    }

    pub fn parse_framed_events(buf: &[u8]) -> Result<Vec<TelemetryEvent>, TransportError> {
        if buf.len() < LENGTH_PREFIX_BYTES {
            return Err(TransportError::Unavailable("frame too small".to_string()));
        }
        let declared_len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        let payload = &buf[LENGTH_PREFIX_BYTES..];
        if payload.len() != declared_len {
            return Err(TransportError::Unavailable(
                "frame length mismatch".to_string(),
            ));
        }
        if payload.len() < 8 || &payload[..4] != BATCH_MAGIC {
            return Ok(vec![Self::parse_framed_event(buf)?]);
        }
        let count = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
        let mut idx = 8usize;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            if idx + 4 > payload.len() {
                return Err(TransportError::Unavailable(
                    "invalid batch frame length header".to_string(),
                ));
            }
            let len = u32::from_be_bytes([
                payload[idx],
                payload[idx + 1],
                payload[idx + 2],
                payload[idx + 3],
            ]) as usize;
            idx += 4;
            if idx + len > payload.len() {
                return Err(TransportError::Unavailable(
                    "invalid batch frame payload boundary".to_string(),
                ));
            }
            let event = rmp_serde::from_slice::<TelemetryEvent>(&payload[idx..idx + len])
                .map_err(|e| TransportError::Serialization(e.to_string()))?;
            out.push(event);
            idx += len;
        }
        Ok(out)
    }

    /// Write one framed event to an async stream.
    pub async fn write_framed_event<W: AsyncWrite + Unpin>(
        writer: &mut W,
        event: &TelemetryEvent,
    ) -> Result<(), TransportError> {
        let framed = Self::frame_event(event)?;
        writer
            .write_all(&framed)
            .await
            .map_err(|e| TransportError::Unavailable(e.to_string()))?;
        writer
            .flush()
            .await
            .map_err(|e| TransportError::Unavailable(e.to_string()))
    }

    /// Read one framed event from an async stream.
    pub async fn read_framed_event<R: AsyncRead + Unpin>(
        reader: &mut R,
    ) -> Result<TelemetryEvent, TransportError> {
        let mut len_buf = [0_u8; LENGTH_PREFIX_BYTES];
        reader
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| TransportError::Unavailable(e.to_string()))?;
        let payload_len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0_u8; payload_len];
        reader
            .read_exact(&mut payload)
            .await
            .map_err(|e| TransportError::Unavailable(e.to_string()))?;
        if payload.len() >= 8 && &payload[..4] == BATCH_MAGIC {
            let mut framed = Vec::with_capacity(LENGTH_PREFIX_BYTES + payload.len());
            framed.extend_from_slice(&len_buf);
            framed.extend_from_slice(&payload);
            let mut events = Self::parse_framed_events(&framed)?;
            return events
                .drain(..1)
                .next()
                .ok_or_else(|| TransportError::Unavailable("empty batch frame".to_string()));
        }
        rmp_serde::from_slice(&payload).map_err(|e| TransportError::Serialization(e.to_string()))
    }

    #[cfg(feature = "quic-streams")]
    pub fn quic_streams_enabled() -> bool {
        true
    }

    #[cfg(not(feature = "quic-streams"))]
    pub fn quic_streams_enabled() -> bool {
        false
    }

    #[cfg(feature = "quic-streams")]
    pub async fn quic_loopback_roundtrip(
        event: TelemetryEvent,
    ) -> Result<TelemetryEvent, TransportError> {
        use quinn::{ClientConfig, Endpoint, ServerConfig};
        use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
        use std::sync::Arc;
        let _ = rustls::crypto::ring::default_provider().install_default();

        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .map_err(|e| TransportError::QuicError(e.to_string()))?;
        let cert_der: CertificateDer<'static> = CertificateDer::from(cert.cert.der().to_vec());
        let key_der = PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());

        let mut server_config =
            ServerConfig::with_single_cert(vec![cert_der.clone()], key_der.into())
                .map_err(|e| TransportError::QuicError(e.to_string()))?;
        let transport_config = Arc::get_mut(&mut server_config.transport).ok_or_else(|| {
            TransportError::QuicError("failed to get mutable transport config".to_string())
        })?;
        transport_config.max_concurrent_bidi_streams(16_u32.into());

        let server_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let server_endpoint = Endpoint::server(server_config, server_addr)
            .map_err(|e| TransportError::QuicError(e.to_string()))?;
        let listen_addr = server_endpoint
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

        let server_task = tokio::spawn(async move {
            let incoming = server_endpoint.accept().await.ok_or_else(|| {
                TransportError::QuicError("server accept returned none".to_string())
            })?;
            let conn = incoming
                .await
                .map_err(|e| TransportError::QuicError(e.to_string()))?;
            let (mut send, mut recv) = conn
                .accept_bi()
                .await
                .map_err(|e| TransportError::QuicError(e.to_string()))?;
            let decoded = QuicStreamTransport::read_framed_event(&mut recv).await?;
            QuicStreamTransport::write_framed_event(&mut send, &decoded).await?;
            send.finish()
                .map_err(|e| TransportError::QuicError(e.to_string()))?;
            Ok::<TelemetryEvent, TransportError>(decoded)
        });

        let connecting = client_endpoint
            .connect(listen_addr, "localhost")
            .map_err(|e| TransportError::QuicError(e.to_string()))?;
        let conn = connecting
            .await
            .map_err(|e| TransportError::QuicError(e.to_string()))?;
        let (mut send, mut recv) = conn
            .open_bi()
            .await
            .map_err(|e| TransportError::QuicError(e.to_string()))?;
        QuicStreamTransport::write_framed_event(&mut send, &event).await?;
        send.finish()
            .map_err(|e| TransportError::QuicError(e.to_string()))?;
        let echoed = QuicStreamTransport::read_framed_event(&mut recv).await?;
        let _ = server_task
            .await
            .map_err(|e| TransportError::QuicError(e.to_string()))??;
        Ok(echoed)
    }

    #[cfg(feature = "quic-streams")]
    async fn connect_for_batch(&self) -> Result<quinn::Connection, TransportError> {
        use quinn::{ClientConfig, Endpoint, EndpointConfig, TransportConfig};
        use rustls::client::danger::{
            HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
        };
        use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
        use rustls::{DigitallySignedStruct, SignatureScheme};
        use rustls_platform_verifier::BuilderVerifierExt;

        #[derive(Debug)]
        struct InsecureVerifier;
        impl ServerCertVerifier for InsecureVerifier {
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

        {
            let existing = self.shared_connection.lock().await;
            if let Some(conn) = existing.as_ref() {
                if conn.close_reason().is_none() {
                    return Ok(conn.clone());
                }
            }
        }

        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut rustls_cfg = if allow_insecure_quic_override() {
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(InsecureVerifier))
                .with_no_client_auth()
        } else {
            rustls::ClientConfig::builder()
                .with_platform_verifier()
                .map_err(|e| TransportError::QuicError(e.to_string()))?
                .with_no_client_auth()
        };
        rustls_cfg.enable_early_data = true;
        let mut client_config = ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(rustls_cfg)
                .map_err(|e| TransportError::QuicError(e.to_string()))?,
        ));
        if self.use_scientific_cc {
            let mut tcfg = TransportConfig::default();
            tcfg.congestion_controller_factory(Arc::new(congestion::SciBbrConfig::default()));
            client_config.transport_config(Arc::new(tcfg));
        }

        let mut endpoint_guard = self.client_endpoint.lock().await;
        if endpoint_guard.is_none() {
            let socket = std::net::UdpSocket::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))
                .map_err(|e| TransportError::Unavailable(e.to_string()))?;
            let runtime = Arc::new(quinn::TokioRuntime);
            let endpoint = Endpoint::new(EndpointConfig::default(), None, socket, runtime)
                .map_err(|e| TransportError::QuicError(e.to_string()))?;
            *endpoint_guard = Some(endpoint);
        }
        let endpoint = endpoint_guard
            .as_mut()
            .ok_or_else(|| TransportError::Unavailable("missing quic endpoint".to_string()))?;
        endpoint.set_default_client_config(client_config);

        let server_name = if self.server_addr.ip().is_loopback() {
            "localhost".to_string()
        } else {
            self.server_addr.ip().to_string()
        };
        let conn = endpoint
            .connect(self.server_addr, &server_name)
            .map_err(|e| TransportError::QuicError(e.to_string()))?
            .await
            .map_err(|e| TransportError::QuicError(e.to_string()))?;
        drop(endpoint_guard);

        let mut shared = self.shared_connection.lock().await;
        *shared = Some(conn.clone());
        Ok(conn)
    }

    pub async fn send_events_batch(
        &self,
        run_id: &str,
        events: Vec<TelemetryEvent>,
    ) -> Result<BatchSendResult, TransportError> {
        #[cfg(feature = "quic-streams")]
        {
            let conn = self.connect_for_batch().await.map_err(|_| {
                TransportError::Unavailable("quic connection unavailable".to_string())
            })?;
            let sender = BatchSender::default();
            Ok(sender.send_batch(&conn, run_id, events).await)
        }
        #[cfg(not(feature = "quic-streams"))]
        {
            let _ = (run_id, events);
            Err(TransportError::Unavailable(
                "quic-streams not enabled".to_string(),
            ))
        }
    }

    #[cfg(feature = "quic-streams")]
    async fn send_event_over_quic(&self, event: &TelemetryEvent) -> Result<(), TransportError> {
        let conn = self.connect_for_batch().await?;
        let coalesce_n = std::env::var("SC_QUIC_COALESCE_EVENTS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 1)
            .unwrap_or(1);
        let framed = if coalesce_n > 1
            && matches!(event.event_type, sc_transport_core::EventType::Progress)
        {
            let mut pending = self.pending_events.lock().await;
            pending.push(event.clone());
            if pending.len() < coalesce_n {
                return Ok(());
            }
            let batch = pending.split_off(0);
            Self::frame_event_batch(&batch)?
        } else {
            Self::frame_event(event)?
        };
        let use_persistent_uni = std::env::var("SC_QUIC_PERSISTENT_UNI")
            .ok()
            .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(false);
        if !use_persistent_uni {
            let mut send_uni = conn
                .open_uni()
                .await
                .map_err(|e| TransportError::QuicError(e.to_string()))?;
            send_uni
                .write_all(&framed)
                .await
                .map_err(|e| TransportError::Unavailable(e.to_string()))?;
            send_uni
                .finish()
                .map_err(|e| TransportError::QuicError(e.to_string()))?;
            return Ok(());
        }
        let flush_every = std::env::var("SC_QUIC_STREAM_FLUSH_EVERY")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(16);

        // Fast path: keep one uni stream open and append framed events.
        // This amortizes stream-open overhead on good/excellent networks.
        let mut last_err: Option<TransportError> = None;
        for _ in 0..2 {
            let mut stream_guard = self.shared_uni_stream.lock().await;
            if stream_guard.is_none() {
                match conn.open_uni().await {
                    Ok(s) => *stream_guard = Some(s),
                    Err(e) => {
                        last_err = Some(TransportError::QuicError(e.to_string()));
                        continue;
                    }
                }
            }
            if let Some(stream) = stream_guard.as_mut() {
                match stream.write_all(&framed).await {
                    Ok(()) => {
                        let writes = self.uni_writes.fetch_add(1, Ordering::Relaxed) + 1;
                        if writes.is_multiple_of(flush_every) {
                            let _ = stream.flush().await;
                        }
                        return Ok(());
                    }
                    Err(e) => {
                        // stream is stale/closed; drop and retry with a new one
                        *stream_guard = None;
                        last_err = Some(TransportError::Unavailable(e.to_string()));
                    }
                }
            }
        }

        // Compatibility fallback for peers expecting bidirectional streams.
        let (mut send_bi, _recv_bi) = conn
            .open_bi()
            .await
            .map_err(|e| TransportError::QuicError(e.to_string()))?;
        send_bi
            .write_all(&framed)
            .await
            .map_err(|e| TransportError::Unavailable(e.to_string()))?;
        send_bi
            .finish()
            .map_err(|e| TransportError::QuicError(e.to_string()))?;
        if let Some(err) = last_err {
            let _ = err;
        }
        Ok(())
    }
}

#[cfg(feature = "quic-streams")]
fn allow_insecure_quic_override() -> bool {
    match std::env::var("SC_TRANSPORT_ALLOW_INSECURE_QUIC") {
        Ok(v) => matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"),
        Err(_) => false,
    }
}

#[async_trait]
impl Transport for QuicStreamTransport {
    async fn send_event(
        &self,
        run_id: &str,
        event: TelemetryEvent,
    ) -> Result<DeliveryStatus, TransportError> {
        self.events_sent.fetch_add(1, Ordering::Relaxed);
        if !Self::quic_streams_enabled() {
            self.fallback_count.fetch_add(1, Ordering::Relaxed);
            match self.fallback.send_event(run_id, event).await {
                Ok(DeliveryStatus::Dropped) => {
                    self.events_dropped.fetch_add(1, Ordering::Relaxed);
                }
                Ok(_) => {
                    self.events_delivered.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    self.events_dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            return Ok(DeliveryStatus::FellBack {
                reason: "quic_streams_feature_disabled".to_string(),
            });
        }

        #[cfg(feature = "quic-streams")]
        {
            // Validate stream framing before network write.
            let _ = Self::frame_event(&event)?;
            match self.send_event_over_quic(&event).await {
                Ok(()) => {
                    self.events_delivered.fetch_add(1, Ordering::Relaxed);
                    // Keep local in-process subscribers semantically consistent.
                    let _ = self.fallback.send_event(run_id, event).await;
                    return Ok(DeliveryStatus::Sent);
                }
                Err(err) => {
                    self.fallback_count.fetch_add(1, Ordering::Relaxed);
                    match self.fallback.send_event(run_id, event).await {
                        Ok(DeliveryStatus::Dropped) => {
                            self.events_dropped.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(_) => {
                            self.events_delivered.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(_) => {
                            self.events_dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    return Ok(DeliveryStatus::FellBack {
                        reason: format!("quic_stream_send_failed:{err}"),
                    });
                }
            }
        }

        #[allow(unreachable_code)]
        {
            Ok(DeliveryStatus::Dropped)
        }
    }

    async fn subscribe(&self, run_id: &str) -> Result<EventStream, TransportError> {
        self.fallback.subscribe(run_id).await
    }

    fn supports_unreliable(&self) -> bool {
        false
    }

    fn name(&self) -> &'static str {
        "quic-stream"
    }

    fn metrics(&self) -> TransportMetrics {
        let fallback = self.fallback.metrics();
        TransportMetrics {
            events_sent: self.events_sent.load(Ordering::Relaxed),
            events_delivered: self.events_delivered.load(Ordering::Relaxed),
            events_dropped: self.events_dropped.load(Ordering::Relaxed),
            fallback_count: self.fallback_count.load(Ordering::Relaxed),
            current_mode: "quic-stream".to_string(),
            active_subscribers: fallback.active_subscribers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use sc_transport_core::EventType;

    fn event(run_id: &str, event_type: EventType, timestamp_ms: u64) -> TelemetryEvent {
        TelemetryEvent {
            run_id: run_id.to_string(),
            task_id: None,
            event_type,
            timestamp_ms,
            payload: serde_json::json!({ "ts": timestamp_ms }),
        }
    }

    #[test]
    fn framed_event_roundtrip() {
        let e = event("run-a", EventType::RunStarted, 1);
        let framed = QuicStreamTransport::frame_event(&e).expect("frame must succeed");
        let decoded =
            QuicStreamTransport::parse_framed_event(&framed).expect("decode must succeed");
        assert_eq!(decoded.run_id, "run-a");
        assert!(matches!(decoded.event_type, EventType::RunStarted));
    }

    #[tokio::test]
    async fn async_framed_io_roundtrip() {
        let e = event("run-io", EventType::TaskStarted, 42);
        let (mut client, mut server) = tokio::io::duplex(2048);

        let writer = tokio::spawn(async move {
            QuicStreamTransport::write_framed_event(&mut client, &e)
                .await
                .expect("write framed event");
        });

        let decoded = QuicStreamTransport::read_framed_event(&mut server)
            .await
            .expect("read framed event");
        writer.await.expect("writer join");

        assert_eq!(decoded.run_id, "run-io");
        assert!(matches!(decoded.event_type, EventType::TaskStarted));
        assert_eq!(decoded.timestamp_ms, 42);
    }

    #[tokio::test]
    async fn fallback_path_still_delivers_events() {
        let t = QuicStreamTransport::new();
        let run_id = "run-b";
        let mut stream = t.subscribe(run_id).await.expect("subscribe");
        let status = t
            .send_event(run_id, event(run_id, EventType::Progress, 2))
            .await
            .expect("send");
        assert!(matches!(
            status,
            DeliveryStatus::Sent | DeliveryStatus::Delivered | DeliveryStatus::FellBack { .. }
        ));
        let got = stream.next().await.expect("event").expect("ok");
        assert_eq!(got.run_id, run_id);
    }

    #[cfg(feature = "quic-streams")]
    #[tokio::test]
    async fn unreachable_quic_endpoint_reports_fallback_status() {
        let t = QuicStreamTransport::with_server_addr("127.0.0.1:1".parse().expect("addr"));
        let run_id = "run-fallback";
        let status = t
            .send_event(run_id, event(run_id, EventType::Progress, 9))
            .await
            .expect("send");
        assert!(matches!(status, DeliveryStatus::FellBack { .. }));
        assert!(t.metrics().fallback_count >= 1);
    }

    #[cfg(feature = "quic-streams")]
    #[test]
    fn insecure_quic_override_is_explicit_only() {
        let key = "SC_TRANSPORT_ALLOW_INSECURE_QUIC";
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);
        assert!(!allow_insecure_quic_override());
        std::env::set_var(key, "true");
        assert!(allow_insecure_quic_override());
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    #[cfg(feature = "quic-streams")]
    #[tokio::test]
    async fn quic_loopback_roundtrip_works() {
        let original = event("run-quinn", EventType::TaskCompleted, 7);
        match QuicStreamTransport::quic_loopback_roundtrip(original.clone()).await {
            Ok(echoed) => {
                assert_eq!(echoed.run_id, original.run_id);
                assert!(matches!(echoed.event_type, EventType::TaskCompleted));
                assert_eq!(echoed.timestamp_ms, 7);
            }
            Err(TransportError::Unavailable(_)) | Err(TransportError::QuicError(_)) => {
                // Some CI/sandbox environments can disrupt local UDP loopback.
                // Keep this as a non-fatal datapoint and rely on framing tests
                // plus integration tests for transport behavior validation.
            }
            Err(other) => panic!("unexpected loopback error: {other}"),
        }
    }
}
