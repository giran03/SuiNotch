//! Opt-in frame profiler. Set SUINOTCH_PROFILE=1 to log, per animation, how evenly frames
//! arrived and what drawing and presenting cost. SUINOTCH_PROFILE_DEMO=1 also opens and
//! closes the notch by itself, then exits, so runs can be compared without touching the mouse.

use std::cell::RefCell;
use std::sync::OnceLock;

pub const DEMO_TOGGLES: u32 = 12;
pub const DEMO_INTERVAL_MS: u32 = 1300;

pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SUINOTCH_PROFILE").is_some() || demo())
}

pub fn demo() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SUINOTCH_PROFILE_DEMO").is_some())
}

#[derive(Default)]
struct Burst {
    intervals: Vec<f32>,
    draw: Vec<f32>,
    present: Vec<f32>,
}

thread_local! {
    static BURST: RefCell<Burst> = RefCell::new(Burst::default());
}

/// One animated frame: time since the previous frame, and the cost of drawing and presenting it.
pub fn record(interval_ms: f32, draw_ms: f32, present_ms: f32) {
    if !enabled() {
        return;
    }
    BURST.with(|b| {
        let mut b = b.borrow_mut();
        b.intervals.push(interval_ms);
        b.draw.push(draw_ms);
        b.present.push(present_ms);
    });
}

thread_local! {
    static COUNTS: RefCell<(std::time::Instant, std::collections::BTreeMap<String, u32>)> =
        RefCell::new((std::time::Instant::now(), Default::default()));
}

const COUNT_WINDOW_S: u64 = 10;

/// Counts how often something happens; every 10 s the per-second rates are logged.
pub fn count(what: &str) {
    if !enabled() {
        return;
    }
    COUNTS.with(|c| {
        let mut c = c.borrow_mut();
        *c.1.entry(what.to_string()).or_default() += 1;
        if c.0.elapsed().as_secs() >= COUNT_WINDOW_S {
            let secs = c.0.elapsed().as_secs_f32();
            let rates: Vec<String> = c.1.iter().map(|(k, v)| format!("{k} {:.1}/s", *v as f32 / secs)).collect();
            crate::log(&format!("rates: {}", rates.join(", ")));
            *c = (std::time::Instant::now(), Default::default());
        }
    });
}

/// The animation came to rest: log its summary.
pub fn end_burst() {
    if !enabled() {
        return;
    }
    let b = BURST.with(|b| std::mem::take(&mut *b.borrow_mut()));
    if b.intervals.len() < 3 {
        return;
    }
    // The first interval spans the idle gap before the animation started.
    let intervals = &b.intervals[1..];
    crate::log(&summary(intervals, &b.draw, &b.present));
}

pub fn summary(intervals: &[f32], draw: &[f32], present: &[f32]) -> String {
    let median = percentile(intervals, 0.5);
    let hitches = intervals.iter().filter(|&&i| i > median * 1.5).count();
    format!(
        "frames {} | interval median {:.1} p95 {:.1} max {:.1} ms, hitches {} | draw avg {:.2} max {:.2} | present avg {:.2} max {:.2} ms",
        intervals.len(),
        median,
        percentile(intervals, 0.95),
        intervals.iter().copied().fold(0.0, f32::max),
        hitches,
        mean(draw),
        draw.iter().copied().fold(0.0, f32::max),
        mean(present),
        present.iter().copied().fold(0.0, f32::max),
    )
}

fn mean(v: &[f32]) -> f32 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f32>() / v.len() as f32
    }
}

fn percentile(v: &[f32], p: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(f32::total_cmp);
    s[((s.len() - 1) as f32 * p).round() as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_counts_hitches_against_the_median() {
        let s = summary(&[16.0, 16.0, 17.0, 16.0, 40.0], &[1.0], &[2.0]);
        assert!(s.contains("frames 5"));
        assert!(s.contains("median 16.0"));
        assert!(s.contains("hitches 1"));
    }
}
