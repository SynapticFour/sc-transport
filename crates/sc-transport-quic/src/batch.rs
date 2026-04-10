use crate::QuicStreamTransport;
use sc_transport_core::TelemetryEvent;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

pub struct BatchSender {
    pub max_parallel_streams: usize,
    pub chunk_size: usize,
}

impl Default for BatchSender {
    fn default() -> Self {
        Self {
            max_parallel_streams: 8,
            chunk_size: 64,
        }
    }
}

pub struct BatchSendResult {
    pub total_events: usize,
    pub sent_events: usize,
    pub failed_chunks: usize,
    pub elapsed: Duration,
    pub effective_throughput_events_per_sec: f64,
}

impl BatchSender {
    pub async fn send_batch(
        &self,
        connection: &quinn::Connection,
        _run_id: &str,
        events: Vec<TelemetryEvent>,
    ) -> BatchSendResult {
        let total = events.len();
        let started = Instant::now();
        let sem = std::sync::Arc::new(Semaphore::new(self.max_parallel_streams.max(1)));
        let mut set = JoinSet::new();

        for chunk in events.chunks(self.chunk_size.max(1)) {
            let permit = sem.clone().acquire_owned().await;
            let Ok(permit) = permit else { continue };
            let conn = connection.clone();
            let chunk_events = chunk.to_vec();
            set.spawn(async move {
                let _permit = permit;
                let mut sent = 0_usize;
                let mut failed = false;
                let stream = conn.open_uni().await;
                let Ok(mut stream) = stream else {
                    return (0_usize, 1_usize);
                };
                for e in chunk_events {
                    if QuicStreamTransport::write_framed_event(&mut stream, &e)
                        .await
                        .is_ok()
                    {
                        sent += 1;
                    } else {
                        failed = true;
                        break;
                    }
                }
                let _ = stream.finish();
                if failed {
                    (sent, 1_usize)
                } else {
                    (sent, 0_usize)
                }
            });
        }

        let mut sent_events = 0_usize;
        let mut failed_chunks = 0_usize;
        while let Some(res) = set.join_next().await {
            if let Ok((sent, failed)) = res {
                sent_events += sent;
                failed_chunks += failed;
            }
        }
        let elapsed = started.elapsed();
        let eps = sent_events as f64 / elapsed.as_secs_f64().max(0.000_001);
        BatchSendResult {
            total_events: total,
            sent_events,
            failed_chunks,
            elapsed,
            effective_throughput_events_per_sec: eps,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::QuicStreamTransport;
    use sc_transport_core::{EventType, TelemetryEvent};

    fn mk(run_id: &str, i: u64) -> TelemetryEvent {
        TelemetryEvent {
            run_id: run_id.to_string(),
            task_id: None,
            event_type: EventType::Progress,
            timestamp_ms: i,
            payload: serde_json::json!({"i": i}),
        }
    }

    #[tokio::test]
    async fn batch_splits_into_correct_chunks() {
        let b = BatchSender {
            max_parallel_streams: 8,
            chunk_size: 64,
        };
        let events = (0..200).map(|i| mk("x", i)).collect::<Vec<_>>();
        let chunk_count = events.chunks(b.chunk_size).count();
        assert_eq!(chunk_count, 4);
    }

    #[tokio::test]
    async fn batch_uses_framing_compatible_with_read_framed_event() {
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        let write = tokio::spawn(async move {
            for i in 0..5_u64 {
                QuicStreamTransport::write_framed_event(&mut a, &mk("r", i))
                    .await
                    .expect("write");
            }
        });
        for i in 0..5_u64 {
            let got = QuicStreamTransport::read_framed_event(&mut b).await.expect("read");
            assert_eq!(got.run_id, "r");
            assert_eq!(got.timestamp_ms, i);
        }
        write.await.expect("join");
    }

    #[cfg(feature = "quic-streams")]
    #[tokio::test]
    async fn batch_loopback_integration() {
        use quinn::{Endpoint, ServerConfig};
        use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .expect("cert");
        let cert_der: CertificateDer<'static> = CertificateDer::from(cert.cert.der().to_vec());
        let key_der = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());
        let mut server_config =
            ServerConfig::with_single_cert(vec![cert_der], key_der.into()).expect("server cfg");
        if let Some(transport) = std::sync::Arc::get_mut(&mut server_config.transport) {
            transport.max_concurrent_uni_streams(128_u32.into());
        }
        let server = Endpoint::server(
            server_config,
            std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 0)),
        )
        .expect("server");
        let addr = server.local_addr().expect("addr");
        let recv_task = tokio::spawn(async move {
            let incoming = server.accept().await.expect("incoming");
            let conn = incoming.await.expect("conn");
            let mut received = 0_usize;
            while received < 512 {
                let uni = conn.accept_uni().await;
                let Ok(mut s) = uni else { break };
                while let Ok(_event) = crate::QuicStreamTransport::read_framed_event(&mut s).await {
                    received += 1;
                    if received >= 512 {
                        break;
                    }
                }
            }
            received
        });
        let events = (0..512).map(|i| mk("loop", i)).collect::<Vec<_>>();
        let t = QuicStreamTransport::with_server_addr(addr);
        let res = t
            .send_events_batch("loop", events)
            .await
            .expect("batch");
        let _ = recv_task.await;
        assert!(res.sent_events >= 480);
        assert!(res.effective_throughput_events_per_sec > 100.0);
    }

    #[cfg(feature = "quic-streams")]
    #[tokio::test]
    async fn batch_fails_gracefully_when_stream_limit_exceeded() {
        use quinn::{Endpoint, ServerConfig};
        use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .expect("cert");
        let cert_der: CertificateDer<'static> = CertificateDer::from(cert.cert.der().to_vec());
        let key_der = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());
        let mut server_config =
            ServerConfig::with_single_cert(vec![cert_der], key_der.into()).expect("server cfg");
        if let Some(transport) = std::sync::Arc::get_mut(&mut server_config.transport) {
            transport.max_concurrent_uni_streams(0_u32.into());
        }
        let server = Endpoint::server(
            server_config,
            std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 0)),
        )
        .expect("server");
        let addr = server.local_addr().expect("addr");
        let _server_task = tokio::spawn(async move {
            if let Some(incoming) = server.accept().await {
                let _ = incoming.await;
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });

        let t = QuicStreamTransport::with_server_addr(addr);
        let events = (0..64).map(|i| mk("limit", i)).collect::<Vec<_>>();
        let res = t.send_events_batch("limit", events).await.expect("batch");
        assert!(res.failed_chunks > 0);
    }
}
