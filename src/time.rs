//! Time abstraction for dev/testing.
//!
//! In production, `now()` delegates to `Utc::now()`. In dev mode (`PILA_DEV_MODE=true`),
//! a mock time can be set via `/dev/time` to simulate tournament progression.
//!
//! The mock state lives in `AppState::mock_now` (Arc<RwLock<Option<DateTime<Utc>>>>).
//! When `None`, real time is used. When `Some(t)`, `t` is returned instead.

use chrono::{DateTime, Utc};
use std::sync::{Arc, RwLock};

/// Global mock time storage. `None` means use real time.
/// Wrapped in Arc for cheap cloning across request handlers.
/// Only set when `PILA_DEV_MODE=true`.
pub type MockTime = Arc<RwLock<Option<DateTime<Utc>>>>;

/// Create a new MockTime instance (always starts as None = real time).
pub fn new_mock_time() -> MockTime {
    Arc::new(RwLock::new(None))
}

/// Get the current time, using mock time if set.
///
/// This is the single source of truth for "now" in the application.
/// All handlers should use this instead of `Utc::now()`.
#[inline]
pub fn now(mock: &MockTime) -> DateTime<Utc> {
    // Fast path: read lock, check if mock is set
    // In production (mock always None), this is just a cheap read
    mock.read()
        .map(|guard| guard.unwrap_or_else(Utc::now))
        .unwrap_or_else(|_| Utc::now())
}

/// Set mock time. Only callable in dev mode.
pub fn set_mock_time(mock: &MockTime, t: DateTime<Utc>) {
    if let Ok(mut guard) = mock.write() {
        *guard = Some(t);
    }
}

/// Clear mock time, returning to real time. Only callable in dev mode.
pub fn clear_mock_time(mock: &MockTime) {
    if let Ok(mut guard) = mock.write() {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn defaults_to_real_time() {
        let mock = new_mock_time();
        let t1 = now(&mock);
        let t2 = Utc::now();
        // Should be within 1 second
        assert!((t2 - t1).num_seconds().abs() < 1);
    }

    #[test]
    fn mock_overrides_real_time() {
        let mock = new_mock_time();
        let fake = Utc.with_ymd_and_hms(2026, 6, 15, 12, 0, 0).unwrap();
        set_mock_time(&mock, fake);
        assert_eq!(now(&mock), fake);
    }

    #[test]
    fn clear_returns_to_real_time() {
        let mock = new_mock_time();
        let fake = Utc.with_ymd_and_hms(2026, 6, 15, 12, 0, 0).unwrap();
        set_mock_time(&mock, fake);
        clear_mock_time(&mock);
        let t1 = now(&mock);
        let t2 = Utc::now();
        assert!((t2 - t1).num_seconds().abs() < 1);
    }

    #[test]
    fn clone_shares_state() {
        let mock1 = new_mock_time();
        let mock2 = mock1.clone();
        let fake = Utc.with_ymd_and_hms(2026, 6, 15, 12, 0, 0).unwrap();
        set_mock_time(&mock1, fake);
        // Both clones should see the same mock time
        assert_eq!(now(&mock2), fake);
    }
}
