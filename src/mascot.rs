//! Sui, the droplet that lives in the notch. It blinks and glances around while you work,
//! dozes off when the PC is quiet, waves when you come back, and cheers for finished goals.

use std::time::{Duration, Instant};

use crate::anim::{ease_out_cubic, Spring};
use crate::gfx::{Canvas, C};

const DOZE_AFTER_S: u32 = 90;
const WAVE_FOR: Duration = Duration::from_millis(1800);
const BLINK_FOR: Duration = Duration::from_millis(160);
const CHEER_FOR: Duration = Duration::from_millis(2600);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mood {
    Awake,
    Dozing,
    Waving,
    /// A short celebration: hops with both arms up.
    Cheering,
}

pub struct Mascot {
    pub mood: Mood,
    mood_since: Instant,
    blink_at: Instant,
    look_at: Instant,
    look: Spring,
    squash: Spring,
    seed: u32,
}

impl Mascot {
    pub fn new() -> Self {
        let now = Instant::now();
        Mascot {
            mood: Mood::Awake,
            mood_since: now,
            blink_at: now + Duration::from_secs(3),
            look_at: now + Duration::from_secs(5),
            look: Spring::new(0.0, 0.5, 0.7),
            squash: Spring::new(0.0, 0.35, 0.45),
            seed: 0x9e37_79b9,
        }
    }

    fn rand(&mut self) -> f32 {
        // xorshift; plenty for picking blink times.
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed % 10_000) as f32 / 10_000.0
    }

    /// Updates mood from user idle time. Returns true when Sui needs to animate.
    pub fn update(&mut self, idle_s: u32, dt: f32) -> bool {
        let now = Instant::now();
        match self.mood {
            Mood::Awake if idle_s >= DOZE_AFTER_S => self.set_mood(Mood::Dozing),
            Mood::Dozing if idle_s < 2 => {
                self.set_mood(Mood::Waving);
                self.squash.kick(6.0);
            }
            Mood::Waving if now - self.mood_since > WAVE_FOR => self.set_mood(Mood::Awake),
            Mood::Cheering if now - self.mood_since > CHEER_FOR => self.set_mood(Mood::Awake),
            _ => {}
        }
        if self.mood == Mood::Awake {
            if now >= self.look_at {
                let r = self.rand();
                self.look.set(if r < 0.33 { -1.0 } else if r < 0.66 { 1.0 } else { 0.0 });
                self.look_at = now + Duration::from_millis(2500 + (self.rand() * 4000.0) as u64);
            }
            if now >= self.blink_at + BLINK_FOR {
                self.blink_at = now + Duration::from_millis(2200 + (self.rand() * 4200.0) as u64);
            }
        }
        let moving = self.look.step(dt) | self.squash.step(dt);
        moving || self.mood != Mood::Awake || self.blinking()
    }

    fn set_mood(&mut self, mood: Mood) {
        self.mood = mood;
        self.mood_since = Instant::now();
        if mood != Mood::Awake {
            self.look.set(0.0);
        }
    }

    pub fn cheer(&mut self) {
        self.set_mood(Mood::Cheering);
        self.squash.kick(7.0);
    }

    pub fn poke(&mut self) {
        self.squash.kick(5.0);
        self.blink_at = Instant::now();
    }

    fn blinking(&self) -> bool {
        let now = Instant::now();
        now >= self.blink_at && now < self.blink_at + BLINK_FOR
    }

    /// Milliseconds until the next scheduled change while otherwise still.
    pub fn next_wake_ms(&self) -> u64 {
        let now = Instant::now();
        let to = |t: Instant| t.saturating_duration_since(now).as_millis() as u64;
        to(self.blink_at).min(to(self.look_at)).max(16)
    }

    /// Draws Sui centered on (cx, cy); `size` is the droplet height.
    pub fn draw(&self, cv: &Canvas, cx: f32, cy: f32, size: f32, body: C) {
        self.draw_mood(cv, self.mood, self.mood_since.elapsed().as_secs_f32(), cx, cy, size, body);
    }

    /// Sui asleep, whatever the current mood (used beside a long break); `t` drives the z's.
    pub fn draw_dozing(&self, cv: &Canvas, t: f32, cx: f32, cy: f32, size: f32, body: C) {
        self.draw_mood(cv, Mood::Dozing, t, cx, cy, size, body);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_mood(&self, cv: &Canvas, mood: Mood, t: f32, cx: f32, cy: f32, size: f32, body: C) {
        let squash = self.squash.value * 0.05;
        let breathe = if mood == Mood::Dozing { (t * 1.6).sin() * 0.035 } else { 0.0 };
        let sy = 1.0 - squash + breathe;
        let sx = 1.0 + squash * 0.8 - breathe * 0.5;
        let bounce = match mood {
            Mood::Waving => -((t * 9.0).sin().abs()) * size * 0.08 * (1.0 - t / 1.8).max(0.0),
            Mood::Cheering => -((t * 7.0).sin().abs()) * size * 0.16 * (1.0 - t / 2.6).max(0.0),
            _ => 0.0,
        };
        let h = size * sy;
        let w = size * 0.78 * sx;
        let base = cy + size / 2.0 + bounce;
        drop_shape(cv, cx, base, w, h, body);

        let eye_y = base - h * 0.34;
        let gap = w * 0.2;
        let look = self.look.value * w * 0.07;
        let eye = C(0.02, 0.03, 0.05, 1.0);
        match mood {
            Mood::Dozing => {
                for dx in [-gap, gap] {
                    let x = cx + dx;
                    cv.arc(x, eye_y - size * 0.03, size * 0.07, 2.0, 4.28, eye, size * 0.045);
                }
                for i in 0..3 {
                    let phase = (t * 0.5 + i as f32 / 3.0).fract();
                    let a = ease_out_cubic(phase);
                    let zx = cx + w * 0.55 + a * size * 0.5;
                    let zy = base - h - a * size * 0.45;
                    let zs = size * (0.22 + a * 0.18);
                    cv.text("z", crate::gfx::R::centered(zx, zy, zs * 2.0, zs * 2.0), zs, 700, body.a(1.0 - phase), crate::gfx::Align::Center);
                }
            }
            Mood::Cheering => {
                // Happy closed eyes, both arms up, and a few sparkles around.
                for dx in [-gap, gap] {
                    cv.arc(cx + dx, eye_y + size * 0.03, size * 0.07, -1.1, 1.1, eye, size * 0.045);
                }
                let swing = (t * 12.0).sin() * size * 0.05;
                for side in [-1.0f32, 1.0] {
                    let (sx, sy) = (cx + side * w * 0.42, base - h * 0.32);
                    let (hx, hy) = (cx + side * (w * 0.7 + swing), base - h * 0.95);
                    cv.line(sx, sy, hx, hy, body, size * 0.09);
                    cv.fill_circle(hx, hy, size * 0.085, body);
                }
                for i in 0..3 {
                    let phase = (t * 0.9 + i as f32 / 3.0).fract();
                    let ang = i as f32 * 2.1 + t;
                    let (px, py) = (cx + ang.cos() * size * 0.85, base - h * 0.6 + ang.sin() * size * 0.5);
                    cv.fill_circle(px, py, size * 0.05 * (1.0 - phase), body.a(1.0 - phase));
                }
            }
            _ => {
                let open = if self.blinking() { 0.12 } else if mood == Mood::Waving { 1.2 } else { 1.0 };
                for dx in [-gap, gap] {
                    cv.fill_ellipse(cx + dx + look, eye_y, size * 0.055, size * 0.075 * open, eye);
                }
                if mood == Mood::Waving {
                    let swing = (t * 14.0).sin() * 0.5;
                    let hx = cx + w * 0.62 + swing.sin() * size * 0.08;
                    let hy = base - h * 0.45 - swing.cos().abs() * size * 0.12;
                    cv.line(cx + w * 0.42, base - h * 0.32, hx, hy, body, size * 0.09);
                    cv.fill_circle(hx, hy, size * 0.085, body);
                }
            }
        }
    }
}

/// A droplet whose round bottom sits on `base`, tip pointing up.
pub fn drop_shape(cv: &Canvas, cx: f32, base: f32, w: f32, h: f32, c: C) {
    let r = w / 2.0;
    let cy = base - r;
    let tip = base - h;
    let Some(mut p) = cv.path() else { return };
    // Tangent points from the tip to the circle, so the sides meet the body smoothly.
    let d = (cy - tip).max(r + 0.01);
    let ang = (r / d).acos();
    let (tx, ty) = (r * ang.sin(), r * ang.cos());
    // Straight sides, matching the logo's droplet.
    p.move_to(cx, tip)
        .line_to(cx + tx, cy - ty)
        .arc_to(cx, base, r, true)
        .arc_to(cx - tx, cy - ty, r, true)
        .line_to(cx, tip);
    let path = p.finish(true);
    cv.fill_path(&path, c);
}
