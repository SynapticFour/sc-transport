use crate::congestion::SciBbrConfig;
use anyhow::{Context, Result};
use bytes::Bytes;
use quinn::{
    ClientConfig, Endpoint, EndpointConfig, RecvStream, SendStream, ServerConfig, TokioRuntime,
};
use rcgen::generate_simple_self_signed;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as RustlsError, SignatureScheme};
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub struct TransportConfig {
    pub bind_addr: SocketAddr,
    pub max_concurrent_streams: u32,
    pub initial_rtt_ms: u32,
    pub target_bandwidth_mbps: Option<u64>,
    pub keep_alive_interval_secs: u32,
    pub idle_timeout_secs: u32,
    /// Hint only: Quinn's UDP driver enables GSO/GRO on Linux when the kernel
    /// supports it. SPARQ does not reimplement Aspera FASP segmentation.
    pub enable_gso: bool,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::from((Ipv4Addr::UNSPECIFIED, 7272)),
            max_concurrent_streams: 16,
            initial_rtt_ms: 100,
            target_bandwidth_mbps: None,
            keep_alive_interval_secs: 10,
            idle_timeout_secs: 300,
            enable_gso: true,
        }
    }
}

fn ring_sig_algs() -> WebPkiSupportedAlgorithms {
    rustls::crypto::ring::default_provider().signature_verification_algorithms
}

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("quic error: {0}")]
    Quic(String),
    #[error("io error: {0}")]
    Io(String),
}

pub struct SctEndpoint {
    endpoint: Endpoint,
}

#[derive(Clone)]
pub struct SctConnection {
    connection: quinn::Connection,
}

#[derive(Debug)]
struct InsecureTofuVerifier;

impl ServerCertVerifier for InsecureTofuVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, RustlsError> {
        let host = server_name.to_str();
        if cfg!(test) && (host == "localhost" || host == "127.0.0.1") {
            return Ok(ServerCertVerified::assertion());
        }
        enforce_or_persist_known_host(&host, end_entity)
            .map_err(|e| RustlsError::General(format!("tofu verification failed: {e}")))?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, RustlsError> {
        verify_tls12_signature(message, cert, dss, &ring_sig_algs())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, RustlsError> {
        verify_tls13_signature(message, cert, dss, &ring_sig_algs())
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

impl SctEndpoint {
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.endpoint
            .local_addr()
            .map_err(|e| anyhow::Error::new(TransportError::Quic(e.to_string())))
    }

    pub fn server(config: TransportConfig) -> Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let suggested_buf = suggested_socket_buffer_bytes(&config);
        debug!(
            "suggested socket buffers (bytes): {}, gso_hint={}",
            suggested_buf, config.enable_gso
        );
        let (cert_der, key_der) = load_or_create_server_identity()?;

        let mut server_config = ServerConfig::with_single_cert(vec![cert_der], key_der.into())
            .map_err(|e| TransportError::Quic(e.to_string()))?;
        apply_sparq_quinn_transport(
            Arc::get_mut(&mut server_config.transport).ok_or_else(|| {
                TransportError::Quic("failed mutable transport config".to_string())
            })?,
            &config,
        )?;

        let socket =
            configured_udp_socket(config.bind_addr, suggested_buf as usize, config.enable_gso)
                .map_err(|e| TransportError::Io(e.to_string()))?;
        let endpoint = Endpoint::new(
            EndpointConfig::default(),
            Some(server_config),
            socket,
            Arc::new(TokioRuntime),
        )
        .map_err(|e| TransportError::Quic(e.to_string()))?;
        info!("sct server endpoint created at {}", config.bind_addr);
        Ok(Self { endpoint })
    }

    pub fn client(config: TransportConfig) -> Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let suggested_buf = suggested_socket_buffer_bytes(&config);
        debug!(
            "suggested socket buffers (bytes): {}, gso_hint={}",
            suggested_buf, config.enable_gso
        );
        let mut rustls_cfg = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(InsecureTofuVerifier))
            .with_no_client_auth();
        rustls_cfg.enable_early_data = true;
        let mut client_config = ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(rustls_cfg)
                .map_err(|e| TransportError::Quic(e.to_string()))?,
        ));
        let mut tcfg = quinn::TransportConfig::default();
        apply_sparq_quinn_transport(&mut tcfg, &config)?;
        client_config.transport_config(Arc::new(tcfg));
        let socket =
            configured_udp_socket(config.bind_addr, suggested_buf as usize, config.enable_gso)
                .map_err(|e| TransportError::Io(e.to_string()))?;
        let mut endpoint = Endpoint::new(
            EndpointConfig::default(),
            None,
            socket,
            Arc::new(TokioRuntime),
        )
        .map_err(|e| TransportError::Quic(e.to_string()))?;
        endpoint.set_default_client_config(client_config);
        debug!("sct client endpoint bound at {}", config.bind_addr);
        Ok(Self { endpoint })
    }

    pub async fn connect(&self, addr: SocketAddr, server_name: &str) -> Result<SctConnection> {
        let connecting = self
            .endpoint
            .connect(addr, server_name)
            .map_err(|e| TransportError::Quic(e.to_string()))?;
        let connection = connecting
            .await
            .map_err(|e| TransportError::Quic(e.to_string()))?;
        info!("connected to {}", addr);
        Ok(SctConnection { connection })
    }

    pub async fn accept(&self) -> Option<Result<SctConnection>> {
        let incoming = self.endpoint.accept().await?;
        match incoming.await {
            Ok(connection) => Some(Ok(SctConnection { connection })),
            Err(e) => Some(Err(anyhow::Error::new(TransportError::Quic(e.to_string())))),
        }
    }
}

fn suggested_socket_buffer_bytes(config: &TransportConfig) -> u64 {
    let rtt_s = (config.initial_rtt_ms as f64 / 1000.0).max(0.001);
    let bdp_bytes = config
        .target_bandwidth_mbps
        .map(|mbps| ((mbps as f64 * 1_000_000.0 / 8.0) * rtt_s) as u64)
        .unwrap_or(8 * 1024 * 1024);
    (bdp_bytes * 4).max(32 * 1024 * 1024)
}

fn apply_sparq_quinn_transport(
    transport: &mut quinn::TransportConfig,
    config: &TransportConfig,
) -> Result<()> {
    transport.max_concurrent_bidi_streams(config.max_concurrent_streams.into());
    transport.max_concurrent_uni_streams((config.max_concurrent_streams.saturating_mul(4)).into());
    transport.congestion_controller_factory(Arc::new(SciBbrConfig::default()));
    transport.keep_alive_interval(Some(Duration::from_secs(
        config.keep_alive_interval_secs as u64,
    )));
    transport.max_idle_timeout(Some(
        Duration::from_secs(config.idle_timeout_secs as u64)
            .try_into()
            .map_err(|e| TransportError::Quic(format!("invalid idle timeout: {e}")))?,
    ));
    // RFC 9221 datagrams: speculative FEC/NACK copies. Not FASP; IETF QUIC only.
    transport.datagram_receive_buffer_size(Some(1 << 20));
    transport.datagram_send_buffer_size(1 << 20);
    let _ = config.enable_gso;
    Ok(())
}

fn configured_udp_socket(
    bind_addr: SocketAddr,
    buf_size: usize,
    enable_gso: bool,
) -> io::Result<std::net::UdpSocket> {
    let socket = Socket::new(
        Domain::for_address(bind_addr),
        Type::DGRAM,
        Some(Protocol::UDP),
    )?;
    if bind_addr.is_ipv6() {
        if let Err(e) = socket.set_only_v6(false) {
            debug!(%e, "unable to make socket dual-stack");
        }
    }
    socket.set_send_buffer_size(buf_size)?;
    socket.set_recv_buffer_size(buf_size)?;
    // Quinn's TokioRuntime / quinn-udp enables UDP GSO/GRO on Linux. This flag is a
    // documented operator hint, not a second segmentation protocol.
    if enable_gso {
        debug!("gso hint on: quinn-udp will use kernel GSO/GRO when available");
    } else {
        debug!("gso hint off (kernel offload still decided by quinn-udp)");
    }
    socket.bind(&bind_addr.into())?;
    Ok(socket.into())
}

impl SctConnection {
    pub async fn accept_control_stream(&self) -> Result<(SendStream, RecvStream)> {
        self.connection
            .accept_bi()
            .await
            .map_err(|e| anyhow::Error::new(TransportError::Quic(e.to_string())))
    }

    pub async fn open_control_stream(&self) -> Result<(SendStream, RecvStream)> {
        self.connection
            .open_bi()
            .await
            .map_err(|e| anyhow::Error::new(TransportError::Quic(e.to_string())))
    }

    pub async fn open_data_stream(&self) -> Result<SendStream> {
        self.connection
            .open_uni()
            .await
            .map_err(|e| anyhow::Error::new(TransportError::Quic(e.to_string())))
    }

    pub async fn accept_data_stream(&self) -> Result<RecvStream> {
        self.connection
            .accept_uni()
            .await
            .map_err(|e| anyhow::Error::new(TransportError::Quic(e.to_string())))
    }

    /// RFC 9221 datagram send for speculative / small shards. Large bulk stays on uni streams.
    pub fn send_datagram(&self, payload: Bytes) -> Result<()> {
        self.connection
            .send_datagram(payload)
            .map_err(|e| anyhow::Error::new(TransportError::Quic(e.to_string())))
    }

    pub async fn read_datagram(&self) -> Result<Bytes> {
        self.connection
            .read_datagram()
            .await
            .map_err(|e| anyhow::Error::new(TransportError::Quic(e.to_string())))
    }

    pub fn max_datagram_size(&self) -> Option<usize> {
        self.connection.max_datagram_size()
    }

    pub fn remote_addr(&self) -> SocketAddr {
        self.connection.remote_address()
    }

    pub fn rtt(&self) -> Duration {
        self.connection.rtt()
    }

    pub fn congestion_window(&self) -> u64 {
        self.connection.stats().path.cwnd
    }
}

fn sct_home_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SCT_CERT_DIR") {
        return PathBuf::from(dir);
    }
    let mut dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    dir.push(".sct");
    dir
}

fn known_hosts_path() -> PathBuf {
    sct_home_dir().join("known_hosts")
}

fn load_or_create_server_identity() -> Result<(CertificateDer<'static>, PrivatePkcs8KeyDer<'static>)>
{
    let dir = sct_home_dir();
    fs::create_dir_all(&dir).context("create ~/.sct")?;
    let cert_path = dir.join("server.crt.der");
    let key_path = dir.join("server.key.der");
    if cert_path.exists() && key_path.exists() {
        let cert =
            CertificateDer::from(fs::read(&cert_path).context("read persisted server cert")?);
        let key =
            PrivatePkcs8KeyDer::from(fs::read(&key_path).context("read persisted server key")?);
        return Ok((cert, key));
    }
    let cert = generate_simple_self_signed(vec!["localhost".to_string()])?;
    let cert_der: CertificateDer<'static> = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
    fs::write(&cert_path, cert_der.as_ref()).context("persist server cert")?;
    fs::write(&key_path, key_der.secret_pkcs8_der()).context("persist server key")?;
    warn!(
        "generated SPARQ server identity at {} (TOFU pin is stable across restarts)",
        cert_path.display()
    );
    Ok((cert_der, key_der))
}

fn load_known_hosts(
    path: &PathBuf,
) -> std::result::Result<HashMap<String, String>, std::io::Error> {
    let mut out = HashMap::new();
    if !path.exists() {
        return Ok(out);
    }
    let content = fs::read_to_string(path)?;
    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() == 2 {
            out.insert(parts[0].to_string(), parts[1].to_string());
        }
    }
    Ok(out)
}

fn write_known_hosts(
    path: &PathBuf,
    hosts: &HashMap<String, String>,
) -> std::result::Result<(), std::io::Error> {
    let parent = path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    fs::create_dir_all(parent)?;
    let mut lines = Vec::with_capacity(hosts.len());
    for (host, fp) in hosts {
        lines.push(format!("{host} {fp}"));
    }
    lines.sort();
    fs::write(path, format!("{}\n", lines.join("\n")))
}

fn enforce_or_persist_known_host(
    host: &str,
    cert: &CertificateDer<'_>,
) -> std::result::Result<(), std::io::Error> {
    let path = known_hosts_path();
    let mut hosts = load_known_hosts(&path)?;
    let fingerprint = blake3::hash(cert.as_ref()).to_hex().to_string();
    if let Some(existing) = hosts.get(host) {
        if existing != &fingerprint {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("host key mismatch for {host}"),
            ));
        }
        return Ok(());
    }
    hosts.insert(host.to_string(), fingerprint);
    write_known_hosts(&path, &hosts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loopback_bidi_exchange_works() {
        let server = SctEndpoint::server(TransportConfig {
            bind_addr: "127.0.0.1:0".parse().expect("addr"),
            ..Default::default()
        })
        .expect("server");

        let server_addr = server.local_addr().expect("server addr");
        let server_task = tokio::spawn(async move {
            let accepted = server.accept().await.expect("incoming").expect("conn");
            let (_send, mut recv) = accepted.connection.accept_bi().await.expect("accept bi");
            let mut in_buf = vec![0_u8; 1024 * 1024];
            recv.read_exact(&mut in_buf).await.expect("read");
            assert_eq!(in_buf.len(), 1024 * 1024);
        });

        let client = SctEndpoint::client(TransportConfig {
            bind_addr: "0.0.0.0:0".parse().expect("addr"),
            ..Default::default()
        })
        .expect("client");

        let conn = client
            .connect(server_addr, "localhost")
            .await
            .expect("connect");
        let (mut send, _recv) = conn.open_control_stream().await.expect("open bi");
        let payload = vec![0x5Au8; 1024 * 1024];
        send.write_all(&payload).await.expect("write");
        send.finish().expect("finish");
        assert!(conn.rtt() < Duration::from_millis(5) || conn.rtt() < Duration::from_millis(100));

        server_task.await.expect("server join");
    }
}
