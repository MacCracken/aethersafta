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
    pub fn new(fps: u32) -> Self {
        Self {
            fps,
            frame_duration: Duration::from_nanos(1_000_000_000 / fps.max(1) as u64),
            frame_count: 0,
            start_time: None,
        }
    }

    /// Target framerate.
    pub fn fps(&self) -> u32 {
        self.fps
    }

    /// Duration of a single frame.
    pub fn frame_duration(&self) -> Duration {
        self.frame_duration
    }

    /// Total frames produced.
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// PTS in microseconds for the current frame.
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
    pub fn elapsed(&self) -> Duration {
        self.start_time
            .map(|s| s.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    /// Whether we're behind schedule (need to drop frames).
    pub fn is_behind(&self) -> bool {
        if let Some(start) = self.start_time {
            let expected = self.frame_duration * self.frame_count as u32;
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
    pub fn total_us(&self) -> u64 {
        self.capture_us + self.composite_us + self.encode_us + self.output_us
    }

    /// Whether the pipeline is within budget.
    pub fn within_budget(&self) -> bool {
        self.total_us() <= self.target.as_micros() as u64
    }

    /// How much headroom remains (negative = over budget).
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
}
