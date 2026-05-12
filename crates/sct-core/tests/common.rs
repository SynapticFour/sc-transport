//! Shared helpers for sct-core integration tests (separate test binaries).
use std::time::Duration;

/// Fails fast with a clear message instead of hanging indefinitely (deadlock / port contention).
pub async fn with_timeout<T>(
    label: &'static str,
    secs: u64,
    fut: impl std::future::Future<Output = T>,
) -> T {
    tokio::time::timeout(Duration::from_secs(secs), fut)
        .await
        .unwrap_or_else(|_| {
            panic!(
                "{label}: exceeded {secs}s (possible hang). \
                 Try serializing test binaries: \
                 cargo test -p sct-core --tests -- --test-threads=1"
            )
        })
}
