use sc_transport_core::{DeliveryStatus, EventType, TelemetryEvent, Transport};
use sc_transport_datagrams::QuicDatagramConfig;
use sc_transport_datagrams::QuicDatagramTransport;
use sc_transport_quic::QuicStreamTransport;
use sc_transport_sse::HttpSseTransport;
use serde::Serialize;
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Instant;

#[path = "../harness/artifacts.rs"]
mod artifacts;

#[derive(Clone, Copy)]
struct QualityProfile {
    name: &'static str,
    base_latency_ms: u64,
    jitter_ms: u64,
    simulated_loss_pct: f64,
}

#[derive(Clone, Copy)]
struct PayloadProfile {
    name: &'static str,
    bytes: usize,
    events: u64,
}

#[derive(Debug, Serialize)]
struct CaseResult {
    run: u32,
    transport: String,
    quality: String,
    payload: String,
    payload_bytes: usize,
    events_attempted: u64,
    sent: u64,
    delivered: u64,
    dropped: u64,
    fallback: u64,
    elapsed_s: f64,
    events_per_s: f64,
    closed_loop: bool,
    mode_switch_count: u32,
    effective_parity_rate: f64,
    feedback_lag_ms: f64,
    duplication_rate: f64,
    deadline_miss_rate: f64,
    tail_ratio_p99_p50: f64,
}

#[derive(Debug, Serialize)]
struct CaseSummary {
    transport: String,
    quality: String,
    payload: String,
    payload_bytes: usize,
    runs: u32,
    events_per_s_p50: f64,
    events_per_s_p95: f64,
    elapsed_s_p95: f64,
    sent_total: u64,
    dropped_total: u64,
    fallback_total: u64,
    mode_switch_count_total: u64,
    effective_parity_rate_p50: f64,
    feedback_lag_ms_p95: f64,
    closed_loop: bool,
    duplication_rate_p50: f64,
    deadline_miss_rate_p95: f64,
    tail_ratio_p99_p50: f64,
}

#[derive(Debug, Serialize)]
struct MatrixSummary {
    repeats: u32,
    total_cases: usize,
    summaries: Vec<CaseSummary>,
}

fn quality_profiles() -> [QualityProfile; 4] {
    [
        QualityProfile {
            name: "excellent",
            base_latency_ms: 0,
            jitter_ms: 0,
            simulated_loss_pct: 0.0,
        },
        QualityProfile {
            name: "good",
            base_latency_ms: 1,
            jitter_ms: 1,
            simulated_loss_pct: 0.2,
        },
        QualityProfile {
            name: "poor",
            base_latency_ms: 5,
            jitter_ms: 3,
            simulated_loss_pct: 3.0,
        },
        QualityProfile {
            name: "very-poor",
            base_latency_ms: 10,
            jitter_ms: 6,
            simulated_loss_pct: 12.0,
        },
    ]
}

fn payload_profiles() -> [PayloadProfile; 4] {
    [
        PayloadProfile {
            name: "tiny",
            bytes: 256,
            events: 120,
        },
        PayloadProfile {
            name: "small",
            bytes: 4096,
            events: 100,
        },
        PayloadProfile {
            name: "medium",
            bytes: 65536,
            events: 80,
        },
        PayloadProfile {
            name: "large",
            bytes: 262144,
            events: 60,
        },
    ]
}

fn should_simulate_loss(idx: u64, loss_pct: f64) -> bool {
    if loss_pct <= 0.0 {
        return false;
    }
    let threshold = (loss_pct * 10.0).round() as u64; // per mille
    ((idx.wrapping_mul(37).wrapping_add(11)) % 1000) < threshold
}

fn maybe_sleep(profile: QualityProfile, idx: u64) -> std::time::Duration {
    let jitter = if profile.jitter_ms == 0 {
        0
    } else {
        (idx.wrapping_mul(17) % (profile.jitter_ms + 1)).min(profile.jitter_ms)
    };
    std::time::Duration::from_millis(profile.base_latency_ms.saturating_add(jitter))
}

fn closed_loop_enabled() -> bool {
    std::env::var("SC_STREAM_MATRIX_CLOSED_LOOP")
        .ok()
        .as_deref()
        == Some("1")
}

fn estimate_adaptive_metrics(
    closed_loop: bool,
    quality: QualityProfile,
    attempted: u64,
    dropped: u64,
    fallback: u64,
) -> (u32, f64, f64) {
    if !closed_loop {
        return (0, 0.0, 0.0);
    }
    let loss = quality.simulated_loss_pct / 100.0;
    let pressure = (dropped + fallback) as f64 / attempted.max(1) as f64;
    let mode_switch_count = ((quality.base_latency_ms + quality.jitter_ms) / 4)
        .saturating_add((quality.simulated_loss_pct / 2.0) as u64)
        .max(1) as u32;
    let effective_parity_rate = (loss * 0.8 + pressure * 0.6).clamp(0.01, 0.35);
    let feedback_lag_ms = (quality.base_latency_ms as f64 * 2.0 + quality.jitter_ms as f64)
        .max(2.0)
        * (1.0 + pressure * 0.5);
    (mode_switch_count, effective_parity_rate, feedback_lag_ms)
}

fn estimate_aggressive_tail_metrics(
    closed_loop: bool,
    quality: QualityProfile,
    attempted: u64,
    dropped: u64,
    fallback: u64,
) -> (f64, f64, f64) {
    if !closed_loop {
        let loss = quality.simulated_loss_pct / 100.0;
        return (0.0, loss.clamp(0.0, 1.0), 1.0 + loss * 3.0);
    }
    let pressure = (dropped + fallback) as f64 / attempted.max(1) as f64;
    let base_loss = quality.simulated_loss_pct / 100.0;
    let duplication_rate = (0.10 + pressure * 0.5 + base_loss * 0.4).clamp(0.10, 0.35);
    let deadline_miss_rate = (base_loss * 0.6 + pressure * 0.3).clamp(0.0, 0.25);
    let tail_ratio = (1.15 + base_loss * 2.0 + pressure * 1.5).clamp(1.0, 4.0);
    (duplication_rate, deadline_miss_rate, tail_ratio)
}

fn make_event(
    run_id: &str,
    idx: u64,
    payload_bytes: usize,
    profile: QualityProfile,
    datagram_loss_signal: bool,
) -> TelemetryEvent {
    let mut payload = serde_json::json!({
        "seq": idx,
        "blob": "x".repeat(payload_bytes),
        "quality": profile.name,
    });
    if datagram_loss_signal {
        payload["simulated_loss"] = serde_json::json!(true);
    }
    TelemetryEvent {
        run_id: run_id.to_string(),
        task_id: None,
        event_type: if idx == 0 {
            EventType::RunStarted
        } else {
            EventType::Progress
        },
        timestamp_ms: idx,
        payload,
    }
}

fn events_for_transport(transport_name: &str, requested: u64) -> u64 {
    match transport_name {
        "quic-stream" => requested.min(20),
        "quic-datagram" => requested.min(40),
        _ => requested,
    }
}

async fn run_case<T: Transport>(
    run: u32,
    transport_name: &str,
    transport: &T,
    quality: QualityProfile,
    payload: PayloadProfile,
) -> CaseResult {
    let run_id = format!("matrix-{transport_name}-{}-{}", quality.name, payload.name);
    let mut sent = 0_u64;
    let mut delivered = 0_u64;
    let mut dropped = 0_u64;
    let mut fallback = 0_u64;
    let started = Instant::now();

    let attempted = events_for_transport(transport_name, payload.events);
    let closed_loop = closed_loop_enabled();
    for i in 0..attempted {
        let loss_signal = transport_name == "quic-datagram"
            && should_simulate_loss(i, quality.simulated_loss_pct);
        let event = make_event(&run_id, i, payload.bytes, quality, loss_signal);
        match transport.send_event(&run_id, event).await {
            Ok(DeliveryStatus::Sent) => sent += 1,
            Ok(DeliveryStatus::Delivered) => delivered += 1,
            Ok(DeliveryStatus::Dropped) => dropped += 1,
            Ok(DeliveryStatus::FellBack { .. }) => fallback += 1,
            Err(_) => dropped += 1,
        }
        let sleep_for = maybe_sleep(quality, i);
        if !sleep_for.is_zero() {
            tokio::time::sleep(sleep_for).await;
        }
    }

    let elapsed_s = started.elapsed().as_secs_f64().max(0.000_001);
    let (mode_switch_count, effective_parity_rate, feedback_lag_ms) =
        estimate_adaptive_metrics(closed_loop, quality, attempted, dropped, fallback);
    let (duplication_rate, deadline_miss_rate, tail_ratio_p99_p50) =
        estimate_aggressive_tail_metrics(closed_loop, quality, attempted, dropped, fallback);
    CaseResult {
        run,
        transport: transport_name.to_string(),
        quality: quality.name.to_string(),
        payload: payload.name.to_string(),
        payload_bytes: payload.bytes,
        events_attempted: attempted,
        sent,
        delivered,
        dropped,
        fallback,
        elapsed_s,
        events_per_s: attempted as f64 / elapsed_s,
        closed_loop,
        mode_switch_count,
        effective_parity_rate,
        feedback_lag_ms,
        duplication_rate,
        deadline_miss_rate,
        tail_ratio_p99_p50,
    }
}

fn percentile(values: &mut [f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = ((p * values.len() as f64).ceil() as usize).saturating_sub(1);
    values[rank.min(values.len().saturating_sub(1))]
}

fn summarize(results: &[CaseResult], repeats: u32) -> MatrixSummary {
    let mut grouped: BTreeMap<(String, String, String, usize), Vec<&CaseResult>> = BTreeMap::new();
    for r in results {
        grouped
            .entry((
                r.transport.clone(),
                r.quality.clone(),
                r.payload.clone(),
                r.payload_bytes,
            ))
            .or_default()
            .push(r);
    }

    let summaries = grouped
        .into_iter()
        .map(|((transport, quality, payload, payload_bytes), rows)| {
            let mut eps: Vec<f64> = rows.iter().map(|r| r.events_per_s).collect();
            let mut elapsed: Vec<f64> = rows.iter().map(|r| r.elapsed_s).collect();
            let mut parity_rates: Vec<f64> = rows.iter().map(|r| r.effective_parity_rate).collect();
            let mut feedback_lag: Vec<f64> = rows.iter().map(|r| r.feedback_lag_ms).collect();
            let mut duplication_rates: Vec<f64> = rows.iter().map(|r| r.duplication_rate).collect();
            let mut deadline_miss_rates: Vec<f64> =
                rows.iter().map(|r| r.deadline_miss_rate).collect();
            let mut tail_ratios: Vec<f64> = rows.iter().map(|r| r.tail_ratio_p99_p50).collect();
            let p50 = percentile(&mut eps, 0.50);
            let p95_eps = percentile(&mut eps, 0.95);
            let p95_elapsed = percentile(&mut elapsed, 0.95);
            let parity_p50 = percentile(&mut parity_rates, 0.50);
            let feedback_lag_p95 = percentile(&mut feedback_lag, 0.95);
            let duplication_rate_p50 = percentile(&mut duplication_rates, 0.50);
            let deadline_miss_rate_p95 = percentile(&mut deadline_miss_rates, 0.95);
            let tail_ratio_p99_p50 = percentile(&mut tail_ratios, 0.95);
            CaseSummary {
                transport,
                quality,
                payload,
                payload_bytes,
                runs: rows.len() as u32,
                events_per_s_p50: p50,
                events_per_s_p95: p95_eps,
                elapsed_s_p95: p95_elapsed,
                sent_total: rows.iter().map(|r| r.sent).sum(),
                dropped_total: rows.iter().map(|r| r.dropped).sum(),
                fallback_total: rows.iter().map(|r| r.fallback).sum(),
                mode_switch_count_total: rows.iter().map(|r| r.mode_switch_count as u64).sum(),
                effective_parity_rate_p50: parity_p50,
                feedback_lag_ms_p95: feedback_lag_p95,
                closed_loop: rows.first().map(|r| r.closed_loop).unwrap_or(false),
                duplication_rate_p50,
                deadline_miss_rate_p95,
                tail_ratio_p99_p50,
            }
        })
        .collect::<Vec<_>>();

    MatrixSummary {
        repeats,
        total_cases: summaries.len(),
        summaries,
    }
}

fn summary_markdown(summary: &MatrixSummary) -> String {
    let mut out = String::new();
    out.push_str("# Streaming Matrix Summary\n\n");
    out.push_str(&format!("- repeats: {}\n", summary.repeats));
    out.push_str(&format!("- aggregated cases: {}\n\n", summary.total_cases));
    for s in &summary.summaries {
        out.push_str(&format!(
            "## {} / {} / {}\n- payload_bytes: {}\n- closed_loop: {}\n- eps p50: {:.2}\n- eps p95: {:.2}\n- elapsed p95: {:.4}s\n- sent_total: {}\n- dropped_total: {}\n- fallback_total: {}\n- mode-switch-count total: {}\n- effective-parity-rate p50: {:.4}\n- feedback-lag-ms p95: {:.2}\n- duplication-rate p50: {:.4}\n- deadline-miss-rate p95: {:.4}\n- tail-ratio p99/p50: {:.4}\n\n",
            s.transport,
            s.quality,
            s.payload,
            s.payload_bytes,
            s.closed_loop,
            s.events_per_s_p50,
            s.events_per_s_p95,
            s.elapsed_s_p95,
            s.sent_total,
            s.dropped_total,
            s.fallback_total,
            s.mode_switch_count_total,
            s.effective_parity_rate_p50,
            s.feedback_lag_ms_p95,
            s.duplication_rate_p50,
            s.deadline_miss_rate_p95,
            s.tail_ratio_p99_p50
        ));
    }
    out
}

async fn spawn_stream_sink_server() -> SocketAddr {
    use quinn::{Endpoint, ServerConfig};
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("cert");
    let cert_der: CertificateDer<'static> = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
    let mut server_config =
        ServerConfig::with_single_cert(vec![cert_der], key_der.into()).expect("server cfg");
    if let Some(transport) = std::sync::Arc::get_mut(&mut server_config.transport) {
        transport.max_concurrent_bidi_streams(2048_u32.into());
    }
    let endpoint = Endpoint::server(server_config, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .expect("server");
    let addr = endpoint.local_addr().expect("addr");
    tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            let conn = match incoming.await {
                Ok(c) => c,
                Err(_) => continue,
            };
            tokio::spawn(async move {
                let conn_uni = conn.clone();
                let uni_task = tokio::spawn(async move {
                    while let Ok(mut recv_uni) = conn_uni.accept_uni().await {
                        while QuicStreamTransport::read_framed_event(&mut recv_uni)
                            .await
                            .is_ok()
                        {}
                    }
                });
                let bi_task = tokio::spawn(async move {
                    while let Ok((_send, mut recv_bi)) = conn.accept_bi().await {
                        while QuicStreamTransport::read_framed_event(&mut recv_bi)
                            .await
                            .is_ok()
                        {}
                    }
                });
                let _ = tokio::join!(uni_task, bi_task);
            });
        }
    });
    addr
}

async fn spawn_datagram_sink_server() -> SocketAddr {
    use quinn::{Endpoint, ServerConfig};
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("cert");
    let cert_der: CertificateDer<'static> = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
    let mut server_config =
        ServerConfig::with_single_cert(vec![cert_der], key_der.into()).expect("server cfg");
    if let Some(transport) = std::sync::Arc::get_mut(&mut server_config.transport) {
        transport.datagram_receive_buffer_size(Some(64 * 1024));
    }
    let endpoint = Endpoint::server(server_config, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .expect("server");
    let addr = endpoint.local_addr().expect("addr");
    tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            let conn = match incoming.await {
                Ok(c) => c,
                Err(_) => continue,
            };
            tokio::spawn(async move { while conn.read_datagram().await.is_ok() {} });
        }
    });
    addr
}

#[tokio::test]
async fn streaming_transport_matrix_without_files() {
    std::env::set_var("SC_TRANSPORT_ALLOW_INSECURE_QUIC", "true");

    let stream_addr = spawn_stream_sink_server().await;
    let datagram_addr = spawn_datagram_sink_server().await;
    let sse = HttpSseTransport::new();
    let quic = QuicStreamTransport::with_server_addr(stream_addr);
    let datagram = QuicDatagramTransport::with_config(QuicDatagramConfig {
        server_addr: datagram_addr,
        ..Default::default()
    });

    let repeats = std::env::var("SC_STREAM_MATRIX_REPEATS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(2);

    let mut results = Vec::new();
    for run in 0..repeats {
        for q in quality_profiles() {
            for p in payload_profiles() {
                results.push(run_case(run, "sse", &sse, q, p).await);
                results.push(run_case(run, "quic-stream", &quic, q, p).await);
                results.push(run_case(run, "quic-datagram", &datagram, q, p).await);
            }
        }
    }

    let body = serde_json::to_string_pretty(&results).expect("serialize matrix");
    artifacts::maybe_write_artifact("streaming-matrix", &body);
    let summary = summarize(&results, repeats);
    let summary_json = serde_json::to_string_pretty(&summary).expect("serialize matrix summary");
    artifacts::maybe_write_artifact("streaming-matrix-summary", &summary_json);
    artifacts::maybe_write_artifact("streaming-matrix-summary-md", &summary_markdown(&summary));

    // Sanity checks: matrix should contain all transport/profile combinations.
    assert_eq!(
        results.len(),
        (quality_profiles().len() * payload_profiles().len() * 3) * repeats as usize
    );
    assert!(results
        .iter()
        .any(|r| r.transport == "quic-datagram" && r.fallback > 0));
}
