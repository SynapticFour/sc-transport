use criterion::{criterion_group, criterion_main, Criterion};
use futures::StreamExt;
use sc_transport_core::{EventType, TelemetryEvent, Transport};
use sc_transport_datagrams::QuicDatagramTransport;
use sc_transport_quic::QuicStreamTransport;
use sc_transport_sse::HttpSseTransport;
use tokio::runtime::Runtime;
use tokio::time::{timeout, Duration};
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn scripted_event(run_id: &str, i: u64) -> TelemetryEvent {
    TelemetryEvent {
        run_id: run_id.to_string(),
        task_id: None,
        event_type: if i == 0 {
            EventType::RunStarted
        } else {
            EventType::Progress
        },
        timestamp_ms: i,
        payload: serde_json::json!({ "i": i }),
    }
}

fn write_result_artifact(scenario: &str, events_per_sec: f64, transport: &str) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let date = chrono_like_date();
    let rust_version = Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());
    let payload = format!(
        "{{\"scenario\":\"{}\",\"transport\":\"{}\",\"events_per_sec\":{},\"timestamp\":{},\"rust_version\":\"{}\",\"hostname\":\"{}\"}}",
        scenario, transport, events_per_sec, ts, rust_version, hostname
    );
    let path = format!("docs/RESULTS/{}-{}-{}.json", date, transport, scenario);
    let _ = fs::write(path, payload);
}

fn chrono_like_date() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = now / 86_400;
    // Stable lightweight YYYY-MM-DD fallback approximation anchored at 1970-01-01.
    // Good enough for artifact filenames.
    let year = 1970 + (days / 365) as i64;
    let day_of_year = (days % 365) as i64;
    let month = 1 + (day_of_year / 30);
    let day = 1 + (day_of_year % 30);
    format!("{:04}-{:02}-{:02}", year, month, day)
}

fn throughput_sse(c: &mut Criterion) {
    let rt = Runtime::new().expect("runtime");
    c.bench_function("throughput_sse_1sub_1000events", |b| {
        b.iter(|| {
            rt.block_on(async {
                let transport = HttpSseTransport::new();
                let run_id = "bench-sse";
                let started = std::time::Instant::now();
                let mut stream = transport.subscribe(run_id).await.expect("subscribe");
                for i in 0..1000_u64 {
                    let _ = transport
                        .send_event(run_id, scripted_event(run_id, i))
                        .await
                        .expect("send");
                }
                for _ in 0..1000 {
                    let _ = stream.next().await.expect("item").expect("ok");
                }
                let eps = 1000.0 / started.elapsed().as_secs_f64().max(0.000_001);
                write_result_artifact("1sub_1000events", eps, "sse");
            });
        })
    });
}

fn throughput_quic_and_datagram(c: &mut Criterion) {
    let rt = Runtime::new().expect("runtime");
    c.bench_function("throughput_quic_stream_1sub_1000events", |b| {
        b.iter(|| {
            rt.block_on(async {
                let transport = QuicStreamTransport::new();
                let run_id = "bench-quic";
                let mut stream = transport.subscribe(run_id).await.expect("subscribe");
                for i in 0..1000_u64 {
                    let _ = transport
                        .send_event(run_id, scripted_event(run_id, i))
                        .await
                        .expect("send");
                }
                for _ in 0..1000 {
                    let _ = stream.next().await.expect("item").expect("ok");
                }
            });
        })
    });

    c.bench_function("throughput_datagram_1sub_1000events", |b| {
        b.iter(|| {
            rt.block_on(async {
                let transport = QuicDatagramTransport::new();
                let run_id = "bench-datagram";
                let mut stream = transport.subscribe(run_id).await.expect("subscribe");
                for i in 0..1000_u64 {
                    let _ = transport
                        .send_event(run_id, scripted_event(run_id, i))
                        .await
                        .expect("send");
                }
                // consume up to 1000 available events
                let mut consumed = 0_u64;
                while consumed < 1000 {
                    let next = timeout(Duration::from_millis(1), stream.next()).await;
                    if let Ok(Some(_)) = next {
                        consumed += 1;
                    } else {
                        break;
                    }
                }
            });
        })
    });
}

fn throughput_with_scientific_cc(c: &mut Criterion) {
    let rt = Runtime::new().expect("runtime");
    c.bench_function("throughput_quic_scientific_cc_1sub_1000events", |b| {
        b.iter(|| {
            rt.block_on(async {
                let transport = QuicStreamTransport::with_scientific_cc();
                let run_id = "bench-quic-sci";
                let mut stream = transport.subscribe(run_id).await.expect("subscribe");
                let started = std::time::Instant::now();
                for i in 0..1000_u64 {
                    let _ = transport
                        .send_event(run_id, scripted_event(run_id, i))
                        .await
                        .expect("send");
                }
                for _ in 0..1000 {
                    let _ = stream.next().await.expect("item").expect("ok");
                }
                let eps = 1000.0 / started.elapsed().as_secs_f64().max(0.000_001);
                write_result_artifact("scientific_cc_1000events", eps, "quic-stream");
            });
        })
    });
}

fn throughput_batch_sender(c: &mut Criterion) {
    let rt = Runtime::new().expect("runtime");
    c.bench_function("throughput_batch_sender_1000events", |b| {
        b.iter(|| {
            rt.block_on(async {
                let transport = QuicStreamTransport::with_scientific_cc();
                let run_id = "bench-batch";
                let events = (0..1000_u64)
                    .map(|i| scripted_event(run_id, i))
                    .collect::<Vec<_>>();
                let started = std::time::Instant::now();
                let _ = transport.send_events_batch(run_id, events).await;
                let eps = 1000.0 / started.elapsed().as_secs_f64().max(0.000_001);
                write_result_artifact("batch_sender_1000events", eps, "quic-stream");
            });
        })
    });
}

criterion_group!(
    benches,
    throughput_sse,
    throughput_quic_and_datagram,
    throughput_with_scientific_cc,
    throughput_batch_sender
);
criterion_main!(benches);
