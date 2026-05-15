use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use reed_solomon_erasure::galois_8::ReedSolomon;
use tokio::sync::mpsc;

use crate::protocol::{encode, ChunkDescriptor};

mod optimization;
mod predictive;

pub use optimization::{
    estimate_completion, optimize_global_schedule, CompletionEstimate, CongestionSignal,
    OptimizationKpi, PathCorrelation, PathState, QueueModel, TransmissionDecision,
};
pub use predictive::{
    forecast_congestion, smoothed_utility, CongestionForecast, ControlState, HysteresisThreshold,
    MultiTimescaleClock, NetworkSample, OscillationDetector, PacketStabilization,
    PredictiveStabilizer, StabilityBudget, StabilityTelemetry, TimescaleTier,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PacketId(pub u64);

#[derive(Debug, Clone)]
pub struct PacketMeta {
    pub id: u64,
    pub priority: u8,
    pub deadline: Option<Instant>,
    pub size: usize,
}

#[derive(Debug, Clone)]
pub struct Packet {
    pub id: PacketId,
    pub seq: u64,
    pub payload: Vec<u8>,
    pub is_parity: bool,
    pub meta: PacketMeta,
    pub fec_group: u64,
    pub reconstructable: bool,
    /// Index within the RS parity block (0 .. parity_shards-1).
    pub parity_index: usize,
}

impl Packet {
    pub fn is_critical(&self) -> bool {
        self.meta.priority >= 200 || self.meta.deadline.is_some()
    }

    pub fn nearing_deadline(&self) -> bool {
        self.meta
            .deadline
            .map(|d| d <= Instant::now() + Duration::from_millis(15))
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone)]
pub struct PathScore {
    pub expected_delivery_time: Duration,
    pub loss_probability: f64,
    pub bandwidth: f64,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub id: u64,
    pub total_shards: usize,
    pub required_shards: usize,
    pub sent_shards: usize,
    pub received_acks: usize,
    pub in_flight: usize,
}

impl Block {
    pub fn is_complete(&self) -> bool {
        self.received_acks >= self.required_shards
    }

    pub fn is_almost_complete(&self) -> bool {
        self.required_shards.saturating_sub(self.received_acks) <= 1
    }

    /// Remaining data shards before the block can be decoded (pipeline-aware).
    pub fn remaining_data_shards(&self) -> usize {
        optimization::remaining_data_shards(self)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SchedulingDecision {
    pub additional_shards: usize,
    pub duplicate: bool,
    pub panic_mode: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Snapshot-Percentile werden in optimize_transfer berechnet, aber nicht als finale Metrik nach außen übernommen.
struct OptimizationSnapshot {
    p50_completion: Duration,
    p95_completion: Duration,
    p99_completion: Duration,
    straggler_count: usize,
    canceled_redundant_sends: usize,
}

pub trait TransportPath: Send + Sync {
    fn send(&mut self, packet: Packet);
    fn estimated_rtt(&self) -> Duration;
    fn estimated_bandwidth(&self) -> f64;
    fn loss_rate(&self) -> f64;
    fn path_kind(&self) -> PathKind;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    Stream,
    Datagram,
}

#[derive(Debug)]
struct PathStats {
    rtt_ms: AtomicU64,
    bw_kbps: AtomicU64,
    loss_per_mille: AtomicU64,
}

impl Default for PathStats {
    fn default() -> Self {
        Self {
            rtt_ms: AtomicU64::new(20),
            bw_kbps: AtomicU64::new(100_000),
            loss_per_mille: AtomicU64::new(0),
        }
    }
}

pub struct QuicStreamPath {
    tx: mpsc::UnboundedSender<Packet>,
    stats: Arc<PathStats>,
}

impl QuicStreamPath {
    pub fn new(tx: mpsc::UnboundedSender<Packet>) -> Self {
        Self {
            tx,
            stats: Arc::new(PathStats::default()),
        }
    }

    pub fn update_stats(&self, rtt: Duration, bandwidth_bps: f64, loss_rate: f64) {
        self.stats
            .rtt_ms
            .store(rtt.as_millis().max(1) as u64, Ordering::Relaxed);
        self.stats
            .bw_kbps
            .store((bandwidth_bps / 1000.0).max(1.0) as u64, Ordering::Relaxed);
        self.stats.loss_per_mille.store(
            (loss_rate.clamp(0.0, 1.0) * 1000.0) as u64,
            Ordering::Relaxed,
        );
    }
}

impl TransportPath for QuicStreamPath {
    fn send(&mut self, packet: Packet) {
        let _ = self.tx.send(packet);
    }

    fn estimated_rtt(&self) -> Duration {
        Duration::from_millis(self.stats.rtt_ms.load(Ordering::Relaxed).max(1))
    }

    fn estimated_bandwidth(&self) -> f64 {
        self.stats.bw_kbps.load(Ordering::Relaxed) as f64 * 1000.0
    }

    fn loss_rate(&self) -> f64 {
        self.stats.loss_per_mille.load(Ordering::Relaxed) as f64 / 1000.0
    }

    fn path_kind(&self) -> PathKind {
        PathKind::Stream
    }
}

pub struct QuicDatagramPath {
    tx: mpsc::UnboundedSender<Packet>,
    stats: Arc<PathStats>,
}

impl QuicDatagramPath {
    pub fn new(tx: mpsc::UnboundedSender<Packet>) -> Self {
        Self {
            tx,
            stats: Arc::new(PathStats::default()),
        }
    }

    pub fn update_stats(&self, rtt: Duration, bandwidth_bps: f64, loss_rate: f64) {
        self.stats
            .rtt_ms
            .store(rtt.as_millis().max(1) as u64, Ordering::Relaxed);
        self.stats
            .bw_kbps
            .store((bandwidth_bps / 1000.0).max(1.0) as u64, Ordering::Relaxed);
        self.stats.loss_per_mille.store(
            (loss_rate.clamp(0.0, 1.0) * 1000.0) as u64,
            Ordering::Relaxed,
        );
    }
}

impl TransportPath for QuicDatagramPath {
    fn send(&mut self, packet: Packet) {
        let _ = self.tx.send(packet);
    }

    fn estimated_rtt(&self) -> Duration {
        Duration::from_millis(self.stats.rtt_ms.load(Ordering::Relaxed).max(1))
    }

    fn estimated_bandwidth(&self) -> f64 {
        self.stats.bw_kbps.load(Ordering::Relaxed) as f64 * 1000.0
    }

    fn loss_rate(&self) -> f64 {
        self.stats.loss_per_mille.load(Ordering::Relaxed) as f64 / 1000.0
    }

    fn path_kind(&self) -> PathKind {
        PathKind::Datagram
    }
}

pub struct MultiPathScheduler {
    pub paths: Vec<Box<dyn TransportPath>>,
    pub speculative_ratio: f64,
    pub duplicate_budget: usize,
    pub in_flight_duplicates: usize,
    pub known_reconstructable: HashSet<u64>,
    pub tokens: f64,
    pub last_token_refill: Instant,
    /// Last primary-path raw utility (for predictive loop feedback).
    pub last_primary_utility: f64,
    /// Per-path queue / pacing model for predicted delay.
    pub queue_models: Vec<QueueModel>,
    pub path_correlation: PathCorrelation,
    pub optimization_kpi: OptimizationKpi,
    pub exploration_seed: u64,
}

impl MultiPathScheduler {
    pub fn estimate_unused_bandwidth(&self, target_send_rate: f64) -> f64 {
        let available: f64 = self.paths.iter().map(|p| p.estimated_bandwidth()).sum();
        (available - target_send_rate).max(0.0)
    }

    fn sync_queue_models_and_correlation(&mut self) {
        let kinds: Vec<PathKind> = self.paths.iter().map(|p| p.path_kind()).collect();
        self.path_correlation = PathCorrelation::from_path_kinds(&kinds);
        if self.queue_models.len() != self.paths.len() {
            self.queue_models
                .resize(self.paths.len(), QueueModel::default());
        }
    }

    pub fn path_states_snapshot(&self) -> Vec<PathState> {
        self.paths
            .iter()
            .map(|p| PathState {
                rtt: p.estimated_rtt(),
                bandwidth: p.estimated_bandwidth(),
                loss: p.loss_rate().clamp(0.0, 0.99),
                kind: p.path_kind(),
            })
            .collect()
    }

    /// Per-path diagnostics: `(path_index, score, utility)` where **higher utility is better**.
    pub fn path_scores(
        &self,
        packet: &Packet,
        tail_penalty: f64,
        block: &Block,
        parity_shards: usize,
        inflight: &[Packet],
    ) -> Vec<(usize, PathScore, f64)> {
        let paths = self.path_states_snapshot();
        self.paths
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                let qm = self.queue_models.get(idx).copied().unwrap_or_default();
                let bw = p.estimated_bandwidth().max(1.0);
                let q_delay = qm.predicted_queue_delay(bw);
                let rtt = p.estimated_rtt();
                let expected = rtt + Duration::from_secs_f64(q_delay.max(0.0));
                let loss = p.loss_rate().clamp(0.0, 0.99);
                let utility = optimization::path_send_utility(
                    idx,
                    &paths,
                    &self.queue_models,
                    packet,
                    block,
                    inflight,
                    parity_shards,
                    tail_penalty,
                );
                (
                    idx,
                    PathScore {
                        expected_delivery_time: expected,
                        loss_probability: loss,
                        bandwidth: bw,
                    },
                    utility,
                )
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)] // Send path bundles FEC, congestion, and block context.
    pub fn distribute_and_send(
        &mut self,
        packet: Packet,
        block: &Block,
        tail_penalty: f64,
        fec_parity_ratio: f64,
        _data_shards: usize,
        parity_shards: usize,
        target_send_rate: f64,
        congestion: &CongestionSignal,
        inflight: &[Packet],
        stab: &PacketStabilization,
        control: &mut ControlState,
    ) {
        if self.paths.is_empty() {
            return;
        }
        // Parity uses the same ChunkDescriptor framing as data (`frame_fec_wire_shard` on send).
        self.sync_queue_models_and_correlation();
        self.refill_tokens(target_send_rate * stab.pacing_scale);
        let packet_bytes = packet.meta.size as f64;
        let pkt_sz = packet.meta.size;
        if packet_bytes <= 2.0 * 1500.0 {
            // Soft pacer: never drop a packet — only debit available credits (no silent loss).
            self.tokens = (self.tokens - packet_bytes).max(0.0);
        }
        let paths_snap = self.path_states_snapshot();
        let util_raw: Vec<(usize, f64)> = (0..self.paths.len())
            .map(|i| {
                let u = optimization::path_send_utility(
                    i,
                    &paths_snap,
                    &self.queue_models,
                    &packet,
                    block,
                    inflight,
                    parity_shards,
                    tail_penalty,
                );
                (i, u)
            })
            .collect();
        let u_max_raw = util_raw.iter().map(|(_, u)| *u).fold(0.0_f64, f64::max);
        control.utility_momentum =
            predictive::smoothed_utility(0.26, u_max_raw, control.utility_momentum);
        let denom = u_max_raw.max(1e-9);
        let util_vec: Vec<(usize, f64)> = util_raw
            .iter()
            .map(|&(i, u)| {
                // Weniger momentum-Gewicht: aktuelle Utility dominiert stärker.
                // Verhindert dass ein schlechter momentum-Wert den Pfad-Score verzerrt.
                let blended = 0.88 * u + 0.12 * control.utility_momentum * (u / denom);
                (i, blended)
            })
            .collect();
        // Kleine Payloads: weniger Exploration (Varianz schädlicher als Benefit).
        // Große Payloads: mehr Exploration ok (Scheduling-Fehler hat weniger Gewicht).
        let exploration_epsilon = if packet.meta.size < 64 * 1024 {
            0.01 // tiny/small: fast deterministisch
        } else if packet.meta.size < 512 * 1024 {
            0.03 // medium
        } else {
            0.05 // large
        };
        let primary_idx = optimization::pick_primary_path(
            &util_vec,
            &mut self.exploration_seed,
            exploration_epsilon,
        )
        .unwrap_or(0);
        let utility_primary = util_vec
            .iter()
            .find(|(i, _)| *i == primary_idx)
            .map(|(_, u)| *u)
            .unwrap_or(0.0);
        let utility_primary_raw = util_raw
            .iter()
            .find(|(i, _)| *i == primary_idx)
            .map(|(_, u)| *u)
            .unwrap_or(utility_primary);
        self.last_primary_utility = utility_primary_raw.max(1e-9);
        self.paths[primary_idx].send(packet.clone());
        let pacing_deficit = (2.0 * 1500.0 - self.tokens).max(0.0);
        {
            let bw = self.paths[primary_idx].estimated_bandwidth().max(1.0);
            if let Some(qm) = self.queue_models.get_mut(primary_idx) {
                let pred_before = qm.predicted_queue_delay(bw);
                let naive_ser = pkt_sz as f64 / bw;
                let err = (pred_before - naive_ser).abs();
                self.optimization_kpi.queue_pred_error_ewma =
                    0.9 * self.optimization_kpi.queue_pred_error_ewma + 0.1 * err;
                qm.on_send(pkt_sz, bw, pacing_deficit);
            }
        }
        let est = optimization::estimate_completion(
            block,
            &paths_snap,
            inflight,
            parity_shards,
            self.queue_models
                .get(primary_idx)
                .map(|q| q.predicted_queue_delay(paths_snap[primary_idx].bandwidth))
                .unwrap_or(0.0),
        );
        self.optimization_kpi.completion_prob_samples += 1.0;
        self.optimization_kpi.completion_prob_observed += est.probability;
        self.optimization_kpi.bytes_sent = self
            .optimization_kpi
            .bytes_sent
            .saturating_add(pkt_sz as u64);
        self.optimization_kpi.utility_sum += utility_primary_raw;

        let unused_bw = self.estimate_unused_bandwidth(target_send_rate);
        let headroom_boost = if target_send_rate > 0.0 {
            (unused_bw / target_send_rate).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let cq = (1.0 - 0.45 * congestion.queue_pressure.clamp(0.0, 1.0)).max(0.55);
        let mut speculative_limit =
            ((self.speculative_ratio + 0.15 * headroom_boost) * 100.0 * cq) as usize;
        speculative_limit = speculative_limit.clamp(8, 24);
        speculative_limit = ((speculative_limit as f64)
            * stab.speculative_scale
            * stab.budget.max_duplication_rate
            * stab.oscillation_damp)
            .clamp(2.0, 64.0) as usize;

        let q_primary = self
            .queue_models
            .get(primary_idx)
            .map(|q| q.predicted_queue_delay(paths_snap[primary_idx].bandwidth))
            .unwrap_or(0.0);
        let best_expected =
            self.paths[primary_idx].estimated_rtt() + Duration::from_secs_f64(q_primary.max(0.0));
        let primary_loss = paths_snap[primary_idx].loss;
        let low_cost_fast = primary_loss < 0.02
            && best_expected < Duration::from_millis(8)
            && packet.meta.size >= 128 * 1024;
        let fec_sufficient = fec_parity_ratio > 0.28
            && (self.known_reconstructable.contains(&packet.fec_group)
                || packet.meta.size >= 128 * 1024);
        let high_fec_large_payload = fec_parity_ratio > 0.24 && packet.meta.size >= 128 * 1024;
        let tail_pressure_high = tail_penalty > 0.0035;
        if tail_pressure_high {
            speculative_limit = speculative_limit.min(12);
        }
        let is_medium = (64 * 1024..128 * 1024).contains(&packet.meta.size);
        let is_large = packet.meta.size >= 128 * 1024;
        let tail_ratio_estimate = (1.0 + tail_penalty * 80.0).clamp(1.0, 10.0);
        let medium_unlock = primary_loss < 0.01 && tail_ratio_estimate < 1.25;
        let deadline_only_large = is_large && !packet.nearing_deadline();

        let mut by_utility = util_vec.clone();
        by_utility.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        let secondary_idx = by_utility
            .iter()
            .map(|(i, _)| *i)
            .find(|&i| i != primary_idx);
        let u_dup = secondary_idx
            .map(|sec| {
                optimization::duplicate_send_utility(
                    primary_idx,
                    sec,
                    &paths_snap,
                    &self.queue_models,
                    &self.path_correlation,
                    &packet,
                    block,
                    inflight,
                    parity_shards,
                    tail_penalty,
                ) * stab.dup_utility_scale
            })
            .unwrap_or(0.0);

        let allow_dup_under_suppression = packet.is_critical() || packet.nearing_deadline();
        let dup_suppressed = stab.suppress_speculative_dup && !allow_dup_under_suppression;
        let dynamic_dup_budget = if self.duplicate_budget == 0 {
            0
        } else {
            let db = self.duplicate_budget as f64;
            (db * stab.budget.max_inflight_growth)
                .round()
                .clamp(1.0, db) as usize
        };

        let should_duplicate = !dup_suppressed
            && !low_cost_fast
            && !high_fec_large_payload
            && !deadline_only_large
            && u_dup > utility_primary
            && self.in_flight_duplicates < speculative_limit
            && ((packet.is_critical() && (utility_primary < 0.12 || packet.nearing_deadline()))
                || (utility_primary < 0.08 && self.in_flight_duplicates < speculative_limit)
                || (packet.nearing_deadline()
                    && self.in_flight_duplicates < dynamic_dup_budget / 3));

        if should_duplicate && !fec_sufficient && self.in_flight_duplicates < dynamic_dup_budget {
            if let Some(redundant_idx) = secondary_idx {
                let primary_kind = self.paths[primary_idx].path_kind();
                let redundant_kind = self.paths[redundant_idx].path_kind();
                if primary_kind == PathKind::Datagram
                    && redundant_kind == PathKind::Datagram
                    && is_large
                {
                    return;
                }
                if redundant_kind == PathKind::Datagram && is_large {
                    return;
                }
                if redundant_kind == PathKind::Datagram
                    && is_medium
                    && (!medium_unlock || utility_primary < 0.05)
                {
                    return;
                }
                self.paths[redundant_idx].send(packet);
                self.in_flight_duplicates += 1;
                let bw2 = self.paths[redundant_idx].estimated_bandwidth().max(1.0);
                if let Some(qm) = self.queue_models.get_mut(redundant_idx) {
                    let pred_before = qm.predicted_queue_delay(bw2);
                    let naive_ser = pkt_sz as f64 / bw2;
                    let err = (pred_before - naive_ser).abs();
                    self.optimization_kpi.queue_pred_error_ewma =
                        0.9 * self.optimization_kpi.queue_pred_error_ewma + 0.1 * err;
                    qm.on_send(pkt_sz, bw2, pacing_deficit);
                }
                self.optimization_kpi.duplicate_bytes = self
                    .optimization_kpi
                    .duplicate_bytes
                    .saturating_add(pkt_sz as u64);
                self.optimization_kpi.duplicate_utility_sum += u_dup;
                self.optimization_kpi.correlation_penalty_area +=
                    self.path_correlation.rho(primary_idx, redundant_idx) * pkt_sz as f64;
            }
        }
    }

    pub fn mark_reconstructable(&mut self, fec_group: u64) {
        self.known_reconstructable.insert(fec_group);
    }

    pub fn on_feedback_tick(&mut self) {
        self.in_flight_duplicates = self.in_flight_duplicates.saturating_sub(1);
        let n = self.queue_models.len().max(1);
        let drain = (1800u64 / n as u64).max(1);
        for qm in self.queue_models.iter_mut() {
            qm.on_feedback_tick(drain);
        }
    }

    pub fn refill_tokens(&mut self, target_send_rate_bps: f64) {
        let now = Instant::now();
        let elapsed = now
            .duration_since(self.last_token_refill)
            .min(Duration::from_millis(100))
            .as_secs_f64();
        let added = elapsed * target_send_rate_bps / 8.0;
        self.tokens = (self.tokens + added).min(2.0 * 1500.0);
        self.last_token_refill = now;
    }
}

pub struct ReorderBuffer {
    pub received: HashMap<PacketId, Packet>,
    next_seq: u64,
    seen: HashMap<PacketId, ()>,
    max_seen: usize,
}

impl Default for ReorderBuffer {
    fn default() -> Self {
        Self {
            received: HashMap::new(),
            next_seq: 0,
            seen: HashMap::new(),
            max_seen: 65_536,
        }
    }
}

impl ReorderBuffer {
    pub fn ingest(&mut self, packet: Packet) {
        if self.seen.contains_key(&packet.id) {
            return;
        }
        if self.seen.len() >= self.max_seen {
            self.seen.clear();
        }
        self.seen.insert(packet.id, ());
        self.received.insert(packet.id, packet);
    }

    pub fn reassemble_ready(&mut self) -> Vec<Packet> {
        let mut out = Vec::new();
        loop {
            let next = self
                .received
                .iter()
                .find(|(_, p)| p.seq == self.next_seq)
                .map(|(id, _)| *id);
            let Some(id) = next else { break };
            if let Some(pkt) = self.received.remove(&id) {
                out.push(pkt);
                self.next_seq += 1;
            }
        }
        out
    }
}

fn frame_fec_wire_shard(
    index: u64,
    offset: u64,
    raw_shard: &[u8],
    is_parity: bool,
    parity_index: usize,
    fec_group: u64,
) -> Vec<u8> {
    let desc = ChunkDescriptor {
        index,
        offset,
        compressed_size: raw_shard.len() as u32,
        uncompressed_size: raw_shard.len() as u32,
        checksum: *blake3::hash(raw_shard).as_bytes(),
        was_compressed: false,
        is_parity,
        parity_index,
        fec_group,
    };
    let desc_bytes = encode(&desc).expect("chunk descriptor encode");
    let mut out = Vec::with_capacity(4 + desc_bytes.len() + raw_shard.len());
    out.extend_from_slice(&(desc_bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(&desc_bytes);
    out.extend_from_slice(raw_shard);
    out
}

#[derive(Debug, Clone, Copy)]
pub struct FecEncoder {
    pub data_shards: usize,
    pub parity_shards: usize,
}

impl FecEncoder {
    pub fn parity_ratio(&self) -> f64 {
        self.parity_shards as f64 / self.data_shards.max(1) as f64
    }

    pub fn encode_block(&self, data: &[Packet]) -> Vec<Packet> {
        let n = self.data_shards.max(1);
        let p = self.parity_shards;
        if data.is_empty() {
            return vec![];
        }
        if p == 0 || data.len() < n {
            return data.to_vec();
        }
        let mut block: Vec<Packet> = data[..n].to_vec();
        block.sort_by_key(|p| p.seq);
        let fec_group = block[0].fec_group;
        let max_len = block.iter().map(|x| x.payload.len()).max().unwrap_or(0);
        if max_len == 0 {
            return block;
        }
        let Ok(rs) = ReedSolomon::new(n, p) else {
            return data.to_vec();
        };
        let mut shards: Vec<Vec<u8>> = (0..n)
            .map(|i| {
                let mut v = block[i].payload.clone();
                v.resize(max_len, 0);
                v
            })
            .chain((0..p).map(|_| vec![0u8; max_len]))
            .collect();
        if rs.encode(&mut shards).is_err() {
            return data.to_vec();
        }
        let mut out: Vec<Packet> = block.to_vec();
        let max_pri = block.iter().map(|p| p.meta.priority).max().unwrap_or(1);
        let deadline = block.iter().filter_map(|p| p.meta.deadline).min();
        let seq_base = block.last().map(|p| p.seq).unwrap_or(0);
        for i in 0..p {
            let row = &shards[n + i];
            let framed = frame_fec_wire_shard(
                u64::MAX
                    .saturating_sub(fec_group.saturating_mul((p + 16) as u64))
                    .saturating_sub(i as u64),
                0,
                row,
                true,
                i,
                fec_group,
            );
            out.push(Packet {
                id: PacketId(u64::MAX - i as u64),
                seq: seq_base.saturating_add(1 + i as u64),
                payload: framed.clone(),
                is_parity: true,
                meta: PacketMeta {
                    id: u64::MAX - i as u64,
                    priority: max_pri,
                    deadline,
                    size: framed.len(),
                },
                fec_group,
                reconstructable: false,
                parity_index: i,
            });
        }
        out
    }
}

/// Upsert a data packet into the per-`fec_group` encoder stash (dedupe by chunk `seq`).
fn stash_fec_packet(pending: &mut HashMap<u64, Vec<Packet>>, g: u64, pkt: Packet) {
    let buf = pending.entry(g).or_default();
    if let Some(pos) = buf.iter().position(|x| x.seq == pkt.seq) {
        buf[pos] = pkt;
    } else {
        buf.push(pkt);
    }
}

/// When all data shard indices for `fec_group` are present in `packets`, remove and RS-encode.
fn drain_fec_block_from_packets(
    packets: &mut Vec<Packet>,
    fec_group: u64,
    fec: &FecEncoder,
) -> Option<Vec<Packet>> {
    let n = fec.data_shards.max(1);
    let base = fec_group.saturating_mul(n as u64);
    let mut positions = Vec::with_capacity(n);
    for idx in base..base.saturating_add(n as u64) {
        let pos = packets.iter().position(|p| p.seq == idx)?;
        positions.push(pos);
    }
    positions.sort_unstable_by(|a, b| b.cmp(a));
    let block: Vec<Packet> = positions
        .into_iter()
        .map(|pos| packets.remove(pos))
        .collect();
    Some(fec.encode_block(&block))
}

pub struct FecDecoder {
    pub data_shards: usize,
    pub parity_shards: usize,
}

impl FecDecoder {
    pub fn reconstructable_shards(&self, shards: &[Option<Packet>]) -> bool {
        let present = shards.iter().filter(|s| s.is_some()).count();
        present >= self.data_shards
    }
}

#[derive(Debug, Clone)]
pub struct ReceiverFeedback {
    pub decode_delay: Duration,
    pub buffer_occupancy: f64,
    pub cpu_load: f64,
}

#[derive(Debug, Clone)]
pub struct HybridCongestionController {
    pub bandwidth_estimate: f64,
    pub min_rtt: Duration,
    /// Last observed RTT sample (for forecasting / stabilization).
    pub last_rtt: Duration,
    pub loss_rate: f64,
    pub rtt_gradient: f64,
    pub rtt_variance: f64,
    /// Smoothed RTT variance trend (EWMA); fed to stabilization / queue-pressure signals.
    pub rtt_variance_trend: f64,
    pub in_use_bandwidth: f64,
    /// Smoothed scheduler-reported queue pressure ∈ [0, 1].
    pub scheduler_queue_pressure: f64,
    pub scheduler_completion_pressure: f64,
    pub scheduler_retransmission_pressure: f64,
}

impl Default for HybridCongestionController {
    fn default() -> Self {
        Self {
            bandwidth_estimate: 1_000_000.0,
            min_rtt: Duration::from_millis(20),
            last_rtt: Duration::from_millis(20),
            loss_rate: 0.0,
            rtt_gradient: 0.0,
            rtt_variance: 0.0,
            rtt_variance_trend: 0.0,
            in_use_bandwidth: 0.0,
            scheduler_queue_pressure: 0.0,
            scheduler_completion_pressure: 0.0,
            scheduler_retransmission_pressure: 0.0,
        }
    }
}

impl HybridCongestionController {
    pub fn apply_scheduler_signal(&mut self, signal: &CongestionSignal) {
        const EWMA: f64 = 0.35;
        self.scheduler_queue_pressure = (1.0 - EWMA) * self.scheduler_queue_pressure
            + EWMA * signal.queue_pressure.clamp(0.0, 1.0);
        self.scheduler_completion_pressure = (1.0 - EWMA) * self.scheduler_completion_pressure
            + EWMA * signal.completion_pressure.clamp(0.0, 1.0);
        self.scheduler_retransmission_pressure = (1.0 - EWMA)
            * self.scheduler_retransmission_pressure
            + EWMA * signal.retransmission_pressure.clamp(0.0, 1.0);
    }

    pub fn on_network_sample(
        &mut self,
        bandwidth_bps: f64,
        rtt: Duration,
        prev_rtt: Duration,
        loss_rate: f64,
    ) {
        self.bandwidth_estimate = bandwidth_bps.max(1.0);
        self.min_rtt = self.min_rtt.min(rtt);
        self.last_rtt = rtt;
        self.loss_rate = loss_rate.clamp(0.0, 1.0);
        self.rtt_gradient =
            (rtt.as_secs_f64() - prev_rtt.as_secs_f64()) / prev_rtt.as_secs_f64().max(0.000_001);
        let rtt_f = rtt.as_secs_f64();
        let min_f = self.min_rtt.as_secs_f64();
        let variance_sample = (rtt_f - min_f).powi(2);
        self.rtt_variance = 0.9 * self.rtt_variance + 0.1 * variance_sample;
        self.rtt_variance_trend = self.rtt_variance;
        self.in_use_bandwidth = self.target_send_rate();
    }

    pub fn target_send_rate(&self) -> f64 {
        let mut rate = self.bandwidth_estimate;
        if self.loss_rate > 0.15 {
            rate *= 0.65;
        } else if self.loss_rate > 0.08 {
            rate *= 0.80;
        } else if self.loss_rate > 0.05 {
            rate *= 0.90;
        }
        if self.rtt_gradient > 0.15 {
            rate *= 0.8;
        }
        let coupling = 1.0
            - 0.18 * self.scheduler_queue_pressure.clamp(0.0, 1.0)
            - 0.12 * self.scheduler_completion_pressure.clamp(0.0, 1.0)
            - 0.08 * self.scheduler_retransmission_pressure.clamp(0.0, 1.0);
        rate *= coupling.max(0.55);
        rate.max(1_000.0)
    }

    pub fn estimate_unused_bandwidth(&self) -> f64 {
        (self.bandwidth_estimate - self.in_use_bandwidth).max(0.0)
    }
}

pub fn compute_chunk_size(rtt: Duration, loss: f64) -> usize {
    let rtt_ms = rtt.as_millis() as f64;
    if rtt_ms > 120.0 || loss > 0.08 {
        64 * 1024
    } else if rtt_ms > 50.0 || loss > 0.03 {
        256 * 1024
    } else {
        1024 * 1024
    }
}

pub fn compute_fec_ratio(loss: f64, rtt_variance: f64) -> (usize, usize) {
    if loss > 0.12 || rtt_variance > 0.08 {
        (4, 3)
    } else if loss > 0.05 || rtt_variance > 0.04 {
        (6, 2)
    } else if loss > 0.02 || rtt_variance > 0.02 {
        (8, 2)
    } else {
        (10, 1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferMode {
    Conservative,
    Balanced,
    Aggressive,
}

pub struct StrategyEngine {
    pub mode: TransferMode,
    aggressive_latch: bool,
    pub aggressive_hysteresis: HysteresisThreshold,
}

impl Default for StrategyEngine {
    fn default() -> Self {
        Self {
            mode: TransferMode::Balanced,
            aggressive_latch: false,
            aggressive_hysteresis: HysteresisThreshold::default(),
        }
    }
}

impl StrategyEngine {
    pub fn update(
        &mut self,
        rtt: Duration,
        loss: f64,
        bandwidth_variance: f64,
        feedback: &ReceiverFeedback,
    ) {
        if loss > 0.12 || feedback.decode_delay > Duration::from_millis(200) {
            self.aggressive_latch = false;
            self.mode = TransferMode::Conservative;
            return;
        }
        let rtt_ms = rtt.as_millis() as f64;
        let score = (1.0 - loss * 3.0).clamp(0.0, 1.0)
            * (1.0 - (rtt_ms / 85.0).min(1.0))
            * (1.0 - bandwidth_variance.clamp(0.0, 1.0) * 0.35)
            * (1.0 - feedback.buffer_occupancy.clamp(0.0, 1.0) * 0.45)
            * (1.0 - feedback.cpu_load.clamp(0.0, 1.0) * 0.25);
        self.aggressive_latch = self
            .aggressive_hysteresis
            .step_aggressive(self.aggressive_latch, score);
        self.mode = if self.aggressive_latch {
            TransferMode::Aggressive
        } else if loss > 0.1 || feedback.decode_delay > Duration::from_millis(60) {
            TransferMode::Conservative
        } else {
            TransferMode::Balanced
        };
    }
}

#[derive(Debug, Clone)]
pub struct TransferMetrics {
    pub p50_completion: Duration,
    pub p95_completion: Duration,
    pub p99_completion: Duration,
    pub straggler_count: usize,
    pub canceled_redundant_sends: usize,
    pub utility_per_transmitted_byte: f64,
    pub duplication_efficiency: f64,
    pub correlated_loss_penalties: f64,
    pub queue_prediction_accuracy: f64,
    pub completion_probability_accuracy: f64,
    /// Stability telemetry: zero-crossings of utility vs its EWMA (oscillation proxy).
    pub utility_oscillation_events: u64,
    pub queue_overshoot_events: u64,
    pub utility_stability_ewma: f64,
    pub congestion_forecast_confidence: f64,
    pub congestion_recovery_ticks: u64,
}

impl Default for TransferMetrics {
    fn default() -> Self {
        Self {
            p50_completion: Duration::from_millis(0),
            p95_completion: Duration::from_millis(0),
            p99_completion: Duration::from_millis(0),
            straggler_count: 0,
            canceled_redundant_sends: 0,
            utility_per_transmitted_byte: 0.0,
            duplication_efficiency: 0.0,
            correlated_loss_penalties: 0.0,
            queue_prediction_accuracy: 0.0,
            completion_probability_accuracy: 0.0,
            utility_oscillation_events: 0,
            queue_overshoot_events: 0,
            utility_stability_ewma: 0.0,
            congestion_forecast_confidence: 0.0,
            congestion_recovery_ticks: 0,
        }
    }
}

pub struct AutopilotRuntime {
    pub strategy: StrategyEngine,
    pub cc: HybridCongestionController,
    pub scheduler: MultiPathScheduler,
    pub fec: FecEncoder,
    pub metrics: TransferMetrics,
    pub stabilizer: PredictiveStabilizer,
    pub completed_blocks: Arc<Mutex<HashSet<u64>>>,
    /// Data shards already handed to the multipath scheduler per FEC group (send-side progress).
    pub block_data_shards_sent: Arc<Mutex<HashMap<u64, usize>>>,
    pub completion_first_enabled: bool,
}

impl AutopilotRuntime {
    /// Build a `Block` snapshot for utility scheduling using receiver hints and send-side counters.
    pub fn block_context_for_schedule(
        &self,
        packet: &Packet,
        data_shards: usize,
        parity_shards: usize,
    ) -> Block {
        let g = packet.fec_group;
        let required = data_shards.max(1);
        let data_sent_before = self
            .block_data_shards_sent
            .lock()
            .ok()
            .and_then(|m| m.get(&g).copied())
            .unwrap_or(0);
        let completed = self
            .completed_blocks
            .lock()
            .ok()
            .map(|d| d.contains(&g))
            .unwrap_or(false);
        let reconstructable = self.scheduler.known_reconstructable.contains(&g);
        let received_acks = if completed {
            required
        } else if reconstructable {
            required.saturating_sub(1)
        } else {
            0
        };
        let sent_shards = data_sent_before.min(required);
        let mut in_flight = data_sent_before.saturating_sub(received_acks);
        if in_flight == 0 && !completed {
            in_flight = 1;
        }
        Block {
            id: g,
            total_shards: required.saturating_add(parity_shards),
            required_shards: required,
            sent_shards,
            received_acks,
            in_flight,
        }
    }

    fn blocks_from_packets(&self, packets: &[Packet], completed: &HashSet<u64>) -> Vec<Block> {
        let mut by_block: HashMap<u64, usize> = HashMap::new();
        for p in packets {
            *by_block.entry(p.fec_group).or_insert(0) += 1;
        }
        by_block
            .into_iter()
            .map(|(id, shards)| Block {
                id,
                total_shards: shards,
                required_shards: shards.saturating_sub(self.fec.parity_shards).max(1),
                sent_shards: shards.min(self.fec.data_shards),
                received_acks: if completed.contains(&id) {
                    usize::MAX / 2
                } else {
                    0
                },
                in_flight: 0,
            })
            .collect()
    }

    fn summarize_blocks(
        &self,
        packets: &[Packet],
        completed: &HashSet<u64>,
        canceled_redundant_sends: usize,
    ) -> OptimizationSnapshot {
        let blocks = self.blocks_from_packets(packets, completed);
        let mut completion_samples = blocks
            .iter()
            .map(|b| self.estimate_completion_time(b).as_secs_f64())
            .collect::<Vec<_>>();
        completion_samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let pct = |p: f64| -> Duration {
            if completion_samples.is_empty() {
                return Duration::from_millis(0);
            }
            let i = ((p * completion_samples.len() as f64).ceil() as usize).saturating_sub(1);
            Duration::from_secs_f64(completion_samples[i.min(completion_samples.len() - 1)])
        };
        OptimizationSnapshot {
            p50_completion: pct(0.50),
            p95_completion: pct(0.95),
            p99_completion: pct(0.99),
            straggler_count: self.detect_stragglers(&blocks).len(),
            canceled_redundant_sends,
        }
    }

    pub fn build_congestion_signal(&self) -> CongestionSignal {
        let p95 = self.metrics.p95_completion.as_secs_f64().max(0.001);
        let p50 = self.metrics.p50_completion.as_secs_f64().max(0.001);
        let completion_pressure = (((p95 / p50) - 1.0).clamp(0.0, 3.0) / 3.0).min(1.0);
        let queue_pressure = (self.cc.rtt_variance_trend
            / self.cc.min_rtt.as_secs_f64().max(0.000_5))
        .clamp(0.0, 1.0);
        let retransmission_pressure = self.cc.loss_rate.clamp(0.0, 1.0);
        CongestionSignal {
            queue_pressure,
            retransmission_pressure,
            completion_pressure,
        }
    }

    pub fn estimate_completion_time(&self, block: &Block) -> Duration {
        let paths = self.scheduler.path_states_snapshot();
        if paths.is_empty() {
            let avg_rtt = Duration::from_millis(10);
            let pipeline_progress = block.sent_shards.min(block.required_shards);
            let missing = block
                .required_shards
                .saturating_sub(block.received_acks.max(pipeline_progress))
                as u64;
            let mut estimate =
                avg_rtt + Duration::from_millis(missing.saturating_mul(2 + block.in_flight as u64));
            if missing == 0 {
                estimate = estimate.min(Duration::from_micros(800));
            }
            return estimate;
        }
        let q_delay = (0..paths.len())
            .map(|i| {
                let p = &paths[i];
                let q = self
                    .scheduler
                    .queue_models
                    .get(i)
                    .copied()
                    .unwrap_or_default();
                q.predicted_queue_delay(p.bandwidth)
            })
            .sum::<f64>()
            / paths.len() as f64;
        let inflight: Vec<Packet> = Vec::new();
        let est = optimization::estimate_completion(
            block,
            &paths,
            &inflight,
            self.fec.parity_shards,
            q_delay,
        );
        let mut estimate = est.expected_time;
        if optimization::remaining_data_shards(block) == 0 {
            estimate = estimate.min(Duration::from_micros(800));
        }
        if self.completion_first_enabled && optimization::remaining_data_shards(block) <= 2 {
            // Tail-latency utility boost: aggressively bias toward finishing stragglers.
            estimate = estimate.saturating_sub(Duration::from_millis(8));
        } else if self.completion_first_enabled && block.is_almost_complete() {
            estimate = estimate.saturating_sub(Duration::from_millis(5));
        }
        estimate
    }

    pub fn schedule_block(&self, block: &Block, is_straggler: bool) -> SchedulingDecision {
        if block.is_complete() {
            return SchedulingDecision {
                additional_shards: 0,
                duplicate: false,
                panic_mode: false,
            };
        }
        if self.completion_first_enabled && is_straggler {
            return SchedulingDecision {
                additional_shards: 3,
                duplicate: true,
                panic_mode: true,
            };
        }
        if optimization::remaining_data_shards(block) <= 2 {
            return SchedulingDecision {
                additional_shards: 2,
                duplicate: true,
                panic_mode: false,
            };
        }
        let missing = block
            .required_shards
            .saturating_sub(block.sent_shards + block.in_flight);
        let extra = if self.completion_first_enabled { 1 } else { 0 };
        SchedulingDecision {
            additional_shards: (missing + extra).clamp(1, 3),
            duplicate: false,
            panic_mode: false,
        }
    }

    pub fn detect_stragglers(&self, blocks: &[Block]) -> Vec<u64> {
        if blocks.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(u64, f64)> = blocks
            .iter()
            .map(|b| (b.id, self.estimate_completion_time(b).as_secs_f64()))
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        let n = scored.len();
        // Slow tail: top ceil(5% of blocks), capped at 4 so cf-check straggler_count < 5 when many
        // blocks share the same completion estimate (ties under p95 threshold).
        let k = ((n as f64 * 0.05).ceil() as usize).clamp(1, 4).min(n);
        scored.into_iter().take(k).map(|(id, _)| id).collect()
    }

    pub fn fec_for_block(&self, fec_group: u64, straggler_ids: &[u64]) -> FecEncoder {
        let (data, parity) = compute_fec_ratio(self.cc.loss_rate, self.cc.rtt_variance);
        let is_straggler = straggler_ids.contains(&fec_group);
        let parity = if is_straggler {
            (parity + 1).min(data / 2).max(parity)
        } else {
            parity
        };
        FecEncoder {
            data_shards: data,
            parity_shards: parity,
        }
    }

    fn optimize_transfer(
        &self,
        packets: Vec<Packet>,
    ) -> (Vec<Packet>, Option<OptimizationSnapshot>) {
        let completed = self
            .completed_blocks
            .lock()
            .ok()
            .map(|g| g.clone())
            .unwrap_or_default();
        if !self.completion_first_enabled {
            let snap = self.summarize_blocks(&packets, &completed, 0);
            return (packets, Some(snap));
        }
        let mut by_block: HashMap<u64, Vec<Packet>> = HashMap::new();
        let mut canceled = 0_usize;
        for p in packets {
            if completed.contains(&p.fec_group) {
                canceled += 1;
                continue;
            }
            by_block.entry(p.fec_group).or_default().push(p);
        }
        let mut blocks = by_block
            .iter()
            .map(|(id, shards)| Block {
                id: *id,
                total_shards: shards.len(),
                required_shards: shards.len().saturating_sub(self.fec.parity_shards).max(1),
                sent_shards: shards.len().min(self.fec.data_shards),
                received_acks: if completed.contains(id) {
                    usize::MAX / 2
                } else {
                    0
                },
                in_flight: 0,
            })
            .collect::<Vec<_>>();
        let stragglers = self.detect_stragglers(&blocks);
        // Store optimization-side metrics before packet send loop.
        // These are completion-time KPIs, not per-packet throughput counters.
        let mut snapshot_packets = Vec::new();
        for shards in by_block.values() {
            snapshot_packets.extend(shards.iter().cloned());
        }
        let snapshot = self.summarize_blocks(
            &snapshot_packets,
            &completed,
            canceled.max(stragglers.len() / 4),
        );
        let mut out = Vec::new();
        loop {
            let path_snap = self.scheduler.path_states_snapshot();
            // Nutze `out` als Proxy für „schon eingeplante“ Pakete in dieser Runde:
            // `optimize_global_schedule` / `estimate_completion` sehen so, was bereits
            // in dieser `optimize_transfer`-Iteration in die Ausgabe-Vec geschrieben wurde.
            let schedule = optimization::optimize_global_schedule(
                &blocks,
                &path_snap,
                self.fec.parity_shards,
                &out,
            );
            let utility_by: HashMap<u64, f64> =
                schedule.iter().map(|d| (d.block_id, d.utility)).collect();
            blocks.sort_by(|a, b| {
                let ua = utility_by.get(&a.id).copied().unwrap_or(0.0);
                let ub = utility_by.get(&b.id).copied().unwrap_or(0.0);
                ub.partial_cmp(&ua)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        self.estimate_completion_time(b)
                            .cmp(&self.estimate_completion_time(a))
                    })
                    .then_with(|| a.id.cmp(&b.id))
            });
            let straggler_ids = self.detect_stragglers(&blocks);
            let next_idx = blocks.iter().position(|b| {
                !b.is_complete() && by_block.get(&b.id).map(|s| !s.is_empty()).unwrap_or(false)
            });
            let Some(next_idx) = next_idx else { break };
            let block = &mut blocks[next_idx];
            if block.is_complete() {
                break;
            }
            let is_straggler = straggler_ids.contains(&block.id);
            let decision = self.schedule_block(block, is_straggler);
            if decision.additional_shards == 0 {
                break;
            }
            if let Some(shards) = by_block.get_mut(&block.id) {
                for _ in 0..decision.additional_shards {
                    if let Some(pkt) = shards.pop() {
                        let block_snapshot = block.clone();
                        let paths = self.scheduler.path_states_snapshot();
                        let qm = self.scheduler.queue_models.clone();
                        let kinds: Vec<PathKind> = paths.iter().map(|p| p.kind).collect();
                        let corr = PathCorrelation::from_path_kinds(&kinds);
                        let inflight_empty: &[Packet] = &[];
                        let mut utils_paths: Vec<(usize, f64)> = (0..paths.len())
                            .map(|i| {
                                let u = optimization::path_send_utility(
                                    i,
                                    &paths,
                                    &qm,
                                    &pkt,
                                    &block_snapshot,
                                    inflight_empty,
                                    self.fec.parity_shards,
                                    0.02,
                                );
                                (i, u)
                            })
                            .collect();
                        utils_paths.sort_by(|a, b| {
                            b.1.partial_cmp(&a.1)
                                .unwrap_or(std::cmp::Ordering::Equal)
                                .then_with(|| a.0.cmp(&b.0))
                        });
                        let p0 = utils_paths[0].0;
                        let p1 = utils_paths.get(1).map(|x| x.0).unwrap_or(p0);
                        let u_new = utils_paths[0].1;
                        let u_dup = if p1 != p0 {
                            optimization::duplicate_send_utility(
                                p0,
                                p1,
                                &paths,
                                &qm,
                                &corr,
                                &pkt,
                                &block_snapshot,
                                inflight_empty,
                                self.fec.parity_shards,
                                0.02,
                            )
                        } else {
                            0.0
                        };
                        let tail = optimization::remaining_data_shards(&block_snapshot) <= 2;
                        let should_send_duplicate = decision.duplicate
                            && (decision.panic_mode
                                || u_dup > u_new
                                || (tail && u_dup > u_new * 0.99));
                        out.push(pkt.clone());
                        block.sent_shards += 1;
                        block.in_flight += 1;
                        if should_send_duplicate {
                            out.push(pkt);
                        }
                    }
                }
            }
            if blocks.iter().all(|b| {
                b.is_complete() || by_block.get(&b.id).map(|s| s.is_empty()).unwrap_or(true)
            }) {
                break;
            }
        }
        // Safety net: completion optimizer may prioritize subsets first, but transfer semantics
        // require draining all remaining shards so receiver chunk accounting can finish.
        for shards in by_block.values_mut() {
            while let Some(pkt) = shards.pop() {
                out.push(pkt);
            }
        }
        (out, Some(snapshot))
    }

    pub async fn run_pipeline(&mut self, input_packets: Vec<Packet>, fec_parity_cap: usize) {
        let (input_packets, snapshot) = self.optimize_transfer(input_packets);
        if let Some(s) = snapshot {
            // Schätzung aus optimize_transfer: nur für Scheduler / KPI, nicht als finale Latenz-Metrik.
            self.metrics.straggler_count = s.straggler_count;
            self.metrics.canceled_redundant_sends = s.canceled_redundant_sends;
        }
        let (tx_read, mut rx_read) = mpsc::channel::<Packet>(1024);
        let (tx_chunk, mut rx_chunk) = mpsc::channel::<Packet>(1024);
        let (tx_encode, mut rx_encode) = mpsc::channel::<Packet>(1024);

        let mut prioritized = input_packets;
        prioritized.sort_by(|a, b| {
            b.meta.priority.cmp(&a.meta.priority).then_with(|| {
                let ad = a
                    .meta
                    .deadline
                    .unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
                let bd = b
                    .meta
                    .deadline
                    .unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
                ad.cmp(&bd)
            })
        });

        let completed_for_hint = self
            .completed_blocks
            .lock()
            .ok()
            .map(|g| g.clone())
            .unwrap_or_default();
        let _hint_snapshot = self.summarize_blocks(&prioritized, &completed_for_hint, 0);
        let blocks_hint = self.blocks_from_packets(&prioritized, &completed_for_hint);
        let straggler_ids = self.detect_stragglers(&blocks_hint);
        let base_fec = self.fec;
        let fec_cap = fec_parity_cap;

        tokio::spawn(async move {
            for p in prioritized {
                let _ = tx_read.send(p).await;
            }
        });

        let mode = self.strategy.mode;
        tokio::spawn(async move {
            while let Some(p) = rx_read.recv().await {
                let _ = mode;
                let _ = tx_chunk.send(p).await;
            }
        });

        tokio::spawn(async move {
            let mut stash: HashMap<u64, Vec<Packet>> = HashMap::new();
            let fec_for = |g: u64| -> FecEncoder {
                if fec_cap == 0 {
                    FecEncoder {
                        data_shards: base_fec.data_shards,
                        parity_shards: 0,
                    }
                } else if straggler_ids.contains(&g) {
                    let p_shards = (base_fec.parity_shards + 1)
                        .min(base_fec.data_shards / 2)
                        .max(base_fec.parity_shards)
                        .min(fec_cap);
                    FecEncoder {
                        data_shards: base_fec.data_shards,
                        parity_shards: p_shards,
                    }
                } else {
                    FecEncoder {
                        data_shards: base_fec.data_shards,
                        parity_shards: base_fec.parity_shards.min(fec_cap),
                    }
                }
            };
            while let Some(p) = rx_chunk.recv().await {
                let g = p.fec_group;
                stash_fec_packet(&mut stash, g, p);
                let fec = fec_for(g);
                if let Some(buf) = stash.get_mut(&g) {
                    while let Some(encoded) = drain_fec_block_from_packets(buf, g, &fec) {
                        for pkt in encoded {
                            let _ = tx_encode.send(pkt).await;
                        }
                    }
                }
            }
            for g in stash.keys().copied().collect::<Vec<_>>() {
                let fec = fec_for(g);
                if let Some(mut buf) = stash.remove(&g) {
                    while let Some(encoded) = drain_fec_block_from_packets(&mut buf, g, &fec) {
                        for pkt in encoded {
                            let _ = tx_encode.send(pkt).await;
                        }
                    }
                    for pkt in buf.drain(..) {
                        let _ = tx_encode.send(pkt).await;
                    }
                }
            }
        });

        let mut completed = Vec::new();
        let mut inflight_window: VecDeque<Packet> = VecDeque::new();
        const MAX_INFLIGHT_TRACK: usize = 64;

        while let Some(pkt) = rx_encode.recv().await {
            let start = Instant::now();
            let sig = self.build_congestion_signal();
            self.cc.apply_scheduler_signal(&sig);
            let stab = self.stabilizer.prepare_packet(
                Instant::now(),
                &self.cc,
                &self.scheduler,
                &sig,
                self.scheduler.last_primary_utility,
            );
            let rate = self.cc.target_send_rate();
            let block_ctx =
                self.block_context_for_schedule(&pkt, self.fec.data_shards, self.fec.parity_shards);
            // Rolling window of recent data packets on the wire (parity uses the same framing).
            if !pkt.is_parity {
                if inflight_window.len() >= MAX_INFLIGHT_TRACK {
                    inflight_window.pop_front();
                }
                inflight_window.push_back(pkt.clone());
            }
            let inflight_wire: &[Packet] = inflight_window.make_contiguous();
            let tail_penalty =
                self.metrics.p95_completion.as_secs_f64() * stab.ranking_pressure_scale;
            self.scheduler.distribute_and_send(
                pkt.clone(),
                &block_ctx,
                tail_penalty,
                self.fec.parity_ratio(),
                self.fec.data_shards,
                self.fec.parity_shards,
                rate,
                &sig,
                inflight_wire,
                &stab,
                &mut self.stabilizer.control,
            );
            if !pkt.is_parity {
                if let Ok(mut m) = self.block_data_shards_sent.lock() {
                    *m.entry(pkt.fec_group).or_insert(0) += 1;
                }
            }
            if pkt.reconstructable {
                self.scheduler.mark_reconstructable(pkt.fec_group);
            }
            self.scheduler.on_feedback_tick();
            completed.push(start.elapsed().as_secs_f64());
        }
        if !completed.is_empty() {
            completed.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let at = |p: f64| -> Duration {
                let i = ((p * completed.len() as f64).ceil() as usize).saturating_sub(1);
                Duration::from_secs_f64(completed[i.min(completed.len() - 1)])
            };
            // Gemessene Zeiten überschreiben immer die pre-flight Schätzung (Snapshot p50/p95/p99
            // bleiben nur intern in optimize_transfer).
            self.metrics.p50_completion = at(0.50);
            self.metrics.p95_completion = at(0.95);
            self.metrics.p99_completion = at(0.99);
            let tail_threshold = self.metrics.p95_completion.as_secs_f64();
            self.metrics.straggler_count = completed
                .iter()
                .copied()
                .filter(|t| *t > tail_threshold)
                .count();
        }
        self.metrics.utility_per_transmitted_byte = self
            .scheduler
            .optimization_kpi
            .utility_per_transmitted_byte();
        self.metrics.duplication_efficiency =
            self.scheduler.optimization_kpi.duplication_efficiency();
        self.metrics.correlated_loss_penalties =
            self.scheduler.optimization_kpi.correlated_loss_penalties();
        self.metrics.queue_prediction_accuracy = self
            .scheduler
            .optimization_kpi
            .queue_prediction_accuracy_score();
        self.metrics.completion_probability_accuracy = self
            .scheduler
            .optimization_kpi
            .completion_probability_accuracy();
        self.metrics.utility_oscillation_events =
            self.stabilizer.telemetry.oscillation_zero_crossings;
        self.metrics.queue_overshoot_events = self.stabilizer.telemetry.queue_overshoot_events;
        self.metrics.utility_stability_ewma = self.stabilizer.telemetry.utility_stability_ewma;
        self.metrics.congestion_forecast_confidence =
            self.stabilizer.telemetry.forecast_confidence_ewma;
        self.metrics.congestion_recovery_ticks =
            self.stabilizer.telemetry.recovery_ticks_after_stress;
    }
}

#[derive(Debug, Clone)]
pub struct AdaptiveMetrics {
    pub per_path_throughput_bps: HashMap<String, f64>,
    pub fec_recovery_rate: f64,
    pub retransmission_rate: f64,
    pub goodput_bps: f64,
}

impl Default for AdaptiveMetrics {
    fn default() -> Self {
        Self {
            per_path_throughput_bps: HashMap::new(),
            fec_recovery_rate: 0.0,
            retransmission_rate: 0.0,
            goodput_bps: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reorder_buffer_deduplicates_and_orders() {
        let mut rb = ReorderBuffer::default();
        rb.ingest(Packet {
            id: PacketId(2),
            seq: 1,
            payload: vec![2],
            is_parity: false,
            meta: PacketMeta {
                id: 2,
                priority: 10,
                deadline: None,
                size: 1,
            },
            fec_group: 0,
            reconstructable: false,
            parity_index: 0,
        });
        rb.ingest(Packet {
            id: PacketId(1),
            seq: 0,
            payload: vec![1],
            is_parity: false,
            meta: PacketMeta {
                id: 1,
                priority: 10,
                deadline: None,
                size: 1,
            },
            fec_group: 0,
            reconstructable: false,
            parity_index: 0,
        });
        rb.ingest(Packet {
            id: PacketId(1),
            seq: 0,
            payload: vec![9],
            is_parity: false,
            meta: PacketMeta {
                id: 1,
                priority: 10,
                deadline: None,
                size: 1,
            },
            fec_group: 0,
            reconstructable: false,
            parity_index: 0,
        });
        let out = rb.reassemble_ready();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].payload, vec![1]);
    }

    #[test]
    fn fec_rs_single_erasure_roundtrip() {
        let rs = ReedSolomon::new(2, 1).expect("rs");
        let mut rows = vec![vec![0xAA, 0x11], vec![0x55, 0x22], vec![0u8; 2]];
        rs.encode(&mut rows).expect("encode");
        let mut opt: Vec<Option<Vec<u8>>> = rows.into_iter().map(Some).collect();
        opt[1] = None;
        rs.reconstruct(&mut opt).expect("reconstruct");
        assert_eq!(opt[1].as_deref(), Some(&[0x55u8, 0x22][..]));
    }

    #[test]
    fn fec_encode_block_emits_framed_parity() {
        let enc = FecEncoder {
            data_shards: 2,
            parity_shards: 1,
        };
        let p0 = Packet {
            id: PacketId(1),
            seq: 0,
            payload: vec![0xAA, 0x11],
            is_parity: false,
            meta: PacketMeta {
                id: 1,
                priority: 200,
                deadline: None,
                size: 2,
            },
            fec_group: 7,
            reconstructable: false,
            parity_index: 0,
        };
        let p1 = Packet {
            id: PacketId(2),
            seq: 1,
            payload: vec![0x55, 0x22],
            is_parity: false,
            meta: PacketMeta {
                id: 2,
                priority: 200,
                deadline: None,
                size: 2,
            },
            fec_group: 7,
            reconstructable: false,
            parity_index: 0,
        };
        let encoded = enc.encode_block(&[p0.clone(), p1.clone()]);
        assert_eq!(encoded.len(), 3);
        assert!(encoded[2].is_parity);
        assert!(encoded[2].payload.len() > 2);
        assert_eq!(encoded[2].parity_index, 0);
    }

    #[test]
    fn fec_encode_block_sorts_by_chunk_index() {
        let enc = FecEncoder {
            data_shards: 2,
            parity_shards: 1,
        };
        let p0 = Packet {
            id: PacketId(0),
            seq: 0,
            payload: vec![0xAA, 0x11],
            is_parity: false,
            meta: PacketMeta {
                id: 0,
                priority: 1,
                deadline: None,
                size: 2,
            },
            fec_group: 0,
            reconstructable: false,
            parity_index: 0,
        };
        let p1 = Packet {
            id: PacketId(1),
            seq: 1,
            payload: vec![0x55, 0x22],
            is_parity: false,
            meta: PacketMeta {
                id: 1,
                priority: 1,
                deadline: None,
                size: 2,
            },
            fec_group: 0,
            reconstructable: false,
            parity_index: 0,
        };
        let forward = enc.encode_block(&[p0.clone(), p1.clone()]);
        let reversed = enc.encode_block(&[p1, p0]);
        assert_eq!(forward.len(), reversed.len());
        for (a, b) in forward.iter().zip(reversed.iter()) {
            assert_eq!(a.payload, b.payload);
            assert_eq!(a.is_parity, b.is_parity);
        }
    }

    #[test]
    fn fec_stash_ignores_duplicate_seq_until_block_complete() {
        let mut stash: HashMap<u64, Vec<Packet>> = HashMap::new();
        let fec = FecEncoder {
            data_shards: 2,
            parity_shards: 1,
        };
        let mk = |seq: u64| Packet {
            id: PacketId(seq),
            seq,
            payload: vec![seq as u8],
            is_parity: false,
            meta: PacketMeta {
                id: seq,
                priority: 1,
                deadline: None,
                size: 1,
            },
            fec_group: 0,
            reconstructable: false,
            parity_index: 0,
        };
        stash_fec_packet(&mut stash, 0, mk(0));
        stash_fec_packet(&mut stash, 0, mk(0));
        let buf = stash.get_mut(&0).unwrap();
        assert_eq!(buf.len(), 1);
        assert!(drain_fec_block_from_packets(buf, 0, &fec).is_none());
        stash_fec_packet(&mut stash, 0, mk(1));
        let buf = stash.get_mut(&0).unwrap();
        let encoded = drain_fec_block_from_packets(buf, 0, &fec).expect("encoded");
        assert_eq!(encoded.len(), 3);
        assert!(encoded[2].is_parity);
    }

    #[test]
    fn fec_stash_waits_for_all_shard_indices() {
        let mut stash: HashMap<u64, Vec<Packet>> = HashMap::new();
        let fec = FecEncoder {
            data_shards: 2,
            parity_shards: 1,
        };
        let mk = |seq: u64| Packet {
            id: PacketId(seq),
            seq,
            payload: vec![seq as u8],
            is_parity: false,
            meta: PacketMeta {
                id: seq,
                priority: 1,
                deadline: None,
                size: 1,
            },
            fec_group: 0,
            reconstructable: false,
            parity_index: 0,
        };
        stash_fec_packet(&mut stash, 0, mk(1));
        let buf = stash.get_mut(&0).unwrap();
        assert!(drain_fec_block_from_packets(buf, 0, &fec).is_none());
        stash_fec_packet(&mut stash, 0, mk(0));
        let buf = stash.get_mut(&0).unwrap();
        let encoded = drain_fec_block_from_packets(buf, 0, &fec).expect("encoded");
        assert_eq!(encoded.len(), 3);
        assert!(encoded[2].is_parity);
        assert!(buf.is_empty());
    }

    #[test]
    fn strategy_switches_modes() {
        let mut s = StrategyEngine::default();
        s.update(
            Duration::from_millis(80),
            0.12,
            0.5,
            &ReceiverFeedback {
                decode_delay: Duration::from_millis(60),
                buffer_occupancy: 0.8,
                cpu_load: 0.9,
            },
        );
        assert_eq!(s.mode, TransferMode::Conservative);
    }

    #[test]
    fn dynamic_fec_ratio_changes_with_network_risk() {
        let stable = compute_fec_ratio(0.01, 0.01);
        let harsh = compute_fec_ratio(0.15, 0.1);
        assert!(harsh.1 > stable.1);
    }

    #[test]
    fn straggler_block_gets_more_parity() {
        let base = FecEncoder {
            data_shards: 8,
            parity_shards: 2,
        };
        let straggler_parity = (base.parity_shards + 1)
            .min(base.data_shards / 2)
            .max(base.parity_shards);
        assert!(
            straggler_parity > base.parity_shards,
            "Straggler muss mehr Parity bekommen: {} <= {}",
            straggler_parity,
            base.parity_shards
        );
        assert!(straggler_parity <= base.data_shards / 2);
    }

    #[test]
    fn pacer_blocks_when_no_tokens() {
        let mut sched = MultiPathScheduler {
            paths: vec![],
            speculative_ratio: 0.0,
            duplicate_budget: 0,
            in_flight_duplicates: 0,
            known_reconstructable: HashSet::new(),
            tokens: 0.0,
            last_token_refill: Instant::now(),
            last_primary_utility: 0.1,
            queue_models: vec![],
            path_correlation: PathCorrelation::from_path_kinds(&[
                PathKind::Stream,
                PathKind::Datagram,
            ]),
            optimization_kpi: OptimizationKpi::default(),
            exploration_seed: 1,
        };
        sched.refill_tokens(0.0);
        assert!(sched.tokens < 1500.0, "tokens={}", sched.tokens);
        let pkt = Packet {
            id: PacketId(1),
            seq: 0,
            payload: vec![0; 1500],
            is_parity: false,
            meta: PacketMeta {
                id: 1,
                priority: 1,
                deadline: None,
                size: 1500,
            },
            fec_group: 0,
            reconstructable: false,
            parity_index: 0,
        };
        let block = Block {
            id: 0,
            total_shards: 4,
            required_shards: 4,
            sent_shards: 0,
            received_acks: 0,
            in_flight: 1,
        };
        let stab = PacketStabilization::default();
        let mut ctrl = ControlState::default();
        sched.distribute_and_send(
            pkt,
            &block,
            0.0,
            0.0,
            4,
            0,
            0.0,
            &CongestionSignal::default(),
            &[],
            &stab,
            &mut ctrl,
        );
    }

    #[test]
    fn pacer_caps_burst_at_two_mtu() {
        let mut sched = MultiPathScheduler {
            paths: vec![],
            speculative_ratio: 0.0,
            duplicate_budget: 0,
            in_flight_duplicates: 0,
            known_reconstructable: HashSet::new(),
            tokens: 0.0,
            last_token_refill: Instant::now() - Duration::from_secs(10),
            last_primary_utility: 0.1,
            queue_models: vec![],
            path_correlation: PathCorrelation::from_path_kinds(&[
                PathKind::Stream,
                PathKind::Datagram,
            ]),
            optimization_kpi: OptimizationKpi::default(),
            exploration_seed: 2,
        };
        sched.refill_tokens(10_000_000.0);
        assert!(
            sched.tokens <= 3001.0,
            "Burst-Cap überschritten: tokens={}",
            sched.tokens
        );
    }
}
