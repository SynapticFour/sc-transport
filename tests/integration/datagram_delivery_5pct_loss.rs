use futures::StreamExt;
use sc_transport_core::{EventType, TelemetryEvent, Transport};
use sc_transport_datagrams::QuicDatagramTransport;
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn datagram_delivery_5pct_loss_simulated() {
    let transport = QuicDatagramTransport::new();
    let run_id = "datagram-5";
    let mut stream = transport.subscribe(run_id).await.expect("subscribe");

    for i in 0..400_u64 {
        transport
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

    let mut app_received = 0_u64;
    let mut seen = 0_u64;
    for _ in 0..400 {
        if let Ok(Some(Ok(_))) = timeout(Duration::from_millis(3), stream.next()).await {
            seen += 1;
            // simulate 5% app/network loss by dropping every 20th event at receiver
            if seen % 20 != 0 {
                app_received += 1;
            }
        }
    }
    let ratio = app_received as f64 / 400.0;
    assert!(ratio > 0.85, "ratio was {ratio}");
}
