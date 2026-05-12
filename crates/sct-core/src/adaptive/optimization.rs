//! Utility-based scheduling and completion modeling for multipath transport.
//!
//! Decisions maximize **expected completion-time reduction per bandwidth-second**,
//! not raw throughput. Path correlation down-weights redundant sends on coupled routes.

use std::time::Duration;

use super::{Block, Packet, PathKind};

/// Expected block completion from the scheduler’s perspective.
#[derive(Debug, Clone)]
pub struct CompletionEstimate {
    pub probability: f64,
    pub expected_time: Duration,
}

/// Per-path snapshot used by the optimizer (no trait objects).
#[derive(Debug, Clone, Copy)]
pub struct PathState {
    pub rtt: Duration,
    pub bandwidth: f64,
    pub loss: f64,
    pub kind: PathKind,
}

/// Pairwise path correlation ρ ∈ [0, 1]; diagonal is 1.0.
#[derive(Debug, Clone)]
pub struct PathCorrelation {
    pub correlation_matrix: Vec<Vec<f64>>,
}

impl PathCorrelation {
    pub fn from_path_kinds(kinds: &[PathKind]) -> Self {
        let n = kinds.len();
        let mut m = vec![vec![0.0f64; n]; n];
        for i in 0..n {
            m[i][i] = 1.0;
            for j in (i + 1)..n {
                let rho = match (kinds[i], kinds[j]) {
                    // Stream + datagram on the same logical route share fate (Wi‑Fi, last mile).
                    (PathKind::Stream, PathKind::Datagram) | (PathKind::Datagram, PathKind::Stream) => {
                        0.82
                    }
                    (PathKind::Stream, PathKind::Stream) => 0.68,
                    (PathKind::Datagram, PathKind::Datagram) => 0.58,
                };
                m[i][j] = rho;
                m[j][i] = rho;
            }
        }
        Self {
            correlation_matrix: m,
        }
    }

    pub fn rho(&self, i: usize, j: usize) -> f64 {
        self.correlation_matrix
            .get(i)
            .and_then(|row| row.get(j))
            .copied()
            .unwrap_or(0.0)
    }
}

/// Predictive queue / pacing backlog model per path.
#[derive(Debug, Clone, Copy)]
pub struct QueueModel {
    pub ewma_delay: f64,
    pub inflight_bytes: u64,
    pub pacing_backlog: u64,
}

impl Default for QueueModel {
    fn default() -> Self {
        Self {
            ewma_delay: 0.0,
            inflight_bytes: 0,
            pacing_backlog: 0,
        }
    }
}

impl QueueModel {
    pub fn predicted_queue_delay(&self, bandwidth_bps: f64) -> f64 {
        let bw = bandwidth_bps.max(1.0);
        let ser = self.inflight_bytes as f64 / bw;
        let pacing_penalty = (self.pacing_backlog as f64 / bw).min(0.05);
        self.ewma_delay + ser + pacing_penalty
    }

    pub fn on_send(&mut self, bytes: usize, bandwidth_bps: f64, pacing_tokens_deficit: f64) {
        let bw = bandwidth_bps.max(1.0);
        let inst = bytes as f64 / bw;
        const ALPHA: f64 = 0.22;
        self.ewma_delay = (1.0 - ALPHA) * self.ewma_delay + ALPHA * inst;
        self.inflight_bytes = self.inflight_bytes.saturating_add(bytes as u64);
        self.pacing_backlog = pacing_tokens_deficit.clamp(0.0, 2.0 * 1500.0) as u64;
    }

    pub fn on_feedback_tick(&mut self, drain_bytes: u64) {
        self.inflight_bytes = self.inflight_bytes.saturating_sub(drain_bytes);
    }
}

/// Pressures exposed by the scheduler to couple congestion control.
#[derive(Debug, Clone, Copy, Default)]
pub struct CongestionSignal {
    pub queue_pressure: f64,
    pub retransmission_pressure: f64,
    pub completion_pressure: f64,
}

/// One block’s priority in the global schedule.
#[derive(Debug, Clone)]
pub struct TransmissionDecision {
    pub block_id: u64,
    pub utility: f64,
    pub favor_duplicate: bool,
}

/// Running KPIs for telemetry (updated incrementally on each send).
#[derive(Debug, Clone, Default)]
pub struct OptimizationKpi {
    pub bytes_sent: u64,
    pub utility_sum: f64,
    pub duplicate_bytes: u64,
    pub duplicate_utility_sum: f64,
    pub correlation_penalty_area: f64,
    pub queue_pred_error_ewma: f64,
    pub completion_prob_samples: f64,
    pub completion_prob_observed: f64,
}

impl OptimizationKpi {
    pub fn utility_per_transmitted_byte(&self) -> f64 {
        if self.bytes_sent == 0 {
            return 0.0;
        }
        self.utility_sum / self.bytes_sent as f64
    }

    pub fn duplication_efficiency(&self) -> f64 {
        if self.duplicate_bytes == 0 {
            return 0.0;
        }
        self.duplicate_utility_sum / self.duplicate_bytes as f64
    }

    pub fn correlated_loss_penalties(&self) -> f64 {
        self.correlation_penalty_area
    }

    pub fn queue_prediction_accuracy_score(&self) -> f64 {
        // Higher is better: inverse of smoothed absolute error (seconds).
        1.0 / (1.0 + self.queue_pred_error_ewma * 1000.0)
    }

    pub fn completion_probability_accuracy(&self) -> f64 {
        if self.completion_prob_samples <= 0.0 {
            return 0.0;
        }
        1.0 - (self.completion_prob_observed / self.completion_prob_samples).abs().min(1.0)
    }
}

/// ρ‑adjusted joint failure probability for two paths (copula-style mixing).
pub fn joint_failure_probability(p1: f64, p2: f64, rho: f64) -> f64 {
    let p1 = p1.clamp(0.0, 1.0);
    let p2 = p2.clamp(0.0, 1.0);
    let rho = rho.clamp(0.0, 1.0);
    let independent = p1 * p2;
    let corr = rho * (p1 * (1.0 - p1)).sqrt() * (p2 * (1.0 - p2)).sqrt();
    (independent + corr).min(1.0)
}

fn effective_paths(paths: &[PathState]) -> f64 {
    paths.len().max(1) as f64
}

/// Remaining data shards still needed for decode (conservative).
pub fn remaining_data_shards(block: &Block) -> usize {
    let pipeline = block.sent_shards.min(block.required_shards);
    block
        .required_shards
        .saturating_sub(block.received_acks.max(pipeline))
}

/// Estimate completion probability and expected time from loss, RTT, queue, FEC, and inflight.
pub fn estimate_completion(
    block: &Block,
    paths: &[PathState],
    inflight: &[Packet],
    parity_shards: usize,
    queue_delay_s: f64,
) -> CompletionEstimate {
    if paths.is_empty() {
        return CompletionEstimate {
            probability: 0.0,
            expected_time: Duration::from_secs(3600),
        };
    }
    let missing = remaining_data_shards(block).max(block.in_flight.min(block.required_shards));
    let inflight_same = inflight
        .iter()
        .filter(|p| p.fec_group == block.id)
        .count();
    let avg_rtt_s: f64 = paths
        .iter()
        .map(|p| p.rtt.as_secs_f64().max(0.000_01))
        .sum::<f64>()
        / effective_paths(paths);
    let avg_loss: f64 = paths.iter().map(|p| p.loss).sum::<f64>() / effective_paths(paths);
    let p_ok = (1.0 - avg_loss).clamp(0.02, 1.0);
    let parity_relief = (parity_shards as f64 * 0.12).min(0.45);
    let rounds = (missing as f64) / (p_ok * (1.0 + parity_relief)).max(0.15);
    let inflight_factor = 1.0 + 0.18 * (inflight_same as f64) + 0.12 * (block.in_flight as f64);
    let expected_s = (avg_rtt_s * rounds * inflight_factor + queue_delay_s).max(0.0);
    let prob = (1.0 - avg_loss.powf((missing as f64 + 1.0).min(32.0))).clamp(0.0, 1.0);
    CompletionEstimate {
        probability: (prob + parity_relief * 0.25).min(1.0),
        expected_time: Duration::from_secs_f64(expected_s),
    }
}

/// Global ordering over blocks: higher utility first; `favor_duplicate` for tail / stragglers.
pub fn optimize_global_schedule(
    blocks: &[Block],
    paths: &[PathState],
    parity_shards: usize,
    inflight: &[Packet],
) -> Vec<TransmissionDecision> {
    let mut out: Vec<TransmissionDecision> = blocks
        .iter()
        .map(|b| {
            let q = paths
                .first()
                .map(|p| b.in_flight as f64 * p.bandwidth.recip() * 1500.0)
                .unwrap_or(0.0);
            let est = estimate_completion(b, paths, inflight, parity_shards, q);
            let rem = remaining_data_shards(b);
            let tail = rem <= 2;
            let time_s = est.expected_time.as_secs_f64().max(1e-6);
            let mut u = est.probability / time_s;
            if tail {
                u *= 1.85;
            }
            TransmissionDecision {
                block_id: b.id,
                utility: u,
                favor_duplicate: tail || rem <= (b.required_shards / 8).max(1),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.utility
            .partial_cmp(&a.utility)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.block_id.cmp(&b.block_id))
    });
    out
}

fn tail_urgency_multiplier(block: &Block, tail_penalty: f64) -> f64 {
    let rem = remaining_data_shards(block);
    let near_done = rem <= 2;
    let base = 1.0 + tail_penalty * 40.0;
    if near_done {
        base * 1.65
    } else {
        base
    }
}

/// Expected completion‑time reduction (seconds) from placing one more copy on `path_idx`.
fn marginal_time_reduction_s(
    block: &Block,
    paths: &[PathState],
    inflight: &[Packet],
    parity_shards: usize,
    path_idx: usize,
    tail_penalty: f64,
    queue_delay_s: f64,
) -> f64 {
    let base = estimate_completion(block, paths, inflight, parity_shards, queue_delay_s);
    let mut b2 = block.clone();
    b2.in_flight = b2.in_flight.saturating_add(1);
    let after = estimate_completion(&b2, paths, inflight, parity_shards, queue_delay_s);
    let raw = base.expected_time.as_secs_f64() - after.expected_time.as_secs_f64();
    let p = paths
        .get(path_idx)
        .map(|x| 1.0 - x.loss)
        .unwrap_or(0.5)
        .clamp(0.05, 1.0);
    raw.max(0.0) * p * tail_urgency_multiplier(block, tail_penalty)
}

/// Bandwidth cost in seconds: serialization + predicted queue / pacing.
pub fn bandwidth_cost_s(packet_bytes: f64, bandwidth_bps: f64, queue_delay_s: f64) -> f64 {
    let ser = packet_bytes / bandwidth_bps.max(1.0);
    (ser + queue_delay_s).max(1e-9)
}

/// Per‑path utility for sending `packet` on `path_idx` for `block`.
#[allow(clippy::too_many_arguments)] // Scheduling context is inherently multi‑factor.
pub fn path_send_utility(
    path_idx: usize,
    paths: &[PathState],
    queue_models: &[QueueModel],
    packet: &Packet,
    block: &Block,
    inflight: &[Packet],
    parity_shards: usize,
    tail_penalty: f64,
) -> f64 {
    let Some(p) = paths.get(path_idx) else {
        return 0.0;
    };
    let qm = queue_models.get(path_idx).copied().unwrap_or_default();
    let q_delay = qm.predicted_queue_delay(p.bandwidth);
    let reduction = marginal_time_reduction_s(
        block,
        paths,
        inflight,
        parity_shards,
        path_idx,
        tail_penalty,
        q_delay,
    );
    let cost = bandwidth_cost_s(packet.meta.size as f64, p.bandwidth, q_delay);
    // Reliability shrinks expected useful work; keeps utility monotone in loss for typical blocks.
    let reliability = (1.0 - p.loss).powf(1.35).max(0.06);
    let retransmission_load = 1.0 + 1.6 * p.loss;
    reduction * reliability / (cost * retransmission_load)
}

/// Utility of duplicating onto `secondary_idx` given primary `primary_idx`.
#[allow(clippy::too_many_arguments)] // Mirrors path/correlation inputs used by the scheduler.
pub fn duplicate_send_utility(
    primary_idx: usize,
    secondary_idx: usize,
    paths: &[PathState],
    queue_models: &[QueueModel],
    correlation: &PathCorrelation,
    packet: &Packet,
    block: &Block,
    inflight: &[Packet],
    parity_shards: usize,
    tail_penalty: f64,
) -> f64 {
    if primary_idx == secondary_idx {
        return 0.0;
    }
    let p0 = paths[primary_idx].loss.clamp(0.0, 0.999);
    let p1 = paths[secondary_idx].loss.clamp(0.0, 0.999);
    let rho = correlation.rho(primary_idx, secondary_idx).clamp(0.0, 1.0);
    let jfail = joint_failure_probability(p0, p1, rho);
    let p_single_fail = p0;
    let marginal_survival = (1.0 - jfail) - (1.0 - p_single_fail);
    let est = estimate_completion(
        block,
        paths,
        inflight,
        parity_shards,
        queue_models
            .get(primary_idx)
            .map(|q| q.predicted_queue_delay(paths[primary_idx].bandwidth))
            .unwrap_or(0.0),
    );
    let reduction = marginal_survival.max(0.0)
        * est.expected_time.as_secs_f64()
        * 0.22
        * tail_urgency_multiplier(block, tail_penalty)
        * (1.0 - 0.55 * rho);
    let qm = queue_models.get(secondary_idx).copied().unwrap_or_default();
    let q_delay = qm.predicted_queue_delay(paths[secondary_idx].bandwidth);
    let cost = bandwidth_cost_s(packet.meta.size as f64, paths[secondary_idx].bandwidth, q_delay);
    reduction / cost.max(1e-9)
}

/// ε‑greedy index selection: returns `true` if exploration branch should be taken.
pub fn epsilon_greedy_explore(epsilon: f64, rng_u01: f64) -> bool {
    rng_u01 < epsilon.clamp(0.0, 0.5)
}

fn xorshift_u01(seed: &mut u64) -> f64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    (*seed as f64 / u64::MAX as f64).clamp(0.0, 1.0)
}

/// Pick primary path index: best utility with occasional second‑best exploration.
pub(crate) fn pick_primary_path(
    utilities: &[(usize, f64)],
    exploration_seed: &mut u64,
    epsilon: f64,
) -> Option<usize> {
    if utilities.is_empty() {
        return None;
    }
    let mut sorted = utilities.to_vec();
    sorted.sort_by(|a, b| {
        b.1
            .partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    if sorted.len() == 1 {
        return Some(sorted[0].0);
    }
    let u = xorshift_u01(exploration_seed);
    if epsilon_greedy_explore(epsilon, u) {
        Some(sorted[1].0)
    } else {
        Some(sorted[0].0)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Packet, PacketId, PacketMeta};
    use super::*;
    use std::time::Duration;

    fn sample_paths() -> Vec<PathState> {
        vec![
            PathState {
                rtt: Duration::from_millis(25),
                bandwidth: 50_000_000.0,
                loss: 0.02,
                kind: PathKind::Stream,
            },
            PathState {
                rtt: Duration::from_millis(30),
                bandwidth: 45_000_000.0,
                loss: 0.04,
                kind: PathKind::Datagram,
            },
        ]
    }

    #[test]
    fn loss_weighting_is_monotone() {
        let w = |loss: f64| {
            let reliability = (1.0 - loss).powf(1.35).max(0.06);
            let retransmission_load = 1.0 + 1.6 * loss;
            reliability / retransmission_load
        };
        assert!(w(0.02) > w(0.38));
    }

    #[test]
    fn path_send_utility_is_non_negative_for_reasonable_inputs() {
        let paths = vec![PathState {
            rtt: Duration::from_millis(25),
            bandwidth: 50_000_000.0,
            loss: 0.02,
            kind: PathKind::Stream,
        }];
        let block = Block {
            id: 1,
            total_shards: 10,
            required_shards: 8,
            sent_shards: 0,
            received_acks: 0,
            in_flight: 2,
        };
        let pkt = Packet {
            id: PacketId(1),
            seq: 0,
            payload: vec![0; 1400],
            is_parity: false,
            meta: PacketMeta {
                id: 1,
                priority: 100,
                deadline: None,
                size: 1400,
            },
            fec_group: 1,
            reconstructable: false,
        };
        let q = [QueueModel::default()];
        let u = path_send_utility(0, &paths, &q, &pkt, &block, &[], 1, 0.01);
        assert!(u.is_finite() && u >= 0.0);
    }

    #[test]
    fn correlation_reduces_duplicate_utility() {
        let paths = sample_paths();
        let corr_high = PathCorrelation::from_path_kinds(&[PathKind::Stream, PathKind::Datagram]);
        let mut corr_low = corr_high.clone();
        for i in 0..corr_low.correlation_matrix.len() {
            for j in 0..corr_low.correlation_matrix.len() {
                if i != j {
                    corr_low.correlation_matrix[i][j] *= 0.2;
                }
            }
        }
        let block = Block {
            id: 2,
            total_shards: 6,
            required_shards: 4,
            sent_shards: 0,
            received_acks: 1,
            in_flight: 1,
        };
        let pkt = Packet {
            id: PacketId(2),
            seq: 0,
            payload: vec![0; 1200],
            is_parity: false,
            meta: PacketMeta {
                id: 2,
                priority: 100,
                deadline: None,
                size: 1200,
            },
            fec_group: 2,
            reconstructable: false,
        };
        let q = [QueueModel::default(), QueueModel::default()];
        let inflight: Vec<Packet> = vec![];
        let u_high = duplicate_send_utility(0, 1, &paths, &q, &corr_high, &pkt, &block, &inflight, 1, 0.02);
        let u_low = duplicate_send_utility(0, 1, &paths, &q, &corr_low, &pkt, &block, &inflight, 1, 0.02);
        assert!(u_low > u_high, "high correlation should shrink duplicate utility");
    }

    #[test]
    fn global_schedule_orders_by_utility() {
        let paths = sample_paths();
        let inflight: Vec<Packet> = vec![];
        let blocks = vec![
            Block {
                id: 10,
                total_shards: 8,
                required_shards: 8,
                sent_shards: 0,
                received_acks: 6,
                in_flight: 1,
            },
            Block {
                id: 11,
                total_shards: 8,
                required_shards: 8,
                sent_shards: 0,
                received_acks: 0,
                in_flight: 0,
            },
        ];
        let d = optimize_global_schedule(&blocks, &paths, 1, &inflight);
        assert_eq!(d[0].block_id, 10, "near-complete block should rank first");
    }
}
