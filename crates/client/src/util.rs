//! # Utilities
//!
//! Shared helper functions for the Willow client.

use std::time::Duration;

/// Default timeout for actor calls (native only; WASM awaits without timeout).
pub const ACTOR_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Run `f` with a timeout, returning `Err(ClientError::ActorTimeout(label))`
/// if it does not complete within [`ACTOR_CALL_TIMEOUT`].
///
/// On WASM there are no tokio timers, so the future is simply awaited with
/// no timeout applied.
pub async fn with_timeout<T, F>(label: &'static str, f: F) -> Result<T, crate::ClientError>
where
    F: std::future::Future<Output = T>,
{
    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio::time::timeout(ACTOR_CALL_TIMEOUT, f)
            .await
            .map_err(|_| crate::ClientError::ActorTimeout(label))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = label; // WASM has no tokio timers — await without timeout for now.
        Ok(f.await)
    }
}

/// Truncate a peer ID for display.
pub fn truncate_peer_id(s: &str) -> String {
    if s.len() > 12 {
        format!("{}...", &s[..12])
    } else {
        s.to_string()
    }
}

/// Format a millisecond timestamp as "HH:MM".
pub fn format_timestamp(ms: u64) -> String {
    if ms == 0 {
        return String::new();
    }
    let secs = ms / 1000;
    let hours = (secs / 3600) % 24;
    let minutes = (secs / 60) % 60;
    format!("{hours:02}:{minutes:02}")
}

/// Build a gossipsub topic string from a server ID and channel name.
pub fn make_topic(server_id: &str, channel_name: &str) -> String {
    format!("{}/{}", server_id, channel_name)
}

/// Get the current wall-clock time in milliseconds since the Unix epoch.
///
/// Uses `std::time::SystemTime` on native and `js_sys::Date::now()` on WASM.
#[cfg(not(target_arch = "wasm32"))]
pub fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Get the current wall-clock time in milliseconds since the Unix epoch.
///
/// Uses `js_sys::Date::now()` on WASM.
#[cfg(target_arch = "wasm32")]
pub fn current_time_ms() -> u64 {
    js_sys::Date::now() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_id() {
        assert_eq!(truncate_peer_id("short"), "short");
    }

    #[test]
    fn truncate_long_id() {
        let long = "12D3KooWAbCdEfGhIjKlMnOpQrStUvWxYz";
        let result = truncate_peer_id(long);
        assert!(result.ends_with("..."));
        assert_eq!(result.len(), 15); // 12 chars + "..."
    }

    #[test]
    fn format_timestamp_zero() {
        assert_eq!(format_timestamp(0), "");
    }

    #[test]
    fn format_timestamp_nonzero() {
        // 1 hour 30 minutes = 5400 seconds = 5400000 ms
        assert_eq!(format_timestamp(5_400_000), "01:30");
    }

    #[test]
    fn format_timestamp_wraps_24h() {
        // 25 hours = 90000 seconds = 90000000 ms -> wraps to 01:00
        assert_eq!(format_timestamp(90_000_000), "01:00");
    }
}
