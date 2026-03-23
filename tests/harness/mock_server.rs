use sc_transport_core::{EventType, TelemetryEvent};

pub fn fake_events(run_id: &str, count: usize) -> Vec<TelemetryEvent> {
    (0..count)
        .map(|i| TelemetryEvent {
            run_id: run_id.to_string(),
            task_id: None,
            event_type: if i == 0 {
                EventType::RunStarted
            } else if i + 1 == count {
                EventType::RunCompleted
            } else {
                EventType::Progress
            },
            timestamp_ms: i as u64,
            payload: serde_json::json!({ "idx": i }),
        })
        .collect()
}
