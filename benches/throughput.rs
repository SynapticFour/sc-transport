use criterion::{Criterion, criterion_group, criterion_main};
use futures::StreamExt;
use sc_transport_core::{EventType, HttpSseTransport, TelemetryEvent, Transport};
use sc_transport_datagrams::QuicDatagramTransport;
use sc_transport_quic::QuicStreamTransport;
use tokio::runtime::Runtime;
use tokio::time::{Duration, timeout};

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

fn throughput_sse(c: &mut Criterion) {
    let rt = Runtime::new().expect("runtime");
    c.bench_function("throughput_sse_1sub_1000events", |b| {
        b.iter(|| {
            rt.block_on(async {
                let transport = HttpSseTransport::new();
                let run_id = "bench-sse";
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

criterion_group!(benches, throughput_sse, throughput_quic_and_datagram);
criterion_main!(benches);
