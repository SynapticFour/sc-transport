use futures::StreamExt;
use sc_transport_core::{EventType, TelemetryEvent, Transport};
use sc_transport_quic::QuicStreamTransport;

#[tokio::test]
async fn quic_stream_reconnect_resubscribe_receives_events() {
    std::env::set_var("SC_QUIC_MIRROR_SSE", "1");
    let transport = QuicStreamTransport::new();
    let run_id = "quic-reconnect";

    let mut first = transport.subscribe(run_id).await.expect("first subscribe");
    transport
        .send_event(
            run_id,
            TelemetryEvent {
                run_id: run_id.to_string(),
                task_id: None,
                event_type: EventType::RunStarted,
                timestamp_ms: 1,
                payload: serde_json::json!({}),
            },
        )
        .await
        .expect("send first");
    let first_event = loop {
        let event = first.next().await.expect("first item").expect("ok");
        if !matches!(event.event_type, EventType::TransportFallback) {
            break event;
        }
    };
    assert!(matches!(first_event.event_type, EventType::RunStarted));
    drop(first);

    let mut second = transport.subscribe(run_id).await.expect("second subscribe");
    transport
        .send_event(
            run_id,
            TelemetryEvent {
                run_id: run_id.to_string(),
                task_id: None,
                event_type: EventType::RunCompleted,
                timestamp_ms: 2,
                payload: serde_json::json!({}),
            },
        )
        .await
        .expect("send second");

    let got = loop {
        let event = second.next().await.expect("second item").expect("ok");
        if !matches!(event.event_type, EventType::TransportFallback) {
            break event;
        }
    };
    assert!(matches!(got.event_type, EventType::RunCompleted));
}
