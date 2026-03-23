//! sc-transport-core: Transport trait and TelemetryEvent types
//! for the Synaptic Core telemetry channel.
//!
//! All implementations (SSE, QUIC streams, QUIC datagrams) implement
//! the Transport trait. The caller is unaware of which transport is active.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use futures::stream::BoxStream;
use tokio::sync::{RwLock, broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;

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

#[derive(Default)]
struct Counters {
    events_sent: AtomicU64,
    events_delivered: AtomicU64,
    events_dropped: AtomicU64,
    fallback_count: AtomicU64,
    active_subscribers: AtomicU64,
}

/// The stable default transport implementation using in-process fan-out.
pub struct HttpSseTransport {
    channels: Arc<RwLock<HashMap<String, broadcast::Sender<TelemetryEvent>>>>,
    counters: Arc<Counters>,
}

impl Default for HttpSseTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpSseTransport {
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            counters: Arc::new(Counters::default()),
        }
    }

    async fn channel_for_run(&self, run_id: &str) -> broadcast::Sender<TelemetryEvent> {
        let mut lock = self.channels.write().await;
        if let Some(tx) = lock.get(run_id) {
            return tx.clone();
        }

        let (tx, _rx) = broadcast::channel(1024);
        lock.insert(run_id.to_string(), tx.clone());
        tx
    }
}

#[async_trait]
impl Transport for HttpSseTransport {
    async fn send_event(
        &self,
        run_id: &str,
        event: TelemetryEvent,
    ) -> Result<DeliveryStatus, TransportError> {
        let _encoded = rmp_serde::to_vec_named(&event)
            .map_err(|e| TransportError::Serialization(e.to_string()))?;

        let tx = self.channel_for_run(run_id).await;
        self.counters.events_sent.fetch_add(1, Ordering::Relaxed);
        match tx.send(event) {
            Ok(_subscribers) => {
                self.counters
                    .events_delivered
                    .fetch_add(1, Ordering::Relaxed);
                Ok(DeliveryStatus::Delivered)
            }
            Err(_e) => Ok(DeliveryStatus::Sent),
        }
    }

    async fn subscribe(&self, run_id: &str) -> Result<EventStream, TransportError> {
        let tx = self.channel_for_run(run_id).await;
        let mut rx = tx.subscribe();
        self.counters
            .active_subscribers
            .fetch_add(1, Ordering::Relaxed);

        let counters = Arc::clone(&self.counters);
        let (event_tx, event_rx) = mpsc::channel::<Result<TelemetryEvent, TransportError>>(256);
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if event_tx.send(Ok(event)).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = event_tx.send(Err(TransportError::Unavailable(e.to_string()))).await;
                        break;
                    }
                }
            }
            counters.active_subscribers.fetch_sub(1, Ordering::Relaxed);
        });

        Ok(Box::pin(ReceiverStream::new(event_rx)))
    }

    fn supports_unreliable(&self) -> bool {
        false
    }

    fn name(&self) -> &'static str {
        "sse"
    }

    fn metrics(&self) -> TransportMetrics {
        TransportMetrics {
            events_sent: self.counters.events_sent.load(Ordering::Relaxed),
            events_delivered: self.counters.events_delivered.load(Ordering::Relaxed),
            events_dropped: self.counters.events_dropped.load(Ordering::Relaxed),
            fallback_count: self.counters.fallback_count.load(Ordering::Relaxed),
            current_mode: "sse".to_string(),
            active_subscribers: self.counters.active_subscribers.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn sample_event(run_id: &str, event_type: EventType, ts: u64) -> TelemetryEvent {
        TelemetryEvent {
            run_id: run_id.to_string(),
            task_id: None,
            event_type,
            timestamp_ms: ts,
            payload: serde_json::json!({ "n": ts }),
        }
    }

    #[tokio::test]
    async fn sse_transport_delivers_to_subscriber() {
        let transport = HttpSseTransport::new();
        let run_id = "run-1";
        let mut stream = transport.subscribe(run_id).await.expect("subscribe should succeed");

        let status = transport
            .send_event(run_id, sample_event(run_id, EventType::RunStarted, 1))
            .await
            .expect("send should succeed");
        assert!(matches!(status, DeliveryStatus::Delivered));

        let received = stream.next().await.expect("event item").expect("ok event");
        assert_eq!(received.run_id, run_id);
        assert!(matches!(received.event_type, EventType::RunStarted));
    }

    #[tokio::test]
    async fn sse_transport_tracks_metrics() {
        let transport = HttpSseTransport::new();
        let run_id = "run-2";
        let mut stream = transport.subscribe(run_id).await.expect("subscribe should succeed");

        for i in 0..3 {
            transport
                .send_event(run_id, sample_event(run_id, EventType::Progress, i))
                .await
                .expect("send should succeed");
            let _ = stream.next().await;
        }

        let metrics = transport.metrics();
        assert_eq!(metrics.events_sent, 3);
        assert_eq!(metrics.events_delivered, 3);
        assert_eq!(metrics.current_mode, "sse");
        assert!(metrics.active_subscribers >= 1);
    }
}
