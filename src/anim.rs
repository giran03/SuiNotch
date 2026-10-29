//! Spring physics used for every motion in SuiNotch. Springs keep velocity when the
//! target changes mid-flight, which is what makes the notch feel fluid instead of tweened.

const STEP: f32 = 1.0 / 240.0;
const REST_DELTA: f32 = 0.01;
const REST_SPEED: f32 = 0.05;

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
    pub target: f32,
    stiffness: f32,
    damping: f32,
}

impl Spring {
    /// `response` is the approximate settle period in seconds, `ratio` the damping ratio
    /// (1.0 = critically damped, below 1.0 overshoots a little).
    pub fn new(value: f32, response: f32, ratio: f32) -> Self {
        let mut s = Spring { value, velocity: 0.0, target: value, stiffness: 0.0, damping: 0.0 };
        s.tune(response, ratio);
        s
    }

    pub fn tune(&mut self, response: f32, ratio: f32) {
        let omega = 2.0 * std::f32::consts::PI / response.max(0.01);
        self.stiffness = omega * omega;
        self.damping = 2.0 * ratio * omega;
    }

    pub fn set(&mut self, target: f32) {
        self.target = target;
    }

    pub fn snap(&mut self, value: f32) {
        self.value = value;
        self.target = value;
        self.velocity = 0.0;
    }

    pub fn kick(&mut self, velocity: f32) {
        self.velocity += velocity;
    }

    pub fn is_resting(&self) -> bool {
        (self.value - self.target).abs() < REST_DELTA && self.velocity.abs() < REST_SPEED
    }

    /// Advances the spring by `dt` seconds. Returns true while it is still moving.
    pub fn step(&mut self, dt: f32) -> bool {
        if self.is_resting() {
            self.value = self.target;
            self.velocity = 0.0;
            return false;
        }
        let mut left = dt.min(0.1);
        while left > 0.0 {
            let h = left.min(STEP);
            let force = -self.stiffness * (self.value - self.target) - self.damping * self.velocity;
            self.velocity += force * h;
            self.value += self.velocity * h;
            left -= h;
        }
        true
    }
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn clamp01(t: f32) -> f32 {
    t.clamp(0.0, 1.0)
}

pub fn ease_out_cubic(t: f32) -> f32 {
    let t = clamp01(t) - 1.0;
    t * t * t + 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_reaches_target_and_rests() {
        let mut s = Spring::new(0.0, 0.4, 0.9);
        s.set(100.0);
        let mut frames = 0;
        while s.step(1.0 / 60.0) {
            frames += 1;
            assert!(frames < 600, "spring never settled");
        }
        assert_eq!(s.value, 100.0);
        assert!(frames > 5);
    }

    #[test]
    fn underdamped_spring_overshoots() {
        let mut s = Spring::new(0.0, 0.4, 0.6);
        s.set(1.0);
        let mut peak: f32 = 0.0;
        while s.step(1.0 / 60.0) {
            peak = peak.max(s.value);
        }
        assert!(peak > 1.0);
    }

    #[test]
    fn retarget_keeps_velocity() {
        let mut s = Spring::new(0.0, 0.4, 1.0);
        s.set(1.0);
        s.step(0.05);
        let v = s.velocity;
        s.set(-1.0);
        assert_eq!(s.velocity, v);
    }

    #[test]
    fn snap_stops_motion() {
        let mut s = Spring::new(0.0, 0.4, 1.0);
        s.set(5.0);
        s.step(0.02);
        s.snap(3.0);
        assert!(!s.step(0.016));
        assert_eq!(s.value, 3.0);
    }
}
