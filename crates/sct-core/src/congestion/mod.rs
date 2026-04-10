use quinn::congestion::{Controller, ControllerFactory, ControllerMetrics};
use quinn_proto::RttEstimator;
use std::any::Any;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct SciBbrConfig {
    pub startup_gain: f64,
    pub drain_gain: f64,
    pub cwnd_gain: f64,
    pub probe_rtt_cwnd_gain: f64,
    pub initial_cwnd_packets: u64,
    pub target_cwnd_headroom: f64,
}

impl Default for SciBbrConfig {
    fn default() -> Self {
        Self {
            startup_gain: 2.885,
            drain_gain: 0.35,
            cwnd_gain: 2.0,
            probe_rtt_cwnd_gain: 0.5,
            initial_cwnd_packets: 10,
            target_cwnd_headroom: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BbrState {
    Startup,
    Drain,
    ProbeBw,
    ProbeRtt,
}

#[derive(Debug, Clone)]
pub struct ScientificBbrController {
    pub max_bandwidth_bps: f64,
    bandwidth_samples: VecDeque<(Instant, f64)>,
    pub min_rtt: Duration,
    min_rtt_stamp: Instant,
    min_rtt_filter_window: Duration,
    pub state: BbrState,
    pacing_rate_bps: f64,
    cwnd: u64,
    bdp: u64,
    config: Arc<SciBbrConfig>,
    mtu: u64,
    startup_rounds_without_growth: u32,
    probe_bw_cycle_idx: usize,
    last_ack: Option<Instant>,
    round_start: Instant,
    ack_events_in_round: u64,
    app_limited_samples: u64,
}

impl ScientificBbrController {
    pub fn new(config: Arc<SciBbrConfig>, now: Instant, current_mtu: u16) -> Self {
        let initial_cwnd = config.initial_cwnd_packets * current_mtu as u64;
        Self {
            max_bandwidth_bps: 0.0,
            bandwidth_samples: VecDeque::new(),
            min_rtt: Duration::from_millis(100),
            min_rtt_stamp: now,
            min_rtt_filter_window: Duration::from_secs(10),
            state: BbrState::Startup,
            pacing_rate_bps: 0.0,
            cwnd: initial_cwnd,
            bdp: initial_cwnd,
            config,
            mtu: current_mtu as u64,
            startup_rounds_without_growth: 0,
            probe_bw_cycle_idx: 0,
            last_ack: None,
            round_start: now,
            ack_events_in_round: 0,
            app_limited_samples: 0,
        }
    }

    pub fn on_acked(&mut self, bytes: u64, rtt: Duration, now: Instant, app_limited: bool) {
        let elapsed = self
            .last_ack
            .map(|last| now.saturating_duration_since(last))
            .unwrap_or_else(|| Duration::from_millis(1))
            .as_secs_f64()
            .max(0.000_001);
        self.last_ack = Some(now);
        self.ack_events_in_round = self.ack_events_in_round.saturating_add(1);
        let delivery_rate_bps = (bytes as f64 * 8.0) / elapsed;
        self.bandwidth_samples.push_back((now, delivery_rate_bps));
        while self.bandwidth_samples.len() > 10 {
            self.bandwidth_samples.pop_front();
        }
        let previous_max = self.max_bandwidth_bps;
        if !app_limited {
            self.max_bandwidth_bps = self
                .bandwidth_samples
                .iter()
                .fold(0.0, |acc, (_, bw)| acc.max(*bw));
        } else {
            self.app_limited_samples = self.app_limited_samples.saturating_add(1);
        }

        if now.duration_since(self.min_rtt_stamp) > self.min_rtt_filter_window {
            self.min_rtt = rtt;
            self.min_rtt_stamp = now;
            self.state = BbrState::ProbeRtt;
        } else if rtt < self.min_rtt {
            self.min_rtt = rtt;
            self.min_rtt_stamp = now;
        }

        self.bdp = ((self.max_bandwidth_bps / 8.0) * self.min_rtt.as_secs_f64()) as u64;
        self.bdp = self.bdp.max(self.mtu * 4);

        match self.state {
            BbrState::Startup => {
                self.cwnd = ((self.config.startup_gain * self.bdp as f64)
                    * self.config.target_cwnd_headroom) as u64;
                if self.max_bandwidth_bps <= previous_max * 1.05 {
                    self.startup_rounds_without_growth += 1;
                } else {
                    self.startup_rounds_without_growth = 0;
                }
                if self.startup_rounds_without_growth >= 3 {
                    self.state = BbrState::Drain;
                }
            }
            BbrState::Drain => {
                self.cwnd = (self.config.drain_gain * self.bdp as f64) as u64;
                self.state = BbrState::ProbeBw;
            }
            BbrState::ProbeBw => {
                const GAINS: [f64; 8] = [1.25, 0.75, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
                if now.duration_since(self.round_start) >= self.min_rtt.max(Duration::from_millis(50)) {
                    self.round_start = now;
                    self.probe_bw_cycle_idx = (self.probe_bw_cycle_idx + 1) % GAINS.len();
                    self.ack_events_in_round = 0;
                }
                let gain = GAINS[self.probe_bw_cycle_idx];
                let app_limited_scale = if app_limited { 0.9 } else { 1.0 };
                self.cwnd =
                    ((self.config.cwnd_gain * self.bdp as f64) * gain * app_limited_scale) as u64;
            }
            BbrState::ProbeRtt => {
                self.cwnd = ((self.config.probe_rtt_cwnd_gain * self.bdp as f64) as u64)
                    .max(self.mtu * 4);
                if now.duration_since(self.min_rtt_stamp) >= Duration::from_millis(200) {
                    self.state = BbrState::ProbeBw;
                }
            }
        }
        self.pacing_rate_bps = self.max_bandwidth_bps.max((self.cwnd as f64 * 8.0) / elapsed);
    }
}

impl Controller for ScientificBbrController {
    fn on_sent(&mut self, _now: Instant, _bytes: u64, _last_packet_number: u64) {}

    fn on_ack(
        &mut self,
        now: Instant,
        _sent: Instant,
        bytes: u64,
        app_limited: bool,
        rtt: &RttEstimator,
    ) {
        self.on_acked(bytes, rtt.get(), now, app_limited);
    }

    fn on_congestion_event(
        &mut self,
        _now: Instant,
        _sent: Instant,
        _is_persistent_congestion: bool,
        _lost_bytes: u64,
    ) {
        // BBR-style behavior: do not multiplicatively reduce cwnd on loss.
    }

    fn on_mtu_update(&mut self, new_mtu: u16) {
        self.mtu = new_mtu as u64;
        self.cwnd = self.cwnd.max(self.mtu * 4);
    }

    fn window(&self) -> u64 {
        self.cwnd.max(self.mtu * 2)
    }

    fn metrics(&self) -> ControllerMetrics {
        let mut m = ControllerMetrics::default();
        m.congestion_window = self.window();
        m.pacing_rate = Some(self.pacing_rate_bps as u64);
        m
    }

    fn clone_box(&self) -> Box<dyn Controller> {
        Box::new(self.clone())
    }

    fn initial_window(&self) -> u64 {
        self.config.initial_cwnd_packets * self.mtu
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

impl ControllerFactory for SciBbrConfig {
    fn build(self: Arc<Self>, now: Instant, current_mtu: u16) -> Box<dyn Controller> {
        Box::new(ScientificBbrController::new(self, now, current_mtu))
    }
}

#[derive(Debug, Clone)]
pub enum LinkType {
    Datacenter,
    Continental,
    Intercontinental,
}

#[derive(Debug, Clone)]
pub struct NetworkProfile {
    pub min_rtt: Duration,
    pub avg_rtt: Duration,
    pub jitter: Duration,
    pub estimated_bandwidth_mbps: f64,
    pub loss_rate: f64,
    pub suggested_initial_cwnd: u64,
    pub link_type: LinkType,
}

pub struct CongestionOracle;

impl CongestionOracle {
    pub async fn probe(endpoint: &crate::transport::SctEndpoint, target: SocketAddr) -> NetworkProfile {
        let mut samples = Vec::new();
        let mut bw_samples = Vec::new();
        let mut failures = 0_u32;
        for _ in 0..5_u32 {
            let server_name = if target.ip().is_loopback() {
                "localhost".to_string()
            } else {
                target.ip().to_string()
            };
            match endpoint.connect(target, &server_name).await {
                Ok(conn) => {
                    let rtt = conn.rtt();
                    samples.push(rtt);
                    let rtt_s = rtt.as_secs_f64().max(0.000_001);
                    let est_mbps = (conn.congestion_window() as f64 * 8.0 / 1_000_000.0) / rtt_s;
                    bw_samples.push(est_mbps);
                }
                Err(_) => failures += 1,
            }
        }
        let min_rtt = samples
            .iter()
            .copied()
            .min()
            .unwrap_or(Duration::from_millis(200));
        let avg_rtt = if samples.is_empty() {
            min_rtt
        } else {
            Duration::from_secs_f64(
                samples.iter().map(|d| d.as_secs_f64()).sum::<f64>() / samples.len() as f64,
            )
        };
        let jitter = if samples.len() < 2 {
            Duration::from_millis(0)
        } else {
            let mut diffs = Vec::new();
            for w in samples.windows(2) {
                let a = w[0].as_secs_f64();
                let b = w[1].as_secs_f64();
                diffs.push((a - b).abs());
            }
            Duration::from_secs_f64(diffs.iter().sum::<f64>() / diffs.len() as f64)
        };
        let estimated_bandwidth_mbps = if bw_samples.is_empty() {
            (1500.0 * 8.0 / 1_000_000.0) / avg_rtt.as_secs_f64().max(0.001) * 1024.0
        } else {
            bw_samples.iter().sum::<f64>() / bw_samples.len() as f64
        };
        let loss_rate = failures as f64 / 5.0;
        let suggested_initial_cwnd = 10 * 1200;
        let link_type = if avg_rtt < Duration::from_millis(10) {
            LinkType::Datacenter
        } else if avg_rtt < Duration::from_millis(80) {
            LinkType::Continental
        } else {
            LinkType::Intercontinental
        };
        NetworkProfile {
            min_rtt,
            avg_rtt,
            jitter,
            estimated_bandwidth_mbps,
            loss_rate,
            suggested_initial_cwnd,
            link_type,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_transitions_to_drain_and_probebw() {
        let cfg = Arc::new(SciBbrConfig::default());
        let mut c = ScientificBbrController::new(cfg, Instant::now(), 1200);
        assert_eq!(c.state, BbrState::Startup);
        let base = c.window();
        let now = Instant::now();
        for i in 0..8 {
            c.on_acked(
                1200 * 10,
                Duration::from_millis(80),
                now + Duration::from_millis(i),
                false,
            );
        }
        assert!(matches!(c.state, BbrState::Drain | BbrState::ProbeBw));
        assert!(c.window() >= base);
    }

    #[test]
    fn loss_does_not_reduce_cwnd() {
        let cfg = Arc::new(SciBbrConfig::default());
        let mut c = ScientificBbrController::new(cfg, Instant::now(), 1200);
        c.on_acked(
            1200 * 20,
            Duration::from_millis(50),
            Instant::now() + Duration::from_millis(1),
            false,
        );
        let before = c.window();
        c.on_congestion_event(Instant::now(), Instant::now(), false, 64 * 1024);
        assert_eq!(before, c.window());
    }

    #[test]
    fn probe_benchmark_reaches_utilization_target_shape() {
        let cfg = Arc::new(SciBbrConfig::default());
        let mut c = ScientificBbrController::new(cfg, Instant::now(), 1200);
        let target_bps = 10_000_000_000_f64;
        let rtt = Duration::from_millis(100);
        let mut now = Instant::now();
        for _ in 0..40 {
            let bytes = ((target_bps / 8.0) * 0.1) as u64;
            now += Duration::from_millis(100);
            c.on_acked(bytes, rtt, now, false);
        }
        let utilization = c.max_bandwidth_bps / target_bps;
        assert!(utilization > 0.8, "utilization={utilization}");
        assert!(c.metrics().pacing_rate.unwrap_or_default() > 0);
    }

    #[test]
    fn app_limited_samples_do_not_stall_controller() {
        let cfg = Arc::new(SciBbrConfig::default());
        let mut c = ScientificBbrController::new(cfg, Instant::now(), 1200);
        let now = Instant::now();
        c.on_acked(1200 * 10, Duration::from_millis(80), now, true);
        c.on_acked(
            1200 * 10,
            Duration::from_millis(80),
            now + Duration::from_millis(100),
            false,
        );
        assert!(c.window() >= 1200 * 4);
        assert!(c.app_limited_samples >= 1);
    }
}
