use quinn::congestion::{Controller, ControllerFactory, ControllerMetrics};
use quinn_proto::RttEstimator;
use std::any::Any;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct SciBbrConfig {
    pub startup_cwnd_gain: f64,
    pub drain_cwnd_gain: f64,
    pub probe_bw_cwnd_gain: f64,
    pub probe_rtt_duration: Duration,
    pub min_rtt_filter_window: Duration,
    pub initial_cwnd_bytes: u64,
    pub max_segment_size: u64,
}

impl Default for SciBbrConfig {
    fn default() -> Self {
        Self {
            startup_cwnd_gain: 2.885,
            drain_cwnd_gain: 0.35,
            probe_bw_cwnd_gain: 2.0,
            probe_rtt_duration: Duration::from_millis(200),
            min_rtt_filter_window: Duration::from_secs(10),
            initial_cwnd_bytes: 10 * 1452,
            max_segment_size: 1452,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum BbrState {
    Startup,
    Drain,
    ProbeBw { pacing_gain: f64 },
    ProbeRtt { entered_at: Instant },
}

#[derive(Debug, Clone)]
pub struct ScientificCongestionController {
    bw_samples: VecDeque<(Instant, f64)>,
    bw_window_duration: Duration,
    max_bandwidth_bps: f64,
    min_rtt: Duration,
    min_rtt_stamp: Instant,
    state: BbrState,
    cycles_in_probe_bw: u8,
    cwnd: u64,
    pacing_rate_bps: f64,
    startup_no_growth_rounds: u8,
    config: Arc<SciBbrConfig>,
}

impl ScientificCongestionController {
    pub fn new(config: Arc<SciBbrConfig>, now: Instant, current_mtu: u16) -> Self {
        let mut c = Self {
            bw_samples: VecDeque::new(),
            bw_window_duration: Duration::from_secs(3),
            max_bandwidth_bps: 0.0,
            min_rtt: Duration::from_millis(100),
            min_rtt_stamp: now,
            state: BbrState::Startup,
            cycles_in_probe_bw: 0,
            cwnd: config.initial_cwnd_bytes.max(current_mtu as u64 * 2),
            pacing_rate_bps: 0.0,
            startup_no_growth_rounds: 0,
            config,
        };
        c.pacing_rate_bps = (c.cwnd as f64 / c.min_rtt.as_secs_f64().max(0.001)) * 8.0;
        c
    }

    fn update_bandwidth_model(&mut self, bytes_acked: u64, rtt: Duration, now: Instant) {
        let delivery_rate = (bytes_acked as f64 / rtt.as_secs_f64().max(0.000_001)) * 8.0;
        self.bw_samples.push_back((now, delivery_rate));
        while let Some((ts, _)) = self.bw_samples.front() {
            if now.duration_since(*ts) > self.bw_window_duration {
                let _ = self.bw_samples.pop_front();
            } else {
                break;
            }
        }
        self.max_bandwidth_bps = self
            .bw_samples
            .iter()
            .map(|(_, b)| *b)
            .fold(0.0, f64::max);
    }

    fn update_rtt_model(&mut self, rtt: Duration, now: Instant) {
        if now.duration_since(self.min_rtt_stamp) > self.config.min_rtt_filter_window {
            self.min_rtt = rtt;
            self.min_rtt_stamp = now;
            return;
        }
        if rtt < self.min_rtt {
            self.min_rtt = rtt;
            self.min_rtt_stamp = now;
        }
    }

    fn bdp(&self) -> u64 {
        (((self.max_bandwidth_bps / 8.0) * self.min_rtt.as_secs_f64()) as u64)
            .max(self.config.initial_cwnd_bytes)
    }

    fn transition_state(&mut self, now: Instant, previous_bw: f64) {
        match &self.state {
            BbrState::Startup => {
                if self.max_bandwidth_bps <= previous_bw * 1.25 {
                    self.startup_no_growth_rounds = self.startup_no_growth_rounds.saturating_add(1);
                } else {
                    self.startup_no_growth_rounds = 0;
                }
                if self.startup_no_growth_rounds >= 3 {
                    self.state = BbrState::Drain;
                }
            }
            BbrState::Drain => {
                if self.cwnd <= self.bdp() {
                    self.state = BbrState::ProbeBw { pacing_gain: 1.0 };
                }
            }
            BbrState::ProbeBw { .. } => {
                if now.duration_since(self.min_rtt_stamp) >= self.config.min_rtt_filter_window {
                    self.state = BbrState::ProbeRtt { entered_at: now };
                }
            }
            BbrState::ProbeRtt { entered_at } => {
                if now.duration_since(*entered_at) >= self.config.probe_rtt_duration {
                    self.state = BbrState::ProbeBw { pacing_gain: 1.0 };
                }
            }
        }
    }
}

impl Controller for ScientificCongestionController {
    fn on_sent(&mut self, _now: Instant, _bytes: u64, _last_packet_number: u64) {}

    fn on_ack(
        &mut self,
        now: Instant,
        _sent: Instant,
        bytes: u64,
        _app_limited: bool,
        rtt: &RttEstimator,
    ) {
        let previous_bw = self.max_bandwidth_bps;
        self.update_bandwidth_model(bytes, rtt.get(), now);
        self.update_rtt_model(rtt.get(), now);
        self.transition_state(now, previous_bw);
        let bdp = self.bdp();
        match &self.state {
            BbrState::Startup => {
                self.cwnd = (bdp as f64 * self.config.startup_cwnd_gain) as u64;
            }
            BbrState::Drain => {
                self.cwnd = (bdp as f64 * self.config.drain_cwnd_gain) as u64;
            }
            BbrState::ProbeBw { .. } => {
                const GAINS: [f64; 8] = [1.25, 0.75, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
                let gain = GAINS[(self.cycles_in_probe_bw as usize) % GAINS.len()];
                self.state = BbrState::ProbeBw { pacing_gain: gain };
                self.cycles_in_probe_bw = self.cycles_in_probe_bw.wrapping_add(1);
                self.cwnd = (bdp as f64 * self.config.probe_bw_cwnd_gain * gain) as u64;
            }
            BbrState::ProbeRtt { .. } => {
                self.cwnd = bdp.min(self.config.initial_cwnd_bytes);
            }
        }
        self.cwnd = self.cwnd.max(2 * self.config.max_segment_size);
        self.pacing_rate_bps =
            (self.cwnd as f64 / self.min_rtt.as_secs_f64().max(0.000_001)) * 8.0;
    }

    fn on_congestion_event(
        &mut self,
        _now: Instant,
        _sent: Instant,
        _is_persistent_congestion: bool,
        _lost_bytes: u64,
    ) {
        // BBR-style: no Reno/CUBIC cwnd-halving.
        self.min_rtt_stamp = Instant::now() - self.config.min_rtt_filter_window;
    }

    fn on_mtu_update(&mut self, new_mtu: u16) {
        self.cwnd = self.cwnd.max(2 * new_mtu as u64);
    }

    fn window(&self) -> u64 {
        self.cwnd.max(2 * self.config.max_segment_size)
    }

    fn clone_box(&self) -> Box<dyn Controller> {
        Box::new(self.clone())
    }

    fn initial_window(&self) -> u64 {
        self.config.initial_cwnd_bytes
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }

    fn metrics(&self) -> ControllerMetrics {
        let mut m = ControllerMetrics::default();
        m.congestion_window = self.window();
        m.pacing_rate = Some(self.pacing_rate_bps as u64);
        m
    }
}

impl ControllerFactory for SciBbrConfig {
    fn build(self: Arc<Self>, now: Instant, current_mtu: u16) -> Box<dyn Controller> {
        Box::new(ScientificCongestionController::new(self, now, current_mtu))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bdp_calculation_approx_works() {
        let cfg = Arc::new(SciBbrConfig::default());
        let mut c = ScientificCongestionController::new(cfg, Instant::now(), 1452);
        c.max_bandwidth_bps = 1_000_000_000.0;
        c.min_rtt = Duration::from_millis(100);
        let bdp = c.bdp();
        assert!(bdp > 10_000_000 && bdp < 15_000_000);
    }

    #[test]
    fn loss_event_does_not_halve_cwnd() {
        let cfg = Arc::new(SciBbrConfig::default());
        let mut c = ScientificCongestionController::new(cfg, Instant::now(), 1452);
        c.cwnd = 2_000_000;
        let before = c.cwnd;
        c.on_congestion_event(Instant::now(), Instant::now(), false, 1000);
        assert!(c.cwnd >= (before as f64 * 0.9) as u64);
    }

    #[test]
    fn startup_to_drain_to_probebw_transition() {
        let cfg = Arc::new(SciBbrConfig::default());
        let mut c = ScientificCongestionController::new(cfg, Instant::now(), 1452);
        let now = Instant::now();
        // Force startup completion condition.
        c.max_bandwidth_bps = 1000.0;
        c.startup_no_growth_rounds = 3;
        c.transition_state(now, 1000.0);
        assert!(matches!(c.state, BbrState::Drain));
        c.cwnd = c.bdp();
        c.transition_state(now + Duration::from_millis(10), c.max_bandwidth_bps);
        assert!(matches!(c.state, BbrState::ProbeBw { .. }));
    }

    #[test]
    fn pacing_rate_is_non_zero_after_ack_modeling() {
        let cfg = Arc::new(SciBbrConfig::default());
        let mut c = ScientificCongestionController::new(cfg, Instant::now(), 1452);
        let now = Instant::now();
        c.update_rtt_model(Duration::from_millis(100), now);
        c.update_bandwidth_model(1452 * 100, Duration::from_millis(100), now);
        c.transition_state(now, 0.0);
        c.cwnd = c.bdp();
        c.pacing_rate_bps = (c.cwnd as f64 / c.min_rtt.as_secs_f64().max(0.000_001)) * 8.0;
        assert!(c.metrics().pacing_rate.unwrap_or_default() > 0);
    }
}
