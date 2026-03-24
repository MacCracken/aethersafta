//! Frame timing, latency tracking, and scheduling.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Frame clock: tracks target framerate and provides frame timing.
#[derive(Debug, Clone)]
pub struct FrameClock {
    fps: u32,
    frame_duration: Duration,
    frame_count: u64,
    start_time: Option<Instant>,
}

impl FrameClock {
    /// Create a new frame clock for the given framerate.
    #[must_use]
    pub fn new(fps: u32) -> Self {
        Self {
            fps,
            frame_duration: Duration::from_nanos(1_000_000_000 / fps.max(1) as u64),
            frame_count: 0,
            start_time: None,
        }
    }

    /// Target framerate.
    #[must_use]
    pub fn fps(&self) -> u32 {
        self.fps
    }

    /// Duration of a single frame.
    #[must_use]
    pub fn frame_duration(&self) -> Duration {
        self.frame_duration
    }

    /// Total frames produced.
    #[must_use]
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// PTS in microseconds for the current frame.
    #[must_use]
    pub fn current_pts_us(&self) -> u64 {
        self.frame_count * self.frame_duration.as_micros() as u64
    }

    /// Advance the clock by one frame.
    pub fn tick(&mut self) {
        if self.start_time.is_none() {
            self.start_time = Some(Instant::now());
        }
        self.frame_count += 1;
    }

    /// Elapsed wall time since start.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.start_time
            .map(|s| s.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    /// Whether we're behind schedule (need to drop frames).
    #[must_use]
    pub fn is_behind(&self) -> bool {
        if let Some(start) = self.start_time {
            let expected_us = self.frame_duration.as_micros() as u64 * self.frame_count;
            let expected = Duration::from_micros(expected_us);
            start.elapsed() > expected + self.frame_duration
        } else {
            false
        }
    }
}

/// Per-stage latency budget for the compositing pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyBudget {
    /// Target total pipeline latency.
    pub target: Duration,
    /// Measured capture time.
    pub capture_us: u64,
    /// Measured composite time.
    pub composite_us: u64,
    /// Measured encode time.
    pub encode_us: u64,
    /// Measured output time.
    pub output_us: u64,
}

impl LatencyBudget {
    /// Create a budget with the given target latency.
    #[must_use]
    pub fn new(target: Duration) -> Self {
        Self {
            target,
            capture_us: 0,
            composite_us: 0,
            encode_us: 0,
            output_us: 0,
        }
    }

    /// Total measured latency in microseconds.
    #[must_use]
    pub fn total_us(&self) -> u64 {
        self.capture_us + self.composite_us + self.encode_us + self.output_us
    }

    /// Whether the pipeline is within budget.
    #[must_use]
    pub fn within_budget(&self) -> bool {
        self.total_us() <= self.target.as_micros() as u64
    }

    /// How much headroom remains (negative = over budget).
    #[must_use]
    pub fn headroom_us(&self) -> i64 {
        self.target.as_micros() as i64 - self.total_us() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_clock_30fps() {
        let clock = FrameClock::new(30);
        assert_eq!(clock.fps(), 30);
        assert_eq!(clock.frame_count(), 0);
        // ~33.3ms per frame
        assert!(clock.frame_duration().as_micros() > 33000);
        assert!(clock.frame_duration().as_micros() < 34000);
    }

    #[test]
    fn frame_clock_tick() {
        let mut clock = FrameClock::new(60);
        assert_eq!(clock.current_pts_us(), 0);
        clock.tick();
        assert_eq!(clock.frame_count(), 1);
        assert!(clock.current_pts_us() > 0);
    }

    #[test]
    fn latency_budget_within() {
        let mut budget = LatencyBudget::new(Duration::from_millis(33));
        budget.capture_us = 5000;
        budget.composite_us = 3000;
        budget.encode_us = 10000;
        budget.output_us = 2000;
        assert_eq!(budget.total_us(), 20000);
        assert!(budget.within_budget());
        assert!(budget.headroom_us() > 0);
    }

    #[test]
    fn latency_budget_over() {
        let mut budget = LatencyBudget::new(Duration::from_millis(16));
        budget.capture_us = 8000;
        budget.composite_us = 5000;
        budget.encode_us = 8000;
        budget.output_us = 2000;
        assert!(!budget.within_budget());
        assert!(budget.headroom_us() < 0);
    }

    #[test]
    fn frame_clock_pts_increments() {
        let mut clock = FrameClock::new(30);
        clock.tick();
        let pts1 = clock.current_pts_us();
        clock.tick();
        let pts2 = clock.current_pts_us();
        assert!(pts2 > pts1);
        // Two frames at 30fps: ~66.6ms difference
        assert!((pts2 - pts1) > 33000);
    }

    #[test]
    fn frame_clock_60fps() {
        let mut clock = FrameClock::new(60);
        for _ in 0..60 {
            clock.tick();
        }
        let pts = clock.current_pts_us();
        assert!(
            (950_000..=1_050_000).contains(&pts),
            "Expected PTS ~1s after 60 ticks at 60fps, got {pts} us"
        );
    }

    #[test]
    fn frame_clock_is_behind() {
        let mut clock = FrameClock::new(60);
        // Before any tick, is_behind should be false (no start_time).
        assert!(!clock.is_behind());
        // After a single tick the clock just started, should not be behind.
        clock.tick();
        assert!(!clock.is_behind());
    }

    #[test]
    fn latency_budget_zero_stages() {
        let budget = LatencyBudget::new(Duration::from_millis(33));
        assert!(budget.within_budget());
        assert_eq!(budget.headroom_us(), budget.target.as_micros() as i64);
    }

    // --- Additional coverage tests ---

    #[test]
    fn frame_clock_zero_fps_no_panic() {
        // fps=0 should be handled via max(1), producing 1fps
        let clock = FrameClock::new(0);
        assert_eq!(clock.fps(), 0);
        // frame_duration uses fps.max(1) so it should be 1s
        assert_eq!(clock.frame_duration().as_secs(), 1);
        assert_eq!(clock.current_pts_us(), 0);
    }

    #[test]
    fn frame_clock_elapsed_before_tick() {
        // Before any tick, start_time is None, elapsed should be ZERO
        let clock = FrameClock::new(30);
        assert_eq!(clock.elapsed(), Duration::ZERO);
    }

    #[test]
    fn frame_clock_elapsed_after_tick() {
        // After tick, start_time is Some, elapsed should be non-negative
        let mut clock = FrameClock::new(30);
        clock.tick();
        // elapsed() should return a small positive duration (just ticked)
        let e = clock.elapsed();
        assert!(
            e < Duration::from_secs(1),
            "elapsed should be tiny, got {:?}",
            e
        );
    }

    #[test]
    fn frame_clock_current_pts_us_at_high_frame_count() {
        // Test pts calculation with many ticks — checks multiplication correctness
        let mut clock = FrameClock::new(30);
        for _ in 0..1000 {
            clock.tick();
        }
        let pts = clock.current_pts_us();
        // 1000 frames at 30fps = ~33.33s = ~33_333_333 us
        assert!(
            (33_000_000..=34_000_000).contains(&pts),
            "Expected PTS ~33.3s, got {} us",
            pts
        );
    }

    #[test]
    fn frame_clock_current_pts_us_before_tick() {
        // frame_count = 0 means pts = 0
        let clock = FrameClock::new(60);
        assert_eq!(clock.current_pts_us(), 0);
    }

    #[test]
    fn frame_clock_is_behind_no_start() {
        // No ticks => no start_time => not behind
        let clock = FrameClock::new(30);
        assert!(!clock.is_behind());
    }

    #[test]
    fn frame_clock_tick_sets_start_time_once() {
        // First tick sets start_time, second tick does not reset it
        let mut clock = FrameClock::new(60);
        clock.tick();
        let e1 = clock.elapsed();
        // Tick again — elapsed should only grow, not reset
        clock.tick();
        let e2 = clock.elapsed();
        assert!(e2 >= e1);
        assert_eq!(clock.frame_count(), 2);
    }

    #[test]
    fn frame_clock_1fps() {
        let mut clock = FrameClock::new(1);
        assert_eq!(clock.frame_duration(), Duration::from_secs(1));
        clock.tick();
        let pts = clock.current_pts_us();
        assert_eq!(pts, 1_000_000);
    }

    #[test]
    fn latency_budget_exact() {
        let mut budget = LatencyBudget::new(Duration::from_micros(33333));
        budget.capture_us = 10000;
        budget.composite_us = 8333;
        budget.encode_us = 10000;
        budget.output_us = 5000;
        assert_eq!(budget.total_us(), 33333);
        assert!(budget.within_budget());
        assert_eq!(budget.headroom_us(), 0);
    }
}
