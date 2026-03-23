use sc_transport_core::{DeliveryStatus, EventType, TelemetryEvent, Transport};
use sc_transport_datagrams::QuicDatagramTransport;

#[tokio::test]
async fn datagram_fallback_trigger_emits_fallback_status() {
    let transport = QuicDatagramTransport::new();
    let run_id = "datagram-fallback";

    let status = transport
        .send_event(
            run_id,
            TelemetryEvent {
                run_id: run_id.to_string(),
                task_id: None,
                event_type: EventType::TransportFallback,
                timestamp_ms: 1,
                payload: serde_json::json!({ "force_fallback": true }),
            },
        )
        .await
        .expect("send");

    assert!(matches!(status, DeliveryStatus::FellBack { .. }));
    assert!(transport.metrics().fallback_count >= 1);
}
