//! When to run firstmate's bearings snapshot again. A run is never on a timer: something must
//! invalidate the last one first (a status file changed, a turn finished, the captain answered a
//! card) or the captain must ask with Refresh. Invalidations that arrive close together share one
//! run, and runs that invalidations start are at least [`Schedule::gap`] apart, so a busy fleet
//! costs one run per gap, with one run after the last change.

use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Schedule {
    /// How long a run waits after the first invalidation, so changes made together share it.
    settle: Duration,
    /// The least time between the starts of two runs that invalidations asked for.
    gap: Duration,
    /// When the next run is due, if one is.
    due: Option<Instant>,
    /// When the last run started.
    last: Option<Instant>,
}

impl Schedule {
    pub fn new(settle: Duration, gap: Duration) -> Schedule {
        Schedule {
            settle,
            gap,
            due: None,
            last: None,
        }
    }

    /// The last run may be out of date. A run already due stays due when it was; otherwise
    /// one is due after the settle time, and no sooner than a gap after the last run.
    pub fn invalidate(&mut self, now: Instant) {
        if self.due.is_none() {
            let settled = now + self.settle;
            let spaced = self.last.map_or(settled, |last| last + self.gap);
            self.due = Some(settled.max(spaced));
        }
    }

    /// The captain asked for a run: it is due now, whatever the gap.
    pub fn refresh(&mut self, now: Instant) {
        self.due = Some(now);
    }

    /// Whether a run is due at `now`; if one is, it counts as started.
    pub fn start(&mut self, now: Instant) -> bool {
        if self.due.is_some_and(|due| due <= now) {
            self.due = None;
            self.last = Some(now);
            true
        } else {
            false
        }
    }

    /// How long until the next run is due, if one is.
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        self.due.map(|due| due.saturating_duration_since(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTLE: Duration = Duration::from_millis(300);
    const GAP: Duration = Duration::from_secs(5);

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn nothing_runs_until_something_invalidates_the_last_run() {
        let t0 = Instant::now();
        let mut s = Schedule::new(SETTLE, GAP);
        assert!(!s.start(at(t0, 60_000)));
        assert_eq!(s.wait(t0), None);
    }

    #[test]
    fn changes_made_together_share_one_run_after_the_settle_time() {
        let t0 = Instant::now();
        let mut s = Schedule::new(SETTLE, GAP);
        s.invalidate(t0);
        s.invalidate(at(t0, 100));
        s.invalidate(at(t0, 200));
        assert!(!s.start(at(t0, 299)));
        assert_eq!(s.wait(at(t0, 100)), Some(Duration::from_millis(200)));
        assert!(s.start(at(t0, 300)));
        assert!(!s.start(at(t0, 301)), "one run for the three changes");
    }

    #[test]
    fn a_busy_fleet_costs_one_run_per_gap_and_one_after_the_last_change() {
        let t0 = Instant::now();
        let mut s = Schedule::new(SETTLE, GAP);
        s.invalidate(t0);
        assert!(s.start(at(t0, 300)));
        // A status line every second for twelve seconds.
        let mut runs = vec![300];
        for second in 1..=12 {
            let now = at(t0, second * 1000);
            if s.start(now) {
                runs.push(second * 1000);
            }
            s.invalidate(now);
        }
        // Then nothing changes; the last change still gets its run.
        for ms in (12_001..30_000).step_by(100) {
            if s.start(at(t0, ms)) {
                runs.push(ms);
            }
        }
        assert_eq!(runs, [300, 6000, 11_000, 16_001]);
    }

    #[test]
    fn refresh_runs_at_once_even_inside_the_gap() {
        let t0 = Instant::now();
        let mut s = Schedule::new(SETTLE, GAP);
        s.refresh(t0);
        assert!(s.start(t0));
        s.invalidate(at(t0, 1000));
        assert_eq!(s.wait(at(t0, 1000)), Some(Duration::from_secs(4)));
        s.refresh(at(t0, 1000));
        assert!(s.start(at(t0, 1000)));
    }
}
