//! Predictive congestion forecasting and control-loop stabilization.
//!
//! Treats the transport stack as a sampled control system: forecast build-up,
//! damp aggressive reactions, hysteresis on mode switches, and cap speculative
//! work using a dynamic stability budget.
//!
//! Operator-oriented overview (bench flows, metrics, tuning): see
//! `docs/NETWORK-EMULATION.md` → **Predictive congestion stabilization**.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::{CongestionSignal, HybridCongestionController, MultiPathScheduler};

/// One control-system sample (ordered oldest → newest in the ring buffer).
#[derive(Debug, Clone, Copy)]
pub struct NetworkSample {
    pub rtt_secs: f64,
    pub loss: f64,
    pub queue_pressure: f64,
    pub pacing_backlog: f64,
    pub retrans_accel: f64,
}

/// Short-horizon congestion build-up forecast (200–1000 ms effective window).
#[derive(Debug, Clone, Copy, Default)]
pub struct CongestionForecast {
    pub predicted_queue_growth: f64,
    pub predicted_loss_growth: f64,
    pub predicted_rtt_growth: f64,
    pub confidence: f64,
}

/// EWMA state for pacing / duplication / utility smoothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct ControlState {
    pub utility_momentum: f64,
    pub pacing_momentum: f64,
    pub duplication_momentum: f64,
}

/// Asymmetric thresholds to avoid mode flapping.
#[derive(Debug, Clone, Copy)]
pub struct HysteresisThreshold {
    pub enter_threshold: f64,
    pub exit_threshold: f64,
}

impl Default for HysteresisThreshold {
    fn default() -> Self {
        Self {
            enter_threshold: 0.8,
            exit_threshold: 0.5,
        }
    }
}

impl HysteresisThreshold {
    pub fn step_aggressive(&self, inside: bool, score: f64) -> bool {
        if inside {
            score >= self.exit_threshold
        } else {
            score > self.enter_threshold
        }
    }
}

/// Caps aggressive behaviors when the plant looks unstable.
#[derive(Debug, Clone, Copy, Default)]
pub struct StabilityBudget {
    pub max_duplication_rate: f64,
    pub max_fec_overhead: f64,
    pub max_inflight_growth: f64,
}

impl StabilityBudget {
    pub fn from_forecast_volatility(
        forecast: &CongestionForecast,
        queue_volatility: f64,
        rtt_accel: f64,
    ) -> Self {
        let stress = (forecast.predicted_queue_growth * 1.1
            + forecast.predicted_loss_growth * 0.9
            + forecast.predicted_rtt_growth * 0.85
            + queue_volatility * 0.6
            + rtt_accel.max(0.0) * 0.5)
            .clamp(0.0, 2.5);
        let relax = (1.0 - stress * 0.38).clamp(0.35, 1.0);
        Self {
            max_duplication_rate: (0.55 + 0.45 * relax).clamp(0.2, 1.0),
            max_fec_overhead: (0.5 + 0.5 * relax).clamp(0.25, 1.0),
            max_inflight_growth: (0.45 + 0.55 * relax).clamp(0.3, 1.0),
        }
    }
}

/// Rolling variance proxies for oscillation-aware damping.
#[derive(Debug, Clone, Copy, Default)]
pub struct OscillationDetector {
    pub utility_variance: f64,
    pub pacing_variance: f64,
    pub queue_variance: f64,
    last_utility: Option<f64>,
    last_pacing: Option<f64>,
    last_queue: Option<f64>,
}

impl OscillationDetector {
    pub fn observe(&mut self, utility: f64, pacing_norm: f64, queue_p: f64) {
        const BETA: f64 = 0.22;
        if let Some(prev) = self.last_utility {
            let d = utility - prev;
            self.utility_variance = (1.0 - BETA) * self.utility_variance + BETA * d * d;
        }
        self.last_utility = Some(utility);
        if let Some(prev) = self.last_pacing {
            let d = pacing_norm - prev;
            self.pacing_variance = (1.0 - BETA) * self.pacing_variance + BETA * d * d;
        }
        self.last_pacing = Some(pacing_norm);
        if let Some(prev) = self.last_queue {
            let d = queue_p - prev;
            self.queue_variance = (1.0 - BETA) * self.queue_variance + BETA * d * d;
        }
        self.last_queue = Some(queue_p);
    }

    pub fn oscillation_score(&self) -> f64 {
        (self.utility_variance * 6.0 + self.pacing_variance * 4.0 + self.queue_variance * 3.0)
            .sqrt()
            .clamp(0.0, 1.5)
    }
}

/// Long-horizon stability telemetry (updated incrementally).
#[derive(Debug, Clone, Default)]
pub struct StabilityTelemetry {
    pub oscillation_zero_crossings: u64,
    pub queue_overshoot_events: u64,
    pub utility_stability_ewma: f64,
    pub forecast_confidence_ewma: f64,
    pub recovery_ticks_after_stress: u64,
    last_utility_sign: i8,
    stress_level: f64,
    recovery_counter: u64,
}

impl StabilityTelemetry {
    pub fn tick(
        &mut self,
        utility: f64,
        forecast: &CongestionForecast,
        queue_pressure: f64,
        overshoot_threshold: f64,
    ) {
        let sign = if utility > self.utility_stability_ewma {
            1
        } else if utility < self.utility_stability_ewma {
            -1
        } else {
            0
        };
        if sign != 0 && self.last_utility_sign != 0 && sign != self.last_utility_sign {
            self.oscillation_zero_crossings = self.oscillation_zero_crossings.saturating_add(1);
        }
        if sign != 0 {
            self.last_utility_sign = sign;
        }
        self.utility_stability_ewma = 0.88 * self.utility_stability_ewma + 0.12 * utility;
        self.forecast_confidence_ewma =
            0.9 * self.forecast_confidence_ewma + 0.1 * forecast.confidence;
        if queue_pressure > overshoot_threshold {
            self.queue_overshoot_events = self.queue_overshoot_events.saturating_add(1);
        }
        let stress =
            (forecast.predicted_queue_growth + forecast.predicted_rtt_growth * 0.5).clamp(0.0, 1.0);
        if stress > 0.35 {
            self.stress_level = stress;
            self.recovery_counter = 0;
        } else if self.stress_level > 0.1 {
            self.recovery_counter = self.recovery_counter.saturating_add(1);
            if self.recovery_counter > 8 {
                self.recovery_ticks_after_stress = self
                    .recovery_ticks_after_stress
                    .saturating_add(self.recovery_counter);
                self.stress_level *= 0.85;
                self.recovery_counter = 0;
            }
        }
    }
}

/// Multi-timescale control decomposition (fast / medium / slow).
#[derive(Debug, Clone, Copy)]
pub struct MultiTimescaleClock {
    pub fast_min: Duration,
    pub medium_min: Duration,
    pub slow_min: Duration,
    last_fast: Instant,
    last_medium: Instant,
    last_slow: Instant,
}

impl Default for MultiTimescaleClock {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            fast_min: Duration::from_millis(12),
            medium_min: Duration::from_millis(120),
            slow_min: Duration::from_millis(1500),
            last_fast: now,
            last_medium: now,
            last_slow: now,
        }
    }
}

impl MultiTimescaleClock {
    pub fn tick(&mut self, now: Instant) -> TimescaleTier {
        if now.duration_since(self.last_slow) >= self.slow_min {
            self.last_slow = now;
            return TimescaleTier::Slow;
        }
        if now.duration_since(self.last_medium) >= self.medium_min {
            self.last_medium = now;
            return TimescaleTier::Medium;
        }
        if now.duration_since(self.last_fast) >= self.fast_min {
            self.last_fast = now;
            return TimescaleTier::Fast;
        }
        TimescaleTier::Fast
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimescaleTier {
    Fast,
    Medium,
    Slow,
}

/// Per-packet stabilization outputs for the scheduler / CC.
#[derive(Debug, Clone, Copy)]
pub struct PacketStabilization {
    pub forecast: CongestionForecast,
    pub budget: StabilityBudget,
    pub pacing_scale: f64,
    pub speculative_scale: f64,
    pub dup_utility_scale: f64,
    pub suppress_speculative_dup: bool,
    pub oscillation_damp: f64,
    pub ranking_pressure_scale: f64,
}

impl Default for PacketStabilization {
    fn default() -> Self {
        Self {
            forecast: CongestionForecast::default(),
            budget: StabilityBudget {
                max_duplication_rate: 1.0,
                max_fec_overhead: 1.0,
                max_inflight_growth: 1.0,
            },
            pacing_scale: 1.0,
            speculative_scale: 1.0,
            dup_utility_scale: 1.0,
            suppress_speculative_dup: false,
            oscillation_damp: 1.0,
            ranking_pressure_scale: 1.0,
        }
    }
}

/// Owns history, forecast, oscillation, telemetry, and timescale state.
#[derive(Debug)]
pub struct PredictiveStabilizer {
    samples: VecDeque<NetworkSample>,
    max_samples: usize,
    pub control: ControlState,
    pub oscillation: OscillationDetector,
    pub telemetry: StabilityTelemetry,
    pub clock: MultiTimescaleClock,
    pub aggressive_hysteresis: HysteresisThreshold,
    last_sample_loss: f64,
    last_wall: Instant,
    pub slow_loop_ranking_bias: f64,
    /// Smoothed RTT variance from `HybridCongestionController` (set by sender each tick).
    pub rtt_variance_trend: f64,
}

impl Default for PredictiveStabilizer {
    fn default() -> Self {
        Self {
            samples: VecDeque::new(),
            max_samples: 48,
            control: ControlState {
                utility_momentum: 0.0,
                pacing_momentum: 1.0,
                duplication_momentum: 1.0,
            },
            oscillation: OscillationDetector::default(),
            telemetry: StabilityTelemetry::default(),
            clock: MultiTimescaleClock::default(),
            aggressive_hysteresis: HysteresisThreshold::default(),
            last_sample_loss: 0.0,
            last_wall: Instant::now(),
            slow_loop_ranking_bias: 1.0,
            rtt_variance_trend: 0.0,
        }
    }
}

impl PredictiveStabilizer {
    pub fn push_network_sample(&mut self, s: NetworkSample) {
        if self.samples.len() >= self.max_samples {
            self.samples.pop_front();
        }
        self.samples.push_back(s);
    }

    /// Build a stabilization context for the next send using CC + scheduler + signal.
    pub fn prepare_packet(
        &mut self,
        now: Instant,
        cc: &HybridCongestionController,
        scheduler: &MultiPathScheduler,
        sig: &CongestionSignal,
        primary_raw_utility: f64,
    ) -> PacketStabilization {
        let pacing_norm = scheduler
            .queue_models
            .iter()
            .map(|q| q.pacing_backlog as f64 / 3000.0)
            .sum::<f64>()
            / scheduler.queue_models.len().max(1) as f64;
        let dt = now
            .checked_duration_since(self.last_wall)
            .unwrap_or(Duration::ZERO)
            .as_secs_f64()
            .max(0.001);
        self.last_wall = now;
        let retrans_accel = (cc.loss_rate - self.last_sample_loss) / dt;
        self.last_sample_loss = cc.loss_rate;

        let sample = NetworkSample {
            rtt_secs: cc.last_rtt.as_secs_f64().max(0.000_01),
            loss: cc.loss_rate,
            queue_pressure: sig.queue_pressure.clamp(0.0, 1.0),
            pacing_backlog: pacing_norm.clamp(0.0, 3.0),
            retrans_accel: retrans_accel.clamp(-5.0, 5.0),
        };
        self.push_network_sample(sample);

        let slice: Vec<NetworkSample> = self.samples.iter().copied().collect();
        let forecast = forecast_congestion(&slice);
        self.telemetry
            .tick(primary_raw_utility, &forecast, sig.queue_pressure, 0.72);

        let osc = self.oscillation.oscillation_score();
        let min_rtt_s = cc.min_rtt.as_secs_f64().max(0.000_5);
        let rtt_jitter = (self.rtt_variance_trend / min_rtt_s.powi(2))
            .sqrt()
            .clamp(0.0, 2.0);
        let queue_vol = self.oscillation.queue_variance.sqrt() + rtt_jitter * 0.25;
        let rtt_accel = cc.rtt_gradient.max(0.0);
        let budget = StabilityBudget::from_forecast_volatility(&forecast, queue_vol, rtt_accel);

        let tier = self.clock.tick(now);
        let oscillation_damp = (1.0 / (1.0 + osc * 2.2)).clamp(0.45, 1.0);

        // Fast loop: pacing reacts to forecast + momentum.
        let raw_pacing = (1.0 - 0.55 * forecast.predicted_queue_growth.clamp(0.0, 0.9))
            * (1.0 - 0.35 * forecast.predicted_rtt_growth.clamp(0.0, 0.8));
        const P_ALPHA: f64 = 0.28;
        self.control.pacing_momentum =
            P_ALPHA * raw_pacing + (1.0 - P_ALPHA) * self.control.pacing_momentum;
        let pacing_scale = (self.control.pacing_momentum * oscillation_damp).clamp(0.35, 1.0);

        // Medium loop: speculative / duplication envelope.
        let mut speculative_scale = 1.0;
        if matches!(tier, TimescaleTier::Medium | TimescaleTier::Slow) {
            speculative_scale = (budget.max_duplication_rate * oscillation_damp).clamp(0.25, 1.0);
            const D_ALPHA: f64 = 0.25;
            self.control.duplication_momentum =
                D_ALPHA * speculative_scale + (1.0 - D_ALPHA) * self.control.duplication_momentum;
        }
        let speculative_scale =
            (speculative_scale * self.control.duplication_momentum).clamp(0.2, 1.0);

        // Slow loop: global ranking / tail pressure coupling.
        if tier == TimescaleTier::Slow {
            let target = (1.0 - 0.2 * forecast.confidence * forecast.predicted_queue_growth)
                .clamp(0.55, 1.0);
            self.slow_loop_ranking_bias = 0.18 * target + 0.82 * self.slow_loop_ranking_bias;
        }

        let q_growth_th = 0.28 + 0.12 * (1.0 - forecast.confidence);
        let suppress = forecast.predicted_queue_growth > q_growth_th && forecast.confidence > 0.35;
        let dup_scale = if suppress {
            (1.0 - (forecast.predicted_queue_growth - q_growth_th).clamp(0.0, 0.5) * 1.4) * 0.55
        } else {
            1.0
        } * budget.max_duplication_rate;

        let out = PacketStabilization {
            forecast,
            budget,
            pacing_scale,
            speculative_scale,
            dup_utility_scale: dup_scale.clamp(0.08, 1.0),
            suppress_speculative_dup: suppress,
            oscillation_damp,
            ranking_pressure_scale: self.slow_loop_ranking_bias.clamp(0.5, 1.0),
        };
        self.oscillation
            .observe(primary_raw_utility, pacing_norm, sig.queue_pressure);
        out
    }
}

/// Forecast congestion build-up from recent samples (200–1000 ms horizon blend).
#[allow(clippy::redundant_closure)] // fn pointers vs field accessors: keep explicit for clarity.
pub fn forecast_congestion(recent_metrics: &[NetworkSample]) -> CongestionForecast {
    if recent_metrics.len() < 3 {
        return CongestionForecast {
            predicted_queue_growth: 0.0,
            predicted_loss_growth: 0.0,
            predicted_rtt_growth: 0.0,
            confidence: (recent_metrics.len() as f64 / 6.0).clamp(0.15, 0.45),
        };
    }
    let n = recent_metrics.len();
    let span = (n - 1) as f64;
    let slope = |f: fn(&NetworkSample) -> f64| -> f64 {
        let first = f(&recent_metrics[0]);
        let last = f(&recent_metrics[n - 1]);
        (last - first) / span.max(1.0)
    };
    let mean_var = |f: fn(&NetworkSample) -> f64| -> f64 {
        let sum: f64 = recent_metrics.iter().map(|s| f(s)).sum();
        let m = sum / n as f64;
        recent_metrics
            .iter()
            .map(|s| {
                let d = f(s) - m;
                d * d
            })
            .sum::<f64>()
            / n as f64
    };

    let rtt_slope = slope(|s| s.rtt_secs);
    let rtt_var = mean_var(|s| s.rtt_secs).sqrt();
    let queue_vel = slope(|s| s.queue_pressure);
    let pacing_vel = slope(|s| s.pacing_backlog);
    let loss_vel = slope(|s| s.loss);
    let retrans = recent_metrics[n - 1].retrans_accel.max(0.0);

    let h_short = 0.20_f64;
    let h_long = 1.0_f64;
    let w = 0.55;
    let horizon = h_short * w + h_long * (1.0 - w);

    let mut predicted_queue_growth = (queue_vel * horizon + pacing_vel * 0.4 * horizon).max(0.0);
    let mut predicted_loss_growth = (loss_vel * horizon + retrans * 0.15 * horizon).max(0.0);
    let mut predicted_rtt_growth = (rtt_slope * horizon + rtt_var * 0.35 * horizon).max(0.0);

    // Variance-driven early warning (queue wobble before mean moves).
    let q_var = mean_var(|s| s.queue_pressure).sqrt();
    predicted_queue_growth += q_var * 0.25 * horizon;
    predicted_rtt_growth += rtt_var * 0.2 * horizon;

    predicted_queue_growth = predicted_queue_growth.clamp(0.0, 1.2);
    predicted_loss_growth = predicted_loss_growth.clamp(0.0, 0.8);
    predicted_rtt_growth = predicted_rtt_growth.clamp(0.0, 1.0);

    let confidence = ((n as f64 - 2.0) / 14.0).clamp(0.2, 1.0);

    CongestionForecast {
        predicted_queue_growth,
        predicted_loss_growth,
        predicted_rtt_growth,
        confidence,
    }
}

/// Smoothed utility for anti-thrash scheduling.
#[inline]
pub fn smoothed_utility(alpha: f64, current: f64, historical: f64) -> f64 {
    let a = alpha.clamp(0.05, 0.95);
    a * current + (1.0 - a) * historical
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramping_queue() -> Vec<NetworkSample> {
        (0..12)
            .map(|i| NetworkSample {
                rtt_secs: 0.02 + i as f64 * 0.0009,
                loss: i as f64 * 0.0004,
                queue_pressure: i as f64 * 0.07,
                pacing_backlog: i as f64 * 0.05,
                retrans_accel: 0.01,
            })
            .collect()
    }

    #[test]
    fn forecast_rising_queue_positive_growth() {
        let v = ramping_queue();
        let f = forecast_congestion(&v);
        assert!(f.predicted_queue_growth > 0.01, "{f:?}");
        assert!(f.confidence > 0.5);
    }

    #[test]
    fn hysteresis_latches_until_exit() {
        let h = HysteresisThreshold {
            enter_threshold: 0.8,
            exit_threshold: 0.5,
        };
        let mut inside = false;
        inside = h.step_aggressive(inside, 0.75);
        assert!(!inside);
        inside = h.step_aggressive(inside, 0.85);
        assert!(inside);
        inside = h.step_aggressive(inside, 0.55);
        assert!(inside);
        inside = h.step_aggressive(inside, 0.45);
        assert!(!inside);
    }

    #[test]
    fn smoothed_utility_blends() {
        let s = smoothed_utility(0.3, 1.0, 0.0);
        assert!((s - 0.3).abs() < 1e-9);
    }
}
