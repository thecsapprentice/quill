//! Hybrid logical clocks (protocol draft, section 5): wall-clock
//! milliseconds and a counter, written `1760861400123-0004`.
//!
//! A device's clock never runs behind anything it has sent or read, so a
//! reply sorts after what it answers even when device clocks disagree.

use std::{cmp::Ordering, fmt, str::FromStr};

/// Remote values further ahead of the local clock than this don't move it,
/// so one bad clock can't drag everyone's forward.
pub const MAX_AHEAD_MS: u64 = 24 * 60 * 60 * 1000;
/// The counter is 4 digits.
const MAX_COUNTER: u16 = 9999;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Hlc {
    pub ms: u64,
    pub counter: u16,
}

impl Ord for Hlc {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.ms, self.counter).cmp(&(other.ms, other.counter))
    }
}

impl PartialOrd for Hlc {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Hlc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{:04}", self.ms, self.counter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadHlc;

impl fmt::Display for BadHlc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a clock value")
    }
}

impl FromStr for Hlc {
    type Err = BadHlc;

    fn from_str(s: &str) -> Result<Self, BadHlc> {
        let (ms, counter) = s.split_once('-').ok_or(BadHlc)?;
        if ms.is_empty() || ms.len() > 16 || counter.len() != 4 {
            return Err(BadHlc);
        }
        if !ms.bytes().all(|b| b.is_ascii_digit()) || !counter.bytes().all(|b| b.is_ascii_digit()) {
            return Err(BadHlc);
        }
        Ok(Self {
            ms: ms.parse().map_err(|_| BadHlc)?,
            counter: counter.parse().map_err(|_| BadHlc)?,
        })
    }
}

impl Hlc {
    /// The value for a message sent now, after `last` (this device's clock).
    pub fn send(last: Self, now_ms: u64) -> Self {
        if now_ms > last.ms {
            Self {
                ms: now_ms,
                counter: 0,
            }
        } else {
            last.next()
        }
    }

    /// This device's clock after reading `remote`. Values too far ahead of
    /// `now_ms` are ignored.
    pub fn read(last: Self, remote: Self, now_ms: u64) -> Self {
        if remote.ms > now_ms.saturating_add(MAX_AHEAD_MS) {
            return last;
        }
        last.max(remote)
    }

    fn next(self) -> Self {
        if self.counter < MAX_COUNTER {
            Self {
                ms: self.ms,
                counter: self.counter + 1,
            }
        } else {
            // A full counter borrows a millisecond.
            Self {
                ms: self.ms + 1,
                counter: 0,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> Hlc {
        s.parse().unwrap()
    }

    #[test]
    fn round_trips_and_rejects_junk() {
        assert_eq!(h("1760861400123-0004").to_string(), "1760861400123-0004");
        assert_eq!(h("5-0000"), Hlc { ms: 5, counter: 0 });
        for bad in [
            "",
            "1-1",
            "1-00001",
            "-0001",
            "x-0001",
            "1760861400123",
            "1-00a1",
            "+1-0001",
        ] {
            assert!(bad.parse::<Hlc>().is_err(), "{bad}");
        }
    }

    #[test]
    fn a_reply_sorts_after_what_it_answers_whatever_the_clocks() {
        // Bob's clock is 10 seconds behind Alice's.
        let alice = Hlc::send(Hlc::default(), 1_000_000);
        let bob = Hlc::read(Hlc::default(), alice, 990_000);
        let reply = Hlc::send(bob, 990_000);
        assert!(reply > alice);
        assert_eq!(reply, h("1000000-0001"));
        // Later sends on a clock that has caught up move on normally.
        assert_eq!(Hlc::send(reply, 1_000_500), h("1000500-0000"));
    }

    #[test]
    fn a_wildly_wrong_clock_doesnt_drag_others_forward() {
        let now = 1_000_000;
        let far = Hlc {
            ms: now + MAX_AHEAD_MS + 1,
            counter: 0,
        };
        assert_eq!(Hlc::read(Hlc::default(), far, now), Hlc::default());
        let near = Hlc {
            ms: now + MAX_AHEAD_MS,
            counter: 0,
        };
        assert_eq!(Hlc::read(Hlc::default(), near, now), near);
    }

    #[test]
    fn the_counter_overflows_into_the_next_millisecond() {
        let full = Hlc {
            ms: 7,
            counter: 9999,
        };
        assert_eq!(Hlc::send(full, 7), Hlc { ms: 8, counter: 0 });
    }
}
