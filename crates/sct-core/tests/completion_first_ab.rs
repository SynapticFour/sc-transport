use sct_core::adaptive::{
    AutopilotRuntime, FecEncoder, HybridCongestionController, MultiPathScheduler, Packet, PacketId, PacketMeta,
    ReceiverFeedback, StrategyEngine, TransferMetrics, TransportPath,
};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct DummyPath {
    rtt: Duration,
    bw: f64,
    loss: f64,
}

impl TransportPath for DummyPath {
    fn send(&mut self, _packet: Packet) {}
    fn estimated_rtt(&self) -> Duration { self.rtt }
    fn estimated_bandwidth(&self) -> f64 { self.bw }
    fn loss_rate(&self) -> f64 { self.loss }
    fn path_kind(&self) -> sct_core::adaptive::PathKind { sct_core::adaptive::PathKind::Stream }
}

fn make_packets() -> Vec<Packet> {
    let mut out = Vec::new();
    for i in 0..240_u64 {
        out.push(Packet {
            id: PacketId(i),
            seq: i,
            payload: vec![0_u8; 1024],
            is_parity: false,
            meta: PacketMeta {
                id: i,
                priority: if i % 7 == 0 { 220 } else { 100 },
                deadline: Some(Instant::now() + Duration::from_millis(50 + (i % 8) * 4)),
                size: 1024,
            },
            fec_group: i / 4,
            reconstructable: false,
        });
    }
    out
}

fn runtime(completion_first_enabled: bool) -> AutopilotRuntime {
    let mut scheduler = MultiPathScheduler {
        paths: Vec::new(),
        speculative_ratio: 0.15,
        duplicate_budget: 256,
        in_flight_duplicates: 0,
        known_reconstructable: HashSet::new(),
    };
    scheduler.paths.push(Box::new(DummyPath {
        rtt: Duration::from_millis(18),
        bw: 250_000_000.0,
        loss: 0.01,
    }));
    scheduler.paths.push(Box::new(DummyPath {
        rtt: Duration::from_millis(32),
        bw: 180_000_000.0,
        loss: 0.04,
    }));
    let mut cc = HybridCongestionController::default();
    cc.on_network_sample(220_000_000.0, Duration::from_millis(22), Duration::from_millis(20), 0.02);
    let mut strategy = StrategyEngine::default();
    strategy.update(
        Duration::from_millis(22),
        0.02,
        0.15,
        &ReceiverFeedback {
            decode_delay: Duration::from_millis(12),
            buffer_occupancy: 0.45,
            cpu_load: 0.35,
        },
    );
    let completed_blocks = Arc::new(Mutex::new(HashSet::new()));
    if let Ok(mut done) = completed_blocks.lock() {
        done.insert(2);
        done.insert(5);
        done.insert(11);
    }
    AutopilotRuntime {
        strategy,
        cc,
        scheduler,
        fec: FecEncoder {
            data_shards: 4,
            parity_shards: 2,
        },
        metrics: TransferMetrics::default(),
        completed_blocks,
        completion_first_enabled,
    }
}

#[tokio::test]
async fn completion_first_reduces_tail_and_waste() {
    let packets = make_packets();
    let mut baseline = runtime(false);
    baseline.run_pipeline(packets.clone()).await;
    let mut completion = runtime(true);
    completion.run_pipeline(packets).await;

    assert!(completion.metrics.canceled_redundant_sends > baseline.metrics.canceled_redundant_sends);
    assert!(completion.metrics.straggler_count > 0);
    assert!(completion.metrics.p95_completion <= completion.metrics.p99_completion);
    eprintln!(
        "completion_first_ab baseline_p95={:?} completion_p95={:?} canceled={} stragglers={}",
        baseline.metrics.p95_completion,
        completion.metrics.p95_completion,
        completion.metrics.canceled_redundant_sends,
        completion.metrics.straggler_count
    );
}
