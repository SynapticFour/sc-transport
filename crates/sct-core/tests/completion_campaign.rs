use sct_core::adaptive::{
    AutopilotRuntime, FecEncoder, HybridCongestionController, MultiPathScheduler, Packet, PacketId,
    PacketMeta, ReceiverFeedback, StrategyEngine, TransferMetrics, TransportPath,
};
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct DummyPath {
    rtt: Duration,
    bw: f64,
    loss: f64,
}

impl TransportPath for DummyPath {
    fn send(&mut self, _packet: Packet) {}
    fn estimated_rtt(&self) -> Duration {
        self.rtt
    }
    fn estimated_bandwidth(&self) -> f64 {
        self.bw
    }
    fn loss_rate(&self) -> f64 {
        self.loss
    }
    fn path_kind(&self) -> sct_core::adaptive::PathKind {
        sct_core::adaptive::PathKind::Stream
    }
}

#[derive(Debug, Serialize)]
struct CompletionCampaignSummary {
    completion_first: bool,
    p50_completion_ms: f64,
    p95_completion_ms: f64,
    p99_completion_ms: f64,
    straggler_count: usize,
    canceled_redundant_sends: usize,
}

fn packets() -> Vec<Packet> {
    (0..400_u64)
        .map(|i| Packet {
            id: PacketId(i),
            seq: i,
            payload: vec![0_u8; 1536],
            is_parity: false,
            meta: PacketMeta {
                id: i,
                priority: if i % 11 == 0 { 230 } else { 90 },
                deadline: Some(Instant::now() + Duration::from_millis(30 + (i % 12) * 5)),
                size: 1536,
            },
            fec_group: i / 4,
            reconstructable: false,
        })
        .collect()
}

fn runtime(enabled: bool) -> AutopilotRuntime {
    let mut scheduler = MultiPathScheduler {
        paths: Vec::new(),
        speculative_ratio: 0.15,
        duplicate_budget: 300,
        in_flight_duplicates: 0,
        known_reconstructable: HashSet::new(),
    };
    scheduler.paths.push(Box::new(DummyPath {
        rtt: Duration::from_millis(20),
        bw: 220_000_000.0,
        loss: 0.015,
    }));
    scheduler.paths.push(Box::new(DummyPath {
        rtt: Duration::from_millis(35),
        bw: 170_000_000.0,
        loss: 0.045,
    }));
    let mut cc = HybridCongestionController::default();
    cc.on_network_sample(
        200_000_000.0,
        Duration::from_millis(24),
        Duration::from_millis(21),
        0.025,
    );
    let mut strategy = StrategyEngine::default();
    strategy.update(
        Duration::from_millis(24),
        0.025,
        0.2,
        &ReceiverFeedback {
            decode_delay: Duration::from_millis(14),
            buffer_occupancy: 0.5,
            cpu_load: 0.4,
        },
    );
    let completed_blocks = Arc::new(Mutex::new(HashSet::new()));
    if let Ok(mut done) = completed_blocks.lock() {
        done.insert(3);
        done.insert(9);
        done.insert(20);
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
        completion_first_enabled: enabled,
    }
}

fn maybe_write_artifact(name: &str, body: &str) {
    if let Ok(dir) = std::env::var("SC_TRANSPORT_ARTIFACT_DIR") {
        let mut path = PathBuf::from(dir);
        let _ = fs::create_dir_all(&path);
        path.push(format!("{name}.json"));
        let _ = fs::write(path, body);
    }
}

#[tokio::test]
async fn completion_campaign_metrics() {
    let enabled = std::env::var("SC_SCT_COMPLETION_FIRST").ok().as_deref() != Some("0");
    let mut rt = runtime(enabled);
    rt.run_pipeline(packets()).await;
    let s = CompletionCampaignSummary {
        completion_first: enabled,
        p50_completion_ms: rt.metrics.p50_completion.as_secs_f64() * 1000.0,
        p95_completion_ms: rt.metrics.p95_completion.as_secs_f64() * 1000.0,
        p99_completion_ms: rt.metrics.p99_completion.as_secs_f64() * 1000.0,
        straggler_count: rt.metrics.straggler_count,
        canceled_redundant_sends: rt.metrics.canceled_redundant_sends,
    };
    let body = serde_json::to_string_pretty(&s).expect("serialize completion campaign summary");
    maybe_write_artifact("completion-campaign-summary", &body);
}
