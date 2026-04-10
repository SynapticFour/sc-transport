#[cfg(target_os = "linux")]
pub fn supported() -> bool {
    true
}

#[cfg(not(target_os = "linux"))]
pub fn supported() -> bool {
    false
}

#[cfg(target_os = "linux")]
pub fn add_loss(interface: &str, percent: u32) -> String {
    format!("tc qdisc add dev {interface} root netem loss {percent}%")
}

#[cfg(target_os = "linux")]
pub fn clear(interface: &str) -> String {
    format!("tc qdisc del dev {interface} root")
}

#[derive(Debug, Clone)]
pub struct NetemConfig {
    pub interface: String,
    pub delay_ms: u32,
    pub delay_jitter_ms: u32,
    pub loss_pct: f64,
    pub correlation_pct: f64,
}

impl NetemConfig {
    pub fn wan_continental() -> Self {
        Self {
            interface: "lo".to_string(),
            delay_ms: 50,
            delay_jitter_ms: 5,
            loss_pct: 0.1,
            correlation_pct: 25.0,
        }
    }
    pub fn wan_intercontinental() -> Self {
        Self {
            interface: "lo".to_string(),
            delay_ms: 100,
            delay_jitter_ms: 10,
            loss_pct: 0.5,
            correlation_pct: 25.0,
        }
    }
    pub fn wan_degraded() -> Self {
        Self {
            interface: "lo".to_string(),
            delay_ms: 150,
            delay_jitter_ms: 20,
            loss_pct: 2.0,
            correlation_pct: 50.0,
        }
    }
}

pub struct NetemGuard {
    interface: String,
    original_config: Option<String>,
}

impl NetemGuard {
    pub fn try_apply(config: &NetemConfig) -> Option<Self> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = config;
            None
        }
        #[cfg(target_os = "linux")]
        {
            use std::process::Command;
            let has_tc = Command::new("sh")
                .arg("-c")
                .arg("command -v tc >/dev/null 2>&1")
                .status()
                .ok()
                .map(|s| s.success())
                .unwrap_or(false);
            if !has_tc {
                return None;
            }
            let original = Command::new("sh")
                .arg("-c")
                .arg(format!("tc qdisc show dev {}", config.interface))
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string());
            let _ = Command::new("sh")
                .arg("-c")
                .arg(format!("tc qdisc del dev {} root >/dev/null 2>&1 || true", config.interface))
                .status();
            let apply = format!(
                "tc qdisc add dev {} root netem delay {}ms {}ms loss {}% {}%",
                config.interface,
                config.delay_ms,
                config.delay_jitter_ms,
                config.loss_pct,
                config.correlation_pct
            );
            let ok = Command::new("sh")
                .arg("-c")
                .arg(apply)
                .status()
                .ok()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                return None;
            }
            Some(Self {
                interface: config.interface.clone(),
                original_config: original,
            })
        }
    }

    pub fn restore(&self) {
        #[cfg(target_os = "linux")]
        {
            let _ = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "tc qdisc del dev {} root >/dev/null 2>&1 || true",
                    self.interface
                ))
                .status();
            let _ = &self.original_config;
        }
    }
}

impl Drop for NetemGuard {
    fn drop(&mut self) {
        self.restore();
    }
}
