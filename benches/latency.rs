use criterion::{criterion_group, criterion_main, Criterion};
use futures::StreamExt;
use sc_transport_core::{EventType, TelemetryEvent, Transport};
use sc_transport_sse::HttpSseTransport;
use std::time::Instant;
use tokio::runtime::Runtime;

fn latency_sse_send_to_receive(c: &mut Criterion) {
    let rt = Runtime::new().expect("runtime");
    c.bench_function("latency_sse_send_to_receive", |b| {
        b.iter(|| {
            rt.block_on(async {
                let transport = HttpSseTransport::new();
                let run_id = "bench-latency";
                let mut stream = transport.subscribe(run_id).await.expect("subscribe");
                let start = Instant::now();
                transport
                    .send_event(
                        run_id,
                        TelemetryEvent {
                            run_id: run_id.to_string(),
                            task_id: None,
                            event_type: EventType::Progress,
                            timestamp_ms: 1,
                            payload: serde_json::json!({}),
                        },
                    )
                    .await
                    .expect("send");
                let _ = stream.next().await.expect("item").expect("ok");
                start.elapsed()
            })
        })
    });
}

criterion_group!(benches, latency_sse_send_to_receive);
criterion_main!(benches);
