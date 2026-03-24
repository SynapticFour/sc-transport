//! sc-transport-core: Transport trait and TelemetryEvent types
//! for the Synaptic Core telemetry channel.
//!
//! All implementations (SSE, QUIC streams, QUIC datagrams) implement
//! the Transport trait. The caller is unaware of which transport is active.

use async_trait::async_trait;
use futures::stream::BoxStream;

/// A telemetry event emitted during workflow execution.
/// Encoded as MessagePack (rmp-serde) on the wire.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TelemetryEvent {
    pub run_id: String,
    pub task_id: Option<String>,
    pub event_type: EventType,
    pub timestamp_ms: u64,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    RunStarted,
    RunCompleted,
    RunFailed,
    TaskQueued,
    TaskStarted,
    TaskCompleted,
    TaskFailed,
    Progress,
    TransportFallback,
}

/// Delivery result for a single event send attempt.
#[derive(Debug, Clone)]
pub enum DeliveryStatus {
    Sent,
    Delivered,
    Dropped,
    FellBack { reason: String },
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("transport unavailable: {0}")]
    Unavailable(String),
    #[error("fallback required")]
    FallbackRequired,
    #[error("quic error: {0}")]
    QuicError(String),
}

#[async_trait]
pub trait Transport: Send + Sync + 'static {
    async fn send_event(
        &self,
        run_id: &str,
        event: TelemetryEvent,
    ) -> Result<DeliveryStatus, TransportError>;

    async fn subscribe(&self, run_id: &str) -> Result<EventStream, TransportError>;

    fn supports_unreliable(&self) -> bool;

    fn name(&self) -> &'static str;

    fn metrics(&self) -> TransportMetrics;
}

pub type EventStream = BoxStream<'static, Result<TelemetryEvent, TransportError>>;

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct TransportMetrics {
    pub events_sent: u64,
    pub events_delivered: u64,
    pub events_dropped: u64,
    pub fallback_count: u64,
    pub current_mode: String,
    pub active_subscribers: u64,
}
