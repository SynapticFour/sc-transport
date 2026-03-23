use futures::StreamExt;
use sc_transport_core::{EventType, TelemetryEvent, Transport};
use sc_transport_datagrams::QuicDatagramTransport;
use tokio::time::{Duration, timeout};

#[tokio::test]
async fn datagram_delivery_no_loss_ratio_over_95pct() {
    let transport = QuicDatagramTransport::new();
    let run_id = "crate-datagram-noloss";
    let mut stream = transport.subscribe(run_id).await.expect("subscribe");

    for i in 0..1000_u64 {
        let _ = transport
            .send_event(
                run_id,
                TelemetryEvent {
                    run_id: run_id.to_string(),
                    task_id: None,
                    event_type: if i == 999 {
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
    for _ in 0..1000 {
        if let Ok(Some(Ok(_))) = timeout(Duration::from_millis(3), stream.next()).await {
            received += 1;
        }
    }
    assert!((received as f64 / 1000.0) > 0.95);
}
