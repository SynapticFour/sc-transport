use criterion::{Criterion, criterion_group, criterion_main};
use futures::StreamExt;
use sc_transport_core::{EventType, TelemetryEvent, Transport};
use sc_transport_datagrams::QuicDatagramTransport;
use tokio::runtime::Runtime;
use tokio::time::{Duration, timeout};

fn packet_loss_datagram_delivery_ratio(c: &mut Criterion) {
    let rt = Runtime::new().expect("runtime");
    c.bench_function("packet_loss_datagram_delivery_ratio_simulated", |b| {
        b.iter(|| {
            rt.block_on(async {
                let transport = QuicDatagramTransport::new();
                let run_id = "bench-loss";
                let mut stream = transport.subscribe(run_id).await.expect("subscribe");
                let sent = 1000_u64;
                for i in 0..sent {
                    let _ = transport
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
                }
                let mut received = 0_u64;
                for _ in 0..sent {
                    if let Ok(Some(Ok(_))) = timeout(Duration::from_millis(2), stream.next()).await {
                        received += 1;
                    }
                }
                received as f64 / sent as f64
            })
        })
    });
}

criterion_group!(benches, packet_loss_datagram_delivery_ratio);
criterion_main!(benches);
