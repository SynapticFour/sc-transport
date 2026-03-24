use sc_transport_core::{EventType, TelemetryEvent};

pub fn deterministic_sequence(run_id: &str) -> Vec<TelemetryEvent> {
    let seq = [
        EventType::RunStarted,
        EventType::TaskQueued,
        EventType::TaskStarted,
        EventType::Progress,
        EventType::TaskCompleted,
        EventType::RunCompleted,
    ];
    seq.into_iter()
        .enumerate()
        .map(|(i, event_type)| TelemetryEvent {
            run_id: run_id.to_string(),
            task_id: None,
            event_type,
            timestamp_ms: i as u64,
            payload: serde_json::json!({ "fixture_index": i }),
        })
        .collect()
}
