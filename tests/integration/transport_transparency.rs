use futures::StreamExt;
use sc_transport_core::{EventType, TelemetryEvent, Transport, TransportError};
use sc_transport_datagrams::QuicDatagramTransport;
use sc_transport_quic::QuicStreamTransport;
use sc_transport_sse::HttpSseTransport;
use tokio::time::{timeout, Duration};

#[path = "../harness/fixtures.rs"]
mod fixtures;

async fn send_sequence<T: Transport>(transport: &T, run_id: &str) {
    for event in fixtures::deterministic_sequence(run_id) {
        transport.send_event(run_id, event).await.expect("send");
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

    let sse_events = collect_payload(&mut sse_stream, 6).await;
    let quic_events = collect_payload(&mut quic_stream, 6).await;
    let datagram_events = collect_payload(&mut datagram_stream, 6).await;

    assert_eq!(final_state(&sse_events), "completed");
    assert_eq!(final_state(&quic_events), "completed");
    assert_eq!(final_state(&datagram_events), "completed");
}

async fn collect_payload(
    stream: &mut (impl StreamExt<Item = Result<TelemetryEvent, TransportError>> + Unpin),
    want: usize,
) -> Vec<TelemetryEvent> {
    let mut out = Vec::new();
    while out.len() < want {
        match timeout(Duration::from_secs(5), stream.next()).await {
            Ok(Some(Ok(event))) => {
                if !matches!(event.event_type, EventType::TransportFallback) {
                    out.push(event);
                }
            }
            _ => break,
        }
    }
    out
}
