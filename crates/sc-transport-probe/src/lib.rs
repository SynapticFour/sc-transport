use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub enum ProbeError {
    Unavailable(String),
}

pub struct NetworkProbe {
    pub probe_packet_count: u32,
    pub probe_duration_secs: f64,
    pub timeout_per_packet: Duration,
}

impl Default for NetworkProbe {
    fn default() -> Self {
        Self {
            probe_packet_count: 50,
            probe_duration_secs: 2.0,
            timeout_per_packet: Duration::from_millis(500),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LinkClass {
    Datacenter,
    Regional,
    Continental,
    Intercontinental,
}

#[derive(Debug, Clone)]
pub struct SuggestedTransportConfig {
    pub initial_cwnd_bytes: u64,
    pub max_parallel_streams: usize,
    pub use_datagrams_for_progress: bool,
    pub chunk_size_for_batch: usize,
}

#[derive(Debug, Clone)]
pub struct NetworkProfile {
    pub min_rtt: Duration,
    pub avg_rtt: Duration,
    pub max_rtt: Duration,
    pub jitter: Duration,
    pub estimated_loss_rate: f64,
    pub estimated_bandwidth_mbps: f64,
    pub link_class: LinkClass,
    pub suggested_config: SuggestedTransportConfig,
}

pub struct ProbeServerHandle {
    join: tokio::task::JoinHandle<()>,
}

impl ProbeServerHandle {
    pub fn abort(self) {
        self.join.abort();
    }
}

pub struct ProbeServer;

impl ProbeServer {
    pub async fn start(addr: SocketAddr) -> Result<(SocketAddr, ProbeServerHandle), ProbeError> {
        use quinn::ServerConfig;
        use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
        let _ = rustls::crypto::ring::default_provider().install_default();
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .map_err(|e| ProbeError::Unavailable(e.to_string()))?;
        let cert_der: CertificateDer<'static> = CertificateDer::from(cert.cert.der().to_vec());
        let key_der = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());
        let mut server_config = ServerConfig::with_single_cert(vec![cert_der], key_der.into())
            .map_err(|e| ProbeError::Unavailable(e.to_string()))?;
        if let Some(tcfg) = Arc::get_mut(&mut server_config.transport) {
            tcfg.datagram_receive_buffer_size(Some(2 * 1024 * 1024));
        }
        let endpoint =
            quinn::Endpoint::server(server_config, addr).map_err(|e| ProbeError::Unavailable(e.to_string()))?;
        let listen_addr = endpoint
            .local_addr()
            .map_err(|e| ProbeError::Unavailable(e.to_string()))?;
        let join = tokio::spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                let conn = incoming.await;
                if let Ok(conn) = conn {
                    tokio::spawn(async move {
                        while let Ok(d) = conn.read_datagram().await {
                            let _ = conn.send_datagram(d);
                        }
                    });
                }
            }
        });
        Ok((listen_addr, ProbeServerHandle { join }))
    }
}

impl NetworkProbe {
    fn classify(min_rtt: Duration) -> LinkClass {
        if min_rtt < Duration::from_millis(5) {
            LinkClass::Datacenter
        } else if min_rtt < Duration::from_millis(30) {
            LinkClass::Regional
        } else if min_rtt < Duration::from_millis(100) {
            LinkClass::Continental
        } else {
            LinkClass::Intercontinental
        }
    }

    fn suggest(link: &LinkClass) -> SuggestedTransportConfig {
        match link {
            LinkClass::Datacenter => SuggestedTransportConfig {
                initial_cwnd_bytes: 64 * 1024,
                max_parallel_streams: 16,
                use_datagrams_for_progress: true,
                chunk_size_for_batch: 128,
            },
            LinkClass::Regional => SuggestedTransportConfig {
                initial_cwnd_bytes: 256 * 1024,
                max_parallel_streams: 8,
                use_datagrams_for_progress: true,
                chunk_size_for_batch: 64,
            },
            LinkClass::Continental => SuggestedTransportConfig {
                initial_cwnd_bytes: 1024 * 1024,
                max_parallel_streams: 8,
                use_datagrams_for_progress: false,
                chunk_size_for_batch: 32,
            },
            LinkClass::Intercontinental => SuggestedTransportConfig {
                initial_cwnd_bytes: 4 * 1024 * 1024,
                max_parallel_streams: 16,
                use_datagrams_for_progress: false,
                chunk_size_for_batch: 16,
            },
        }
    }

    pub fn compute_jitter(rtts: &[Duration]) -> Duration {
        if rtts.is_empty() {
            return Duration::from_millis(0);
        }
        let vals = rtts.iter().map(|d| d.as_secs_f64()).collect::<Vec<_>>();
        let mean = vals.iter().sum::<f64>() / vals.len() as f64;
        let var = vals
            .iter()
            .map(|v| (v - mean) * (v - mean))
            .sum::<f64>()
            / vals.len() as f64;
        Duration::from_secs_f64(var.sqrt())
    }

    pub async fn measure(&self, target_addr: SocketAddr) -> Result<NetworkProfile, ProbeError> {
        use quinn::{ClientConfig, Endpoint};
        use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
        use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
        use rustls::{DigitallySignedStruct, SignatureScheme};

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

        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut rustls_cfg = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(InsecureVerifier))
            .with_no_client_auth();
        rustls_cfg.enable_early_data = true;
        let client_config = ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(rustls_cfg)
                .map_err(|e| ProbeError::Unavailable(e.to_string()))?,
        ));
        let mut endpoint = Endpoint::client(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))
            .map_err(|e| ProbeError::Unavailable(e.to_string()))?;
        endpoint.set_default_client_config(client_config);
        let server_name = if target_addr.ip().is_loopback() {
            "localhost".to_string()
        } else {
            target_addr.ip().to_string()
        };
        let conn = endpoint
            .connect(target_addr, &server_name)
            .map_err(|e| ProbeError::Unavailable(e.to_string()))?
            .await
            .map_err(|e| ProbeError::Unavailable(e.to_string()))?;

        let mut rtts = Vec::new();
        let mut acks = 0_u32;
        let mut sent = 0_u32;
        let start = Instant::now();
        while sent < self.probe_packet_count
            && start.elapsed().as_secs_f64() < self.probe_duration_secs
        {
            let seq = sent;
            sent += 1;
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u32)
                .unwrap_or(0);
            let mut payload = [0_u8; 8];
            payload[..4].copy_from_slice(&seq.to_be_bytes());
            payload[4..].copy_from_slice(&ts.to_be_bytes());
            let t0 = Instant::now();
            let _ = conn.send_datagram(payload.to_vec().into());
            if let Ok(Ok(_echo)) =
                tokio::time::timeout(self.timeout_per_packet, conn.read_datagram()).await
            {
                acks += 1;
                rtts.push(t0.elapsed());
            }
        }
        if rtts.is_empty() {
            return Err(ProbeError::Unavailable("no probe responses".to_string()));
        }
        let min_rtt = *rtts.iter().min().unwrap_or(&Duration::from_millis(1));
        let max_rtt = *rtts.iter().max().unwrap_or(&min_rtt);
        let avg_rtt =
            Duration::from_secs_f64(rtts.iter().map(|d| d.as_secs_f64()).sum::<f64>() / rtts.len() as f64);
        let jitter = Self::compute_jitter(&rtts);
        let loss = 1.0 - (acks as f64 / sent.max(1) as f64);

        let mut bw_estimates = Vec::new();
        for _ in 0..5 {
            let p1 = vec![0_u8; 1200];
            let p2 = vec![1_u8; 1200];
            let _ = conn.send_datagram(p1.into());
            let _ = conn.send_datagram(p2.into());
            if let Ok(Ok(_)) = tokio::time::timeout(self.timeout_per_packet, conn.read_datagram()).await {
                let t1 = Instant::now();
                if let Ok(Ok(_)) =
                    tokio::time::timeout(self.timeout_per_packet, conn.read_datagram()).await
                {
                    let dispersion = t1.elapsed().as_secs_f64().max(0.000_001);
                    bw_estimates.push((1200.0 * 8.0) / dispersion / 1_000_000.0);
                }
            }
        }
        bw_estimates.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let estimated_bandwidth_mbps = bw_estimates
            .get(bw_estimates.len().saturating_sub(1) / 2)
            .copied()
            .unwrap_or(0.0);
        let link = Self::classify(min_rtt);
        let suggested = Self::suggest(&link);
        Ok(NetworkProfile {
            min_rtt,
            avg_rtt,
            max_rtt,
            jitter,
            estimated_loss_rate: loss,
            estimated_bandwidth_mbps,
            link_class: link.clone(),
            suggested_config: suggested,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loopback_profile_is_datacenter_class() {
        let (addr, handle) =
            ProbeServer::start(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await.expect("server");
        let p = NetworkProbe {
            probe_packet_count: 20,
            probe_duration_secs: 1.0,
            ..Default::default()
        };
        let profile = p.measure(addr).await.expect("measure");
        assert_eq!(profile.link_class, LinkClass::Datacenter);
        assert!(profile.min_rtt < Duration::from_millis(5));
        assert!(profile.estimated_loss_rate < 0.05);
        handle.abort();
    }

    #[test]
    fn suggested_config_scales_with_link_class() {
        let link = NetworkProbe::classify(Duration::from_millis(120));
        let cfg = NetworkProbe::suggest(&link);
        assert!(cfg.initial_cwnd_bytes >= 1_000_000);
        assert!(!cfg.use_datagrams_for_progress);
    }

    #[test]
    fn jitter_calculation_is_correct() {
        let samples = [10, 12, 8, 11, 9, 13, 10, 11, 9, 12]
            .iter()
            .map(|ms| Duration::from_millis(*ms))
            .collect::<Vec<_>>();
        let j = NetworkProbe::compute_jitter(&samples);
        assert!(j > Duration::from_millis(0));
        assert!(j < Duration::from_millis(3));
    }

    #[tokio::test]
    async fn probe_handles_timeout_gracefully() {
        let p = NetworkProbe {
            probe_packet_count: 10,
            probe_duration_secs: 0.2,
            timeout_per_packet: Duration::from_millis(10),
        };
        let res = p
            .measure(SocketAddr::from((Ipv4Addr::LOCALHOST, 9)))
            .await;
        assert!(res.is_err() || res.as_ref().map(|x| x.estimated_loss_rate > 0.3).unwrap_or(true));
    }
}
