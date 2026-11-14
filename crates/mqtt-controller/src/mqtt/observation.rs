use std::time::{Duration, Instant};

use serde_json::Value;

use crate::time::Clock;

/// Convert z2m's optional `last_seen` field to the process monotonic clock.
/// `epoch` mode is milliseconds; ISO modes are RFC 3339 timestamps.
pub(crate) fn z2m_observed_at(payload: &Value, clock: &dyn Clock) -> Option<Instant> {
    let value = payload.get("last_seen")?;
    let observed_epoch_ms = value.as_u64().or_else(|| {
        let timestamp = value.as_str()?;
        let parsed = chrono::DateTime::parse_from_rfc3339(timestamp).ok()?;
        u64::try_from(parsed.timestamp_millis()).ok()
    })?;
    let age_ms = clock.epoch_millis().checked_sub(observed_epoch_ms)?;
    clock.now().checked_sub(Duration::from_millis(age_ms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::FakeClock;

    #[test]
    fn parses_iso_and_epoch_last_seen() {
        let clock = FakeClock::new(12);
        for payload in [
            serde_json::json!({"last_seen": "2023-11-14T22:12:20Z"}),
            serde_json::json!({"last_seen": 1_699_999_940_000_u64}),
        ] {
            let observed = z2m_observed_at(&payload, &clock).unwrap();
            assert_eq!(clock.now().duration_since(observed), Duration::from_secs(60));
        }
    }

    #[test]
    fn rejects_missing_invalid_or_future_last_seen() {
        let clock = FakeClock::new(12);
        for payload in [
            serde_json::json!({}),
            serde_json::json!({"last_seen": "not-a-timestamp"}),
            serde_json::json!({"last_seen": 1_700_000_000_001_u64}),
        ] {
            assert_eq!(z2m_observed_at(&payload, &clock), None);
        }
    }
}
