use futures::StreamExt;
use sc_transport_core::{DeliveryStatus, EventType, TelemetryEvent, Transport};
use sc_transport_datagrams::QuicDatagramTransport;
use sc_transport_quic::QuicStreamTransport;
use sc_transport_sse::HttpSseTransport;
use tokio::time::{timeout, Duration};

#[path = "../harness/netem.rs"]
mod netem;

fn event(run_id: &str, i: u64) -> TelemetryEvent {
    TelemetryEvent {
        run_id: run_id.to_string(),
        task_id: None,
        event_type: if i == 0 {
            EventType::RunStarted
        } else if i == 499 {
            EventType::RunCompleted
        } else {
            EventType::Progress
        },
        timestamp_ms: i,
        payload: serde_json::json!({ "i": i }),
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
    } else {
        "unknown"
    }
}

#[tokio::test]
async fn quic_stream_continental_wan_delivery() {
    if let Some(_guard) = netem::NetemGuard::try_apply(&netem::NetemConfig::wan_continental()) {
        let transport = QuicStreamTransport::new();
        let run_id = "wan-qs";
        let mut delivered = 0_u64;
        let fut = async {
            for i in 0..500_u64 {
                let status = transport
                    .send_event(run_id, event(run_id, i))
                    .await
                    .expect("send");
                if matches!(status, DeliveryStatus::Delivered | DeliveryStatus::Sent) {
                    delivered += 1;
                }
            }
        };
        let _ = timeout(Duration::from_secs(30), fut)
            .await
            .expect("timeout");
        assert!(delivered >= 490);
    } else {
        eprintln!("tc netem not available, skipping");
    }
}

#[tokio::test]
async fn datagram_transport_intercontinental_fallback_behavior() {
    if let Some(_guard) = netem::NetemGuard::try_apply(&netem::NetemConfig::wan_intercontinental())
    {
        let transport = QuicDatagramTransport::new();
        let run_id = "wan-dgram";
        let mut fallback = 0_u64;
        let mut delivered = 0_u64;
        for i in 0..200_u64 {
            let status = transport
                .send_event(run_id, event(run_id, i))
                .await
                .expect("send");
            match status {
                DeliveryStatus::FellBack { .. } => fallback += 1,
                DeliveryStatus::Sent | DeliveryStatus::Delivered => delivered += 1,
                DeliveryStatus::Dropped => {}
            }
        }
        let metrics = transport.metrics();
        assert!(fallback >= 1 || (delivered as f64 / 200.0) > 0.90);
        assert_eq!(
            metrics.fallback_count + metrics.events_delivered,
            metrics.events_sent
        );
    } else {
        eprintln!("tc netem not available, skipping");
    }
}

#[tokio::test]
async fn transport_transparency_under_degraded_wan() {
    if let Some(_guard) = netem::NetemGuard::try_apply(&netem::NetemConfig::wan_degraded()) {
        let run_id = "wan-transparent";
        let sse = HttpSseTransport::new();
        let quic = QuicStreamTransport::new();
        let mut sse_stream = sse.subscribe(run_id).await.expect("sub");
        let mut quic_stream = quic.subscribe(run_id).await.expect("sub");
        for i in 0..500_u64 {
            let e = event(run_id, i);
            sse.send_event(run_id, e.clone()).await.expect("send");
            quic.send_event(run_id, e).await.expect("send");
        }
        let mut sse_events = Vec::new();
        let mut quic_events = Vec::new();
        for _ in 0..500 {
            sse_events.push(sse_stream.next().await.expect("x").expect("ok"));
            quic_events.push(quic_stream.next().await.expect("x").expect("ok"));
        }
        assert_eq!(final_state(&sse_events), "completed");
        assert_eq!(final_state(&quic_events), "completed");
    } else {
        eprintln!("tc netem not available, skipping");
    }
}
