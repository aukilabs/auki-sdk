use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const FLASH_PERIOD: Duration = Duration::from_secs(3);
pub const FLASH_ON: Duration = Duration::from_millis(180);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashMode {
    Utc,
    Domain,
}

pub fn peer_suffix(peer_id: &str) -> String {
    let suffix: String = peer_id
        .chars()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("...{suffix}")
}

pub fn utc_now_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

pub fn next_period_boundary_ns(now_ns: u128, period_ns: u128) -> u128 {
    ((now_ns / period_ns) + 1) * period_ns
}

pub fn elapsed_in_period_ns(now_ns: u128, period_ns: u128) -> u128 {
    now_ns % period_ns
}

pub fn flash_is_on(now_ns: u128, period_ns: u128, flash_on_ns: u128) -> bool {
    elapsed_in_period_ns(now_ns, period_ns) < flash_on_ns
}

pub fn flash_is_on_i64(now_ns: i64, period_ns: u128, flash_on_ns: u128) -> bool {
    let Ok(now_ns) = u128::try_from(now_ns) else {
        return false;
    };
    flash_is_on(now_ns, period_ns, flash_on_ns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_suffix_uses_last_six_characters() {
        assert_eq!(peer_suffix("12D3KooWabcdefgh"), "...cdefgh");
    }

    #[test]
    fn peer_suffix_handles_short_ids() {
        assert_eq!(peer_suffix("abc"), "...abc");
    }

    #[test]
    fn next_boundary_uses_strictly_future_period() {
        assert_eq!(next_period_boundary_ns(0, 3_000), 3_000);
        assert_eq!(next_period_boundary_ns(2_999, 3_000), 3_000);
        assert_eq!(next_period_boundary_ns(3_000, 3_000), 6_000);
    }

    #[test]
    fn flash_is_on_inside_opening_window() {
        assert!(flash_is_on(6_050, 3_000, 180));
        assert!(!flash_is_on(6_250, 3_000, 180));
    }

    #[test]
    fn flash_is_on_i64_rejects_negative_domain_time() {
        assert!(!flash_is_on_i64(-1, 3_000, 180));
        assert!(flash_is_on_i64(6_050, 3_000, 180));
    }
}
