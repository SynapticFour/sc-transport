use prometheus::{IntCounter, IntGauge, Registry};

#[derive(Clone)]
pub struct SctMetrics {
    pub registry: Registry,
    pub transfers_started: IntCounter,
    pub transfers_completed: IntCounter,
    pub active_transfers: IntGauge,
}

impl SctMetrics {
    pub fn new() -> Self {
        let registry = Registry::new();
        let transfers_started =
            IntCounter::new("sct_transfers_started", "Started transfers").expect("valid metric");
        let transfers_completed = IntCounter::new("sct_transfers_completed", "Completed transfers")
            .expect("valid metric");
        let active_transfers =
            IntGauge::new("sct_active_transfers", "Active transfers").expect("valid metric");
        registry
            .register(Box::new(transfers_started.clone()))
            .expect("register");
        registry
            .register(Box::new(transfers_completed.clone()))
            .expect("register");
        registry
            .register(Box::new(active_transfers.clone()))
            .expect("register");
        Self {
            registry,
            transfers_started,
            transfers_completed,
            active_transfers,
        }
    }
}

impl Default for SctMetrics {
    fn default() -> Self {
        Self::new()
    }
}
