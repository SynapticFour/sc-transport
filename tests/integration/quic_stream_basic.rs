use futures::StreamExt;
use sc_transport_core::{EventType, TelemetryEvent, Transport};
use sc_transport_quic::QuicStreamTransport;

#[path = "../harness/artifacts.rs"]
mod artifacts;

#[tokio::test]
async fn quic_stream_basic_send_subscribe() {
    let transport = QuicStreamTransport::new();
    let run_id = "quic-basic";
    let mut stream = transport.subscribe(run_id).await.expect("subscribe");

    transport
        .send_event(
            run_id,
            TelemetryEvent {
                run_id: run_id.to_string(),
                task_id: None,
                event_type: EventType::TaskStarted,
                timestamp_ms: 1,
                payload: serde_json::json!({"ok": true}),
            },
        )
        .await
        .expect("send");

    let event = stream.next().await.expect("item").expect("ok");
    assert!(matches!(event.event_type, EventType::TaskStarted));
    artifacts::maybe_write_artifact(
        "quic_stream_basic",
        r#"{"test":"quic_stream_basic","status":"ok"}"#,
    );
}
