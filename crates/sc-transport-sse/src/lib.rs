use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use sc_transport_core::{
    DeliveryStatus, EventStream, TelemetryEvent, Transport, TransportError, TransportMetrics,
};
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio_stream::wrappers::ReceiverStream;

#[derive(Default)]
struct Counters {
    events_sent: AtomicU64,
    events_delivered: AtomicU64,
    events_dropped: AtomicU64,
    fallback_count: AtomicU64,
    active_subscribers: AtomicU64,
}

/// Stable SSE fan-out implementation backed by Tokio broadcast channels.
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
            Ok(_) => {
                self.counters
                    .events_delivered
                    .fetch_add(1, Ordering::Relaxed);
                Ok(DeliveryStatus::Delivered)
            }
            Err(_) => Ok(DeliveryStatus::Sent),
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
                        let _ = event_tx
                            .send(Err(TransportError::Unavailable(e.to_string())))
                            .await;
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
