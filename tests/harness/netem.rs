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
