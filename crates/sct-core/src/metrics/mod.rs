use prometheus::{Histogram, HistogramOpts, IntCounter, IntGauge, Registry};

#[derive(Clone)]
pub struct SctMetrics {
    pub registry: Registry,
    pub transfers_started: IntCounter,
    pub transfers_completed: IntCounter,
    pub active_transfers: IntGauge,
    pub transfer_duration_seconds: Histogram,
    pub transfer_bytes_total: IntCounter,
    pub transfer_loss_rate: Histogram,
    pub transfer_p99_ms: Histogram,
    pub nack_retransmits_total: IntCounter,
    pub fec_encoded_blocks_total: IntCounter,
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

        let transfer_duration_seconds = Histogram::with_opts(
            HistogramOpts::new("sct_transfer_duration_seconds", "Transfer duration").buckets(vec![
                0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0, 300.0,
            ]),
        )
        .expect("valid metric");

        let transfer_bytes_total =
            IntCounter::new("sct_transfer_bytes_total", "Total bytes transferred")
                .expect("valid metric");

        let transfer_loss_rate = Histogram::with_opts(
            HistogramOpts::new("sct_transfer_loss_rate", "Observed loss rate (0–1)")
                .buckets(vec![0.0, 0.01, 0.02, 0.05, 0.08, 0.1, 0.15, 0.25, 0.5, 1.0]),
        )
        .expect("valid metric");

        let transfer_p99_ms = Histogram::with_opts(
            HistogramOpts::new("sct_transfer_p99_ms", "p99 completion latency ms").buckets(vec![
                1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 500.0,
            ]),
        )
        .expect("valid metric");

        let nack_retransmits_total =
            IntCounter::new("sct_nack_retransmits_total", "NACK-triggered retransmits")
                .expect("valid metric");

        let fec_encoded_blocks_total =
            IntCounter::new("sct_fec_encoded_blocks_total", "FEC-encoded blocks")
                .expect("valid metric");

        registry
            .register(Box::new(transfers_started.clone()))
            .expect("register");
        registry
            .register(Box::new(transfers_completed.clone()))
            .expect("register");
        registry
            .register(Box::new(active_transfers.clone()))
            .expect("register");
        registry
            .register(Box::new(transfer_duration_seconds.clone()))
            .expect("register");
        registry
            .register(Box::new(transfer_bytes_total.clone()))
            .expect("register");
        registry
            .register(Box::new(transfer_loss_rate.clone()))
            .expect("register");
        registry
            .register(Box::new(transfer_p99_ms.clone()))
            .expect("register");
        registry
            .register(Box::new(nack_retransmits_total.clone()))
            .expect("register");
        registry
            .register(Box::new(fec_encoded_blocks_total.clone()))
            .expect("register");

        Self {
            registry,
            transfers_started,
            transfers_completed,
            active_transfers,
            transfer_duration_seconds,
            transfer_bytes_total,
            transfer_loss_rate,
            transfer_p99_ms,
            nack_retransmits_total,
            fec_encoded_blocks_total,
        }
    }
}

impl Default for SctMetrics {
    fn default() -> Self {
        Self::new()
    }
}
