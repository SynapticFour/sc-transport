use futures::StreamExt;
use sc_transport_core::{EventType, TelemetryEvent, Transport};
use sc_transport_sse::HttpSseTransport;

#[tokio::test]
async fn sse_basic_delivers_events() {
    let transport = HttpSseTransport::new();
    let run_id = "sse-basic";
    let mut stream = transport.subscribe(run_id).await.expect("subscribe");

    for i in 0..100_u64 {
        transport
            .send_event(
                run_id,
                TelemetryEvent {
                    run_id: run_id.to_string(),
                    task_id: None,
                    event_type: if i == 0 {
                        EventType::RunStarted
                    } else if i == 99 {
                        EventType::RunCompleted
                    } else {
                        EventType::Progress
                    },
                    timestamp_ms: i,
                    payload: serde_json::json!({ "i": i }),
                },
            )
            .await
            .expect("send");
    }

    let mut received = 0_u64;
    while received < 100 {
        let _ = stream.next().await.expect("stream item").expect("ok");
        received += 1;
    }

    assert_eq!(received, 100);
}
