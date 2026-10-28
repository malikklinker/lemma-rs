//! Integer-frame transport shared by keyboard and pointer controls.
use crate::time::FrameRate;
use std::time::{Duration, Instant};

/// Wall time chooses an integer sample from one fixed anchor. Polling frequency
/// and dropped presentation frames cannot accumulate scene-time drift.
pub(super) struct Playhead {
    pub(super) fps: FrameRate,
    pub(super) last: u64,
    pub(super) frame: u64,
    pub(super) playing: bool,
    pub(super) looping: bool,
    pub(super) scrubbing: bool,
    anchor_frame: u64,
    anchor: Instant,
}

impl Playhead {
    pub(super) fn new(
        fps: FrameRate,
        last: u64,
        frame: u64,
        playing: bool,
        looping: bool,
        now: Instant,
    ) -> Self {
        Self {
            fps,
            last,
            frame,
            playing,
            looping,
            scrubbing: false,
            anchor_frame: frame,
            anchor: now,
        }
    }

    pub(super) fn advance(&mut self, now: Instant) -> bool {
        let old = (self.frame, self.playing);
        if self.playing && !self.scrubbing {
            let nanos = now.saturating_duration_since(self.anchor).as_nanos();
            let elapsed = nanos.saturating_mul(u128::from(self.fps.numerator()))
                / (1_000_000_000 * u128::from(self.fps.denominator()));
            let requested = u128::from(self.anchor_frame).saturating_add(elapsed);
            if self.looping {
                self.frame = (requested % (u128::from(self.last) + 1)) as u64;
            } else {
                self.frame = requested.min(u128::from(self.last)) as u64;
                if requested >= u128::from(self.last) {
                    self.playing = false;
                }
            }
        }
        old != (self.frame, self.playing)
    }

    pub(super) fn seek(&mut self, frame: u64, now: Instant) {
        self.frame = frame.min(self.last);
        self.anchor_frame = self.frame;
        self.anchor = now;
    }

    pub(super) fn toggle_play(&mut self, now: Instant) {
        self.end_scrub();
        self.advance(now);
        self.playing = !self.playing;
        if self.playing && !self.looping && self.frame == self.last {
            self.frame = 0;
        }
        self.seek(self.frame, now);
    }

    pub(super) fn step(&mut self, forwards: bool, now: Instant) {
        self.end_scrub();
        self.advance(now);
        self.playing = false;
        self.seek(
            if forwards {
                self.frame.saturating_add(1)
            } else {
                self.frame.saturating_sub(1)
            },
            now,
        );
    }

    pub(super) fn set_bounds(&mut self, fps: FrameRate, last: u64, now: Instant) {
        self.advance(now);
        self.fps = fps;
        self.last = last;
        self.seek(self.frame, now);
    }

    /// Scrubbing deliberately pauses transport, including after release.
    pub(super) fn begin_scrub(&mut self, frame: u64, now: Instant) {
        self.playing = false;
        self.scrubbing = true;
        self.seek(frame, now);
    }

    pub(super) fn scrub_to(&mut self, frame: u64, now: Instant) -> bool {
        if !self.scrubbing {
            return false;
        }
        let old = self.frame;
        self.seek(frame, now);
        old != self.frame
    }

    pub(super) fn end_scrub(&mut self) {
        self.scrubbing = false;
    }

    pub(super) fn redraw_interval(&self) -> Duration {
        // Presentation may skip samples at very high authored frame rates.
        let nanos = (1_000_000_000u64 * u64::from(self.fps.denominator()))
            .div_ceil(u64::from(self.fps.numerator()));
        Duration::from_nanos(nanos.max(4_166_667))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playhead_uses_rational_anchor_independent_of_polling() {
        let now = Instant::now();
        let fps = FrameRate::new(30_000, 1001).unwrap();
        let mut direct = Playhead::new(fps, 40_000, 0, true, false, now);
        let mut frequent = Playhead::new(fps, 40_000, 0, true, false, now);
        for ms in 0..=1001 {
            frequent.advance(now + Duration::from_millis(ms));
        }
        direct.advance(now + Duration::from_millis(1001));
        assert_eq!(direct.frame, 30);
        assert_eq!(frequent.frame, direct.frame);
        direct.advance(now + Duration::from_secs(1001));
        assert_eq!(direct.frame, 30_000);
    }

    #[test]
    fn playhead_pause_seek_step_loop_and_endpoint() {
        let now = Instant::now();
        let mut clock = Playhead::new(FrameRate::new(10, 1).unwrap(), 9, 0, true, true, now);
        clock.advance(now + Duration::from_millis(1200));
        assert_eq!(clock.frame, 2);
        clock.toggle_play(now + Duration::from_millis(1200));
        clock.advance(now + Duration::from_secs(100));
        assert_eq!(clock.frame, 2);
        clock.step(false, now + Duration::from_secs(100));
        assert_eq!(clock.frame, 1);
        clock.seek(9, now + Duration::from_secs(100));
        clock.step(true, now + Duration::from_secs(100));
        assert_eq!(clock.frame, 9);
        clock.looping = false;
        clock.toggle_play(now + Duration::from_secs(100));
        assert_eq!(clock.frame, 0);
        clock.advance(now + Duration::from_secs(102));
        assert_eq!(clock.frame, 9);
        assert!(!clock.playing);
    }

    #[test]
    fn scrub_drag_clamps_endpoints_and_stays_paused_after_release() {
        let now = Instant::now();
        let mut clock = Playhead::new(FrameRate::new(10, 1).unwrap(), 99, 10, true, true, now);
        assert!(!clock.scrub_to(80, now));
        assert_eq!(clock.frame, 10);
        clock.begin_scrub(40, now + Duration::from_secs(1));
        assert_eq!(clock.frame, 40);
        assert!(clock.scrubbing);
        assert!(!clock.playing);
        assert!(!clock.advance(now + Duration::from_secs(100)));
        assert!(clock.scrub_to(u64::MAX, now + Duration::from_secs(101)));
        assert_eq!(clock.frame, 99);
        assert!(clock.scrub_to(0, now + Duration::from_secs(102)));
        clock.end_scrub();
        assert!(!clock.scrubbing);
        assert!(!clock.playing);
        assert!(!clock.scrub_to(50, now + Duration::from_secs(103)));
        clock.advance(now + Duration::from_secs(1000));
        assert_eq!(clock.frame, 0);
    }

    #[test]
    fn scrub_bounds_reload_and_keyboard_resume_do_not_reintroduce_clock_drift() {
        let now = Instant::now();
        let mut clock = Playhead::new(FrameRate::new(10, 1).unwrap(), 99, 0, true, true, now);
        clock.begin_scrub(80, now);
        clock.set_bounds(FrameRate::new(30_000, 1001).unwrap(), 49, now);
        assert_eq!(clock.frame, 49);
        assert!(clock.scrubbing);
        assert!(!clock.playing);
        clock.scrub_to(20, now + Duration::from_secs(10));
        clock.toggle_play(now + Duration::from_secs(10));
        assert!(!clock.scrubbing);
        assert!(clock.playing);
        assert!(!clock.scrub_to(0, now + Duration::from_secs(10)));
        clock.advance(now + Duration::from_secs(10) + Duration::from_millis(1001));
        assert_eq!(clock.frame, 0); // 20 + 30 frames wraps once over 0..=49.
        clock.begin_scrub(40, now + Duration::from_secs(12));
        clock.step(false, now + Duration::from_secs(12));
        assert_eq!(clock.frame, 39);
        assert!(!clock.scrubbing);
        assert!(!clock.playing);
    }
}
