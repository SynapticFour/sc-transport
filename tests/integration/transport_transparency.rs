use futures::StreamExt;
use sc_transport_core::{EventType, HttpSseTransport, TelemetryEvent, Transport};
use sc_transport_datagrams::QuicDatagramTransport;
use sc_transport_quic::QuicStreamTransport;
use tokio::time::{timeout, Duration};

async fn send_sequence<T: Transport>(transport: &T, run_id: &str) {
    let seq = [
        EventType::RunStarted,
        EventType::TaskQueued,
        EventType::TaskStarted,
        EventType::Progress,
        EventType::TaskCompleted,
        EventType::RunCompleted,
    ];
    for (i, event_type) in seq.into_iter().enumerate() {
        transport
            .send_event(
                run_id,
                TelemetryEvent {
                    run_id: run_id.to_string(),
                    task_id: None,
                    event_type,
                    timestamp_ms: i as u64,
                    payload: serde_json::json!({ "i": i }),
                },
            )
            .await
            .expect("send");
    }
}

fn final_state(events: &[TelemetryEvent]) -> &'static str {
    if events
        .iter()
        .any(|e| matches!(e.event_type, EventType::RunFailed))
    {
        "failed"
    } else if events
        .iter()
        .any(|e| matches!(e.event_type, EventType::RunCompleted))
    {
        "completed"
    } else if events
        .iter()
        .any(|e| matches!(e.event_type, EventType::RunStarted))
    {
        "running"
    } else {
        "unknown"
    }
}

#[tokio::test]
async fn transport_transparency_final_state_identical() {
    let run_id = "transparency";
    let sse = HttpSseTransport::new();
    let quic = QuicStreamTransport::new();
    let datagram = QuicDatagramTransport::new();

    let mut sse_stream = sse.subscribe(run_id).await.expect("sse subscribe");
    let mut quic_stream = quic.subscribe(run_id).await.expect("quic subscribe");
    let mut datagram_stream = datagram
        .subscribe(run_id)
        .await
        .expect("datagram subscribe");

    send_sequence(&sse, run_id).await;
    send_sequence(&quic, run_id).await;
    send_sequence(&datagram, run_id).await;

    let mut sse_events = Vec::new();
    let mut quic_events = Vec::new();
    let mut datagram_events = Vec::new();
    for _ in 0..6 {
        sse_events.push(sse_stream.next().await.expect("sse item").expect("ok"));
        quic_events.push(quic_stream.next().await.expect("quic item").expect("ok"));
        if let Ok(Some(Ok(e))) = timeout(Duration::from_millis(20), datagram_stream.next()).await {
            datagram_events.push(e);
        }
    }

    assert_eq!(final_state(&sse_events), "completed");
    assert_eq!(final_state(&quic_events), "completed");
    assert_eq!(final_state(&datagram_events), "completed");
}
