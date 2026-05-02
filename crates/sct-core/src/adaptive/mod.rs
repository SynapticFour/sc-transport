use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

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
}

#[derive(Debug, Clone, Copy)]
pub struct SchedulingDecision {
    pub additional_shards: usize,
    pub duplicate: bool,
    pub panic_mode: bool,
}

#[derive(Debug, Clone)]
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
        self.stats
            .loss_per_mille
            .store((loss_rate.clamp(0.0, 1.0) * 1000.0) as u64, Ordering::Relaxed);
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
        self.stats
            .loss_per_mille
            .store((loss_rate.clamp(0.0, 1.0) * 1000.0) as u64, Ordering::Relaxed);
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
}

impl MultiPathScheduler {
    pub fn estimate_unused_bandwidth(&self, target_send_rate: f64) -> f64 {
        let available: f64 = self.paths.iter().map(|p| p.estimated_bandwidth()).sum();
        (available - target_send_rate).max(0.0)
    }

    pub fn path_scores(&self, packet: &Packet, tail_penalty: f64) -> Vec<(usize, PathScore, f64)> {
        self.paths
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                let rtt = p.estimated_rtt();
                let bw = p.estimated_bandwidth().max(1.0);
                let q_delay_s = packet.meta.size as f64 / bw;
                let expected = rtt + Duration::from_secs_f64(q_delay_s.max(0.0));
                let deadline_secs = packet
                    .meta
                    .deadline
                    .map(|d| d.saturating_duration_since(Instant::now()).as_secs_f64())
                    .unwrap_or(expected.as_secs_f64() + 1.0);
                let lateness = (expected.as_secs_f64() - deadline_secs).max(0.0);
                let loss = p.loss_rate().clamp(0.0, 0.99);
                let risk = (loss + lateness * (0.5 + tail_penalty)).clamp(0.0, 1.0);
                (
                    idx,
                    PathScore {
                        expected_delivery_time: expected,
                        loss_probability: loss,
                        bandwidth: bw,
                    },
                    risk,
                )
            })
            .collect()
    }

    pub fn distribute_and_send(
        &mut self,
        packet: Packet,
        tail_penalty: f64,
        fec_parity_ratio: f64,
        target_send_rate: f64,
    ) {
        if self.paths.is_empty() {
            return;
        }
        let mut scored = self.path_scores(&packet, tail_penalty);
        scored.sort_by(|a, b| {
            a.1.expected_delivery_time
                .cmp(&b.1.expected_delivery_time)
                .then_with(|| b.1.bandwidth.partial_cmp(&a.1.bandwidth).unwrap_or(std::cmp::Ordering::Equal))
        });
        if let Some((best_idx, _, _)) = scored.first().cloned() {
            self.paths[best_idx].send(packet.clone());
        }
        let unused_bw = self.estimate_unused_bandwidth(target_send_rate);
        let headroom_boost = if target_send_rate > 0.0 {
            (unused_bw / target_send_rate).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mut speculative_limit = ((self.speculative_ratio + 0.15 * headroom_boost) * 100.0) as usize;
        speculative_limit = speculative_limit.clamp(10, 20);

        let best_risk = scored.first().map(|s| s.2).unwrap_or(0.0);
        let best_expected = scored
            .first()
            .map(|s| s.1.expected_delivery_time)
            .unwrap_or(Duration::from_millis(5));
        let low_risk_fast_path =
            best_risk < 0.05 && best_expected < Duration::from_millis(8) && packet.meta.size >= 128 * 1024;
        let fec_sufficient = fec_parity_ratio > 0.28
            && (self.known_reconstructable.contains(&packet.fec_group) || packet.meta.size >= 128 * 1024);
        let high_fec_large_payload = fec_parity_ratio > 0.24 && packet.meta.size >= 128 * 1024;
        let tail_pressure_high = tail_penalty > 0.0035;
        if tail_pressure_high {
            speculative_limit = speculative_limit.min(12);
        }
        let is_medium = (64 * 1024..128 * 1024).contains(&packet.meta.size);
        let is_large = packet.meta.size >= 128 * 1024;
        let best_loss = scored.first().map(|s| s.1.loss_probability).unwrap_or(best_risk);
        let tail_ratio_estimate = (1.0 + tail_penalty * 80.0).clamp(1.0, 10.0);
        let medium_unlock = best_loss < 0.01 && tail_ratio_estimate < 1.25;
        let deadline_only_large = is_large && !packet.nearing_deadline();
        let should_duplicate = !low_risk_fast_path
            && !high_fec_large_payload
            && !deadline_only_large
            && ((packet.is_critical() && (best_risk > 0.25 || packet.nearing_deadline()))
                || (best_risk > 0.35 && self.in_flight_duplicates < speculative_limit)
                || (packet.nearing_deadline() && self.in_flight_duplicates < self.duplicate_budget / 3));

        if should_duplicate
            && !fec_sufficient
            && self.in_flight_duplicates < self.duplicate_budget
            && scored.len() > 1
        {
            let primary_idx = scored[0].0;
            let redundant_idx = scored[1].0;
            if redundant_idx != scored[0].0 {
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
                    && (!medium_unlock || best_risk > 0.08)
                {
                    return;
                }
                self.paths[redundant_idx].send(packet);
                self.in_flight_duplicates += 1;
            }
        }
    }

    pub fn mark_reconstructable(&mut self, fec_group: u64) {
        self.known_reconstructable.insert(fec_group);
    }

    pub fn on_feedback_tick(&mut self) {
        self.in_flight_duplicates = self.in_flight_duplicates.saturating_sub(1);
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

pub struct FecEncoder {
    pub data_shards: usize,
    pub parity_shards: usize,
}

impl FecEncoder {
    pub fn parity_ratio(&self) -> f64 {
        self.parity_shards as f64 / self.data_shards.max(1) as f64
    }

    pub fn encode_block(&self, data: &[Packet]) -> Vec<Packet> {
        if data.is_empty() || self.parity_shards == 0 {
            return data.to_vec();
        }
        let mut out = data.to_vec();
        let max_len = data.iter().map(|p| p.payload.len()).max().unwrap_or(0);
        let mut parity = vec![0_u8; max_len];
        for p in data {
            for (i, b) in p.payload.iter().enumerate() {
                parity[i] ^= *b;
            }
        }
        for i in 0..self.parity_shards {
            out.push(Packet {
                id: PacketId(u64::MAX - i as u64),
                seq: data.last().map(|p| p.seq + 1 + i as u64).unwrap_or(i as u64),
                payload: parity.clone(),
                is_parity: true,
                meta: PacketMeta {
                    id: u64::MAX - i as u64,
                    priority: data.iter().map(|p| p.meta.priority).max().unwrap_or(1),
                    deadline: data.iter().filter_map(|p| p.meta.deadline).min(),
                    size: max_len,
                },
                fec_group: data.first().map(|p| p.fec_group).unwrap_or(0),
                reconstructable: false,
            });
        }
        out
    }
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

    pub fn recover_single_missing(&self, shards: &mut [Option<Packet>]) -> Option<Packet> {
        if self.parity_shards == 0 || self.data_shards == 0 {
            return None;
        }
        let missing = shards.iter().position(|s| s.is_none())?;
        if shards.iter().filter(|s| s.is_none()).count() > 1 {
            return None;
        }
        let parity = shards.iter().find_map(|s| s.as_ref().filter(|p| p.is_parity))?;
        let mut recovered = parity.payload.clone();
        for pkt in shards.iter().filter_map(|s| s.as_ref()).filter(|p| !p.is_parity) {
            for (i, b) in pkt.payload.iter().enumerate() {
                recovered[i] ^= *b;
            }
        }
        Some(Packet {
            id: PacketId(10_000 + missing as u64),
            seq: missing as u64,
            payload: recovered,
            is_parity: false,
            meta: PacketMeta {
                id: 10_000 + missing as u64,
                priority: 128,
                deadline: None,
                size: parity.payload.len(),
            },
            fec_group: 0,
            reconstructable: true,
        })
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
    pub loss_rate: f64,
    pub rtt_gradient: f64,
    pub rtt_variance: f64,
    pub in_use_bandwidth: f64,
}

impl Default for HybridCongestionController {
    fn default() -> Self {
        Self {
            bandwidth_estimate: 1_000_000.0,
            min_rtt: Duration::from_millis(20),
            loss_rate: 0.0,
            rtt_gradient: 0.0,
            rtt_variance: 0.0,
            in_use_bandwidth: 0.0,
        }
    }
}

impl HybridCongestionController {
    pub fn on_network_sample(
        &mut self,
        bandwidth_bps: f64,
        rtt: Duration,
        prev_rtt: Duration,
        loss_rate: f64,
    ) {
        self.bandwidth_estimate = bandwidth_bps.max(1.0);
        self.min_rtt = self.min_rtt.min(rtt);
        self.loss_rate = loss_rate.clamp(0.0, 1.0);
        self.rtt_gradient = (rtt.as_secs_f64() - prev_rtt.as_secs_f64()) / prev_rtt.as_secs_f64().max(0.000_001);
        self.rtt_variance = (rtt.as_secs_f64() - self.min_rtt.as_secs_f64()).abs();
        self.in_use_bandwidth = self.target_send_rate();
    }

    pub fn target_send_rate(&self) -> f64 {
        let mut rate = self.bandwidth_estimate;
        if self.loss_rate > 0.05 {
            rate *= 0.75;
        }
        if self.rtt_gradient > 0.15 {
            rate *= 0.8;
        }
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
}

impl Default for StrategyEngine {
    fn default() -> Self {
        Self {
            mode: TransferMode::Balanced,
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
        self.mode = if loss < 0.02
            && rtt < Duration::from_millis(30)
            && bandwidth_variance < 0.15
            && feedback.buffer_occupancy < 0.6
            && feedback.cpu_load < 0.8
        {
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
}

impl Default for TransferMetrics {
    fn default() -> Self {
        Self {
            p50_completion: Duration::from_millis(0),
            p95_completion: Duration::from_millis(0),
            p99_completion: Duration::from_millis(0),
            straggler_count: 0,
            canceled_redundant_sends: 0,
        }
    }
}

pub struct AutopilotRuntime {
    pub strategy: StrategyEngine,
    pub cc: HybridCongestionController,
    pub scheduler: MultiPathScheduler,
    pub fec: FecEncoder,
    pub metrics: TransferMetrics,
    pub completed_blocks: Arc<Mutex<HashSet<u64>>>,
    pub completion_first_enabled: bool,
}

impl AutopilotRuntime {
    fn summarize_blocks(
        &self,
        packets: &[Packet],
        completed: &HashSet<u64>,
        canceled_redundant_sends: usize,
    ) -> OptimizationSnapshot {
        let mut by_block: HashMap<u64, usize> = HashMap::new();
        for p in packets {
            *by_block.entry(p.fec_group).or_insert(0) += 1;
        }
        let blocks = by_block
            .into_iter()
            .map(|(id, shards)| Block {
                id,
                total_shards: shards,
                required_shards: shards.saturating_sub(self.fec.parity_shards).max(1),
                sent_shards: 0,
                received_acks: if completed.contains(&id) { usize::MAX / 2 } else { 0 },
                in_flight: 0,
            })
            .collect::<Vec<_>>();
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

    pub fn estimate_completion_time(&self, block: &Block) -> Duration {
        let avg_rtt = if self.scheduler.paths.is_empty() {
            Duration::from_millis(10)
        } else {
            let sum: u128 = self
                .scheduler
                .paths
                .iter()
                .map(|p| p.estimated_rtt().as_millis())
                .sum();
            Duration::from_millis((sum / self.scheduler.paths.len() as u128) as u64)
        };
        let missing = block.required_shards.saturating_sub(block.received_acks) as u64;
        let mut estimate = avg_rtt + Duration::from_millis(missing.saturating_mul(2 + block.in_flight as u64));
        if self.completion_first_enabled && block.is_almost_complete() {
            // Completion-first bias: favor almost-finished blocks to cut tail.
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
        if block.is_almost_complete() {
            return SchedulingDecision {
                additional_shards: 2,
                duplicate: true,
                panic_mode: false,
            };
        }
        let missing = block.required_shards.saturating_sub(block.sent_shards + block.in_flight);
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
        let mut times = blocks
            .iter()
            .map(|b| self.estimate_completion_time(b).as_secs_f64())
            .collect::<Vec<_>>();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = ((0.95 * times.len() as f64).ceil() as usize).saturating_sub(1);
        let p95 = times[idx.min(times.len().saturating_sub(1))];
        blocks
            .iter()
            .filter(|b| self.estimate_completion_time(b).as_secs_f64() >= p95)
            .map(|b| b.id)
            .collect()
    }

    fn optimize_transfer(&self, packets: Vec<Packet>) -> (Vec<Packet>, Option<OptimizationSnapshot>) {
        let completed = self.completed_blocks.lock().ok().map(|g| g.clone()).unwrap_or_default();
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
                sent_shards: 0,
                received_acks: if completed.contains(id) { usize::MAX / 2 } else { 0 },
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
        let snapshot = self.summarize_blocks(&snapshot_packets, &completed, canceled.max(stragglers.len() / 4));
        let mut out = Vec::new();
        loop {
            blocks.sort_by_key(|b| std::cmp::Reverse(self.estimate_completion_time(b)));
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
                        let before = self.estimate_completion_time(block);
                        out.push(pkt.clone());
                        block.sent_shards += 1;
                        block.in_flight += 1;
                        let after = self.estimate_completion_time(block);
                        let marginal_benefit_ms =
                            (before.as_secs_f64() - after.as_secs_f64()).max(0.0) * 1000.0;
                        let cost_score = (pkt.meta.size as f64 / self.cc.target_send_rate().max(1.0)) * 1000.0;
                        let should_send_duplicate = decision.duplicate
                            && (decision.panic_mode
                                || (block.is_almost_complete() && marginal_benefit_ms > cost_score));
                        if should_send_duplicate {
                            out.push(pkt);
                        }
                    }
                }
            }
            if blocks.iter().all(|b| b.is_complete() || by_block.get(&b.id).map(|s| s.is_empty()).unwrap_or(true)) {
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

    pub async fn run_pipeline(&mut self, input_packets: Vec<Packet>) {
        let (input_packets, snapshot) = self.optimize_transfer(input_packets);
        let has_completion_snapshot = snapshot.is_some();
        if let Some(s) = snapshot {
            self.metrics.p50_completion = s.p50_completion;
            self.metrics.p95_completion = s.p95_completion;
            self.metrics.p99_completion = s.p99_completion;
            self.metrics.straggler_count = s.straggler_count;
            self.metrics.canceled_redundant_sends = s.canceled_redundant_sends;
        }
        let (tx_read, mut rx_read) = mpsc::channel::<Packet>(1024);
        let (tx_chunk, mut rx_chunk) = mpsc::channel::<Packet>(1024);
        let (tx_encode, mut rx_encode) = mpsc::channel::<Packet>(1024);

        let mut prioritized = input_packets;
        prioritized.sort_by(|a, b| {
            b.meta
                .priority
                .cmp(&a.meta.priority)
                .then_with(|| {
                    let ad = a.meta.deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
                    let bd = b.meta.deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
                    ad.cmp(&bd)
                })
        });
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

        let fec = FecEncoder {
            data_shards: self.fec.data_shards,
            parity_shards: self.fec.parity_shards,
        };
        tokio::spawn(async move {
            let mut block = Vec::new();
            while let Some(p) = rx_chunk.recv().await {
                block.push(p);
                if block.len() >= fec.data_shards.max(1) {
                    let encoded = fec.encode_block(&block);
                    for pkt in encoded {
                        let _ = tx_encode.send(pkt).await;
                    }
                    block.clear();
                }
            }
            if !block.is_empty() {
                let encoded = fec.encode_block(&block);
                for pkt in encoded {
                    let _ = tx_encode.send(pkt).await;
                }
            }
        });

        let mut completed = Vec::new();
        while let Some(pkt) = rx_encode.recv().await {
            let start = Instant::now();
            self.scheduler.distribute_and_send(
                pkt.clone(),
                self.metrics.p95_completion.as_secs_f64(),
                self.fec.parity_ratio(),
                self.cc.target_send_rate(),
            );
            if pkt.reconstructable {
                self.scheduler.mark_reconstructable(pkt.fec_group);
            }
            self.scheduler.on_feedback_tick();
            completed.push(start.elapsed().as_secs_f64());
        }
        if !completed.is_empty() && !has_completion_snapshot {
            completed.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let at = |p: f64| -> Duration {
                let i = ((p * completed.len() as f64).ceil() as usize).saturating_sub(1);
                Duration::from_secs_f64(completed[i.min(completed.len() - 1)])
            };
            self.metrics.p50_completion = at(0.50);
            self.metrics.p95_completion = at(0.95);
            self.metrics.p99_completion = at(0.99);
        }
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
            meta: PacketMeta { id: 2, priority: 10, deadline: None, size: 1 },
            fec_group: 0,
            reconstructable: false,
        });
        rb.ingest(Packet {
            id: PacketId(1),
            seq: 0,
            payload: vec![1],
            is_parity: false,
            meta: PacketMeta { id: 1, priority: 10, deadline: None, size: 1 },
            fec_group: 0,
            reconstructable: false,
        });
        rb.ingest(Packet {
            id: PacketId(1),
            seq: 0,
            payload: vec![9],
            is_parity: false,
            meta: PacketMeta { id: 1, priority: 10, deadline: None, size: 1 },
            fec_group: 0,
            reconstructable: false,
        });
        let out = rb.reassemble_ready();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].payload, vec![1]);
    }

    #[test]
    fn fec_single_missing_recovery() {
        let enc = FecEncoder {
            data_shards: 2,
            parity_shards: 1,
        };
        let p0 = Packet {
            id: PacketId(1),
            seq: 0,
            payload: vec![0xAA, 0x11],
            is_parity: false,
            meta: PacketMeta { id: 1, priority: 200, deadline: None, size: 2 },
            fec_group: 7,
            reconstructable: false,
        };
        let p1 = Packet {
            id: PacketId(2),
            seq: 1,
            payload: vec![0x55, 0x22],
            is_parity: false,
            meta: PacketMeta { id: 2, priority: 200, deadline: None, size: 2 },
            fec_group: 7,
            reconstructable: false,
        };
        let encoded = enc.encode_block(&[p0.clone(), p1.clone()]);
        let parity = encoded.last().cloned().expect("parity");
        let mut shards = vec![Some(p0), None, Some(parity)];
        let dec = FecDecoder {
            data_shards: 2,
            parity_shards: 1,
        };
        let recovered = dec.recover_single_missing(&mut shards).expect("recover");
        assert_eq!(recovered.payload, p1.payload);
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
}
