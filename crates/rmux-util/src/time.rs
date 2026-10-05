// Ported from tmux compat.h @ 8f25579c (timercmp, timeradd, timersub; tmux.c get_timer)
//! `Timestamp` is `struct timeval`; `MonotonicTime` wraps `std::time::Instant`.

use std::cmp::Ordering;
use std::ops::{Add, Sub};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const USEC_PER_SEC: i32 = 1_000_000;

/// `struct timeval`: seconds and microseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Timestamp {
    pub sec: i64,
    pub usec: i32,
}

impl Timestamp {
    pub const ZERO: Timestamp = Timestamp { sec: 0, usec: 0 };

    pub const fn new(sec: i64, usec: i32) -> Timestamp {
        Timestamp { sec, usec }
    }

    /// `gettimeofday`.
    pub fn now() -> Timestamp {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => Timestamp::from(d),
            Err(e) => {
                let d = e.duration();
                Timestamp::ZERO - Timestamp::from(d)
            }
        }
    }
}

/// `timeradd`: carries once when `usec >= 1000000`.
impl Add for Timestamp {
    type Output = Timestamp;

    fn add(self, o: Timestamp) -> Timestamp {
        let mut sec = self.sec.wrapping_add(o.sec);
        let mut usec = self.usec.wrapping_add(o.usec);
        if usec >= USEC_PER_SEC {
            sec = sec.wrapping_add(1);
            usec -= USEC_PER_SEC;
        }
        Timestamp { sec, usec }
    }
}

/// `timersub`: borrows once when `usec < 0`.
impl Sub for Timestamp {
    type Output = Timestamp;

    fn sub(self, o: Timestamp) -> Timestamp {
        let mut sec = self.sec.wrapping_sub(o.sec);
        let mut usec = self.usec.wrapping_sub(o.usec);
        if usec < 0 {
            sec = sec.wrapping_sub(1);
            usec += USEC_PER_SEC;
        }
        Timestamp { sec, usec }
    }
}

impl Timestamp {
    /// Both fields zero (`timerisset` false).
    pub const fn is_zero(self) -> bool {
        self.sec == 0 && self.usec == 0
    }

    /// Non-negative value as a `Duration`; `None` when negative.
    pub fn to_duration(self) -> Option<Duration> {
        let sec = u64::try_from(self.sec).ok()?;
        let usec = u32::try_from(self.usec).ok()?;
        Some(Duration::new(sec, usec.checked_mul(1000)?))
    }
}

/// `timercmp`: seconds first, then microseconds.
impl Ord for Timestamp {
    fn cmp(&self, other: &Timestamp) -> Ordering {
        if self.sec == other.sec {
            self.usec.cmp(&other.usec)
        } else {
            self.sec.cmp(&other.sec)
        }
    }
}

impl PartialOrd for Timestamp {
    fn partial_cmp(&self, other: &Timestamp) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl From<Duration> for Timestamp {
    fn from(d: Duration) -> Timestamp {
        Timestamp {
            sec: i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
            usec: i32::try_from(d.subsec_micros()).unwrap_or(i32::MAX),
        }
    }
}

/// `clock_gettime(CLOCK_MONOTONIC)` as used by `get_timer` (`tmux.c:339-341`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MonotonicTime(pub Instant);

impl MonotonicTime {
    pub fn now() -> MonotonicTime {
        MonotonicTime(Instant::now())
    }

    pub fn elapsed(self) -> Duration {
        self.0.elapsed()
    }

    /// Saturates at zero when `earlier` is later, like `Instant::duration_since`.
    pub fn duration_since(self, earlier: MonotonicTime) -> Duration {
        self.0.duration_since(earlier.0)
    }

    /// `get_timer`: milliseconds, truncated.
    pub fn millis_since(self, earlier: MonotonicTime) -> u64 {
        u64::try_from(self.duration_since(earlier).as_millis()).unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeradd_carries_usec() {
        let a = Timestamp::new(1, 999_999);
        let b = Timestamp::new(2, 1);
        assert_eq!(a + b, Timestamp::new(4, 0));
        assert_eq!(
            Timestamp::new(1, 500_000) + Timestamp::new(0, 600_000),
            Timestamp::new(2, 100_000)
        );
        assert_eq!(
            Timestamp::new(1, 1) + Timestamp::new(1, 1),
            Timestamp::new(2, 2)
        );
    }

    #[test]
    fn timeradd_carries_once_only() {
        // compat.h:249-252 subtracts a single second even for usec >= 2000000.
        assert_eq!(
            Timestamp::new(0, 1_500_000) + Timestamp::new(0, 1_500_000),
            Timestamp::new(1, 2_000_000)
        );
    }

    #[test]
    fn timersub_borrows_usec() {
        let a = Timestamp::new(4, 0);
        let b = Timestamp::new(1, 1);
        assert_eq!(a - b, Timestamp::new(2, 999_999));
        assert_eq!(
            Timestamp::new(2, 100_000) - Timestamp::new(1, 500_000),
            Timestamp::new(0, 600_000)
        );
        assert_eq!(
            Timestamp::new(2, 2) - Timestamp::new(1, 1),
            Timestamp::new(1, 1)
        );
        assert_eq!(
            Timestamp::new(0, 0) - Timestamp::new(0, 1),
            Timestamp::new(-1, 999_999)
        );
    }

    #[test]
    fn timercmp_orders_sec_then_usec() {
        assert!(Timestamp::new(1, 999_999) < Timestamp::new(2, 0));
        assert!(Timestamp::new(2, 1) > Timestamp::new(2, 0));
        assert_eq!(
            Timestamp::new(2, 0).cmp(&Timestamp::new(2, 0)),
            Ordering::Equal
        );
        assert!(Timestamp::new(-1, 999_999) < Timestamp::ZERO);
    }

    #[test]
    fn zero_and_duration() {
        assert!(Timestamp::default().is_zero());
        assert!(!Timestamp::new(0, 1).is_zero());
        let d = Duration::new(5, 123_456_789);
        assert_eq!(Timestamp::from(d), Timestamp::new(5, 123_456));
        assert_eq!(
            Timestamp::new(5, 123_456).to_duration(),
            Some(Duration::new(5, 123_456_000))
        );
        assert_eq!(Timestamp::new(-1, 0).to_duration(), None);
    }

    #[test]
    fn now_is_after_epoch() {
        let t = Timestamp::now();
        assert!(t.sec > 1_600_000_000);
        assert!((0..USEC_PER_SEC).contains(&t.usec));
    }

    #[test]
    fn monotonic_is_monotonic() {
        let a = MonotonicTime::now();
        let b = MonotonicTime::now();
        assert!(b >= a);
        assert_eq!(a.duration_since(b), Duration::ZERO);
        assert!(b.duration_since(a) <= b.elapsed());
        assert_eq!(a.millis_since(a), 0);
    }
}
