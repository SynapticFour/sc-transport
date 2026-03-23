use sc_transport_core::{TelemetryEvent, Transport};

pub async fn collect_events<T: Transport>(transport: &T, run_id: &str, n: usize) -> Vec<TelemetryEvent> {
    let mut stream = transport.subscribe(run_id).await.expect("subscribe");
    let mut out = Vec::new();
    while out.len() < n {
        if let Some(Ok(event)) = futures::StreamExt::next(&mut stream).await {
            out.push(event);
        } else {
            break;
        }
    }
    out
}
