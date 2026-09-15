//! Release velocity and frame-rate-independent momentum for canvas panning.
//! Port of momentum.ts and its tests. UI passes recent pointer samples and time.
//! Velocity stays in screen pixels per millisecond; the caller converts by zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub x: f64,
    pub y: f64,
    pub t: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Velocity {
    pub vx: f64,
    pub vy: f64,
}
pub const VELOCITY_SAMPLE_MS: f64 = 60.;
pub const MOMENTUM_DECAY_MS: f64 = 120.;
pub const MOMENTUM_MIN_VELOCITY: f64 = 0.05;
pub fn velocity_from(samples: &[Sample], now: f64) -> Velocity {
    let mut recent = samples.iter().filter(|s| now - s.t <= VELOCITY_SAMPLE_MS);
    let Some(first) = recent.next() else {
        return Velocity::default();
    };
    let Some(last) = recent.next_back() else {
        return Velocity::default();
    };
    let dt = last.t - first.t;
    if dt <= 0. {
        return Velocity::default();
    }
    Velocity {
        vx: (last.x - first.x) / dt,
        vy: (last.y - first.y) / dt,
    }
}
pub fn decay_velocity(v: f64, dt_ms: f64) -> f64 {
    v * (-dt_ms / MOMENTUM_DECAY_MS).exp()
}
pub fn should_glide(v: Velocity) -> bool {
    v.vx.hypot(v.vy) >= MOMENTUM_MIN_VELOCITY
}

#[cfg(test)]
mod tests {
    use super::*;
    fn s(x: f64, y: f64, t: f64) -> Sample {
        Sample { x, y, t }
    }
    #[test]
    fn velocity_in_screen_pixels_per_ms() {
        assert_eq!(
            velocity_from(&[s(0., 0., 1000.), s(100., 0., 1050.)], 1050.),
            Velocity { vx: 2., vy: 0. }
        );
    }
    #[test]
    fn both_axes() {
        assert_eq!(
            velocity_from(&[s(0., 0., 0.), s(20., -40., 20.)], 20.),
            Velocity { vx: 1., vy: -2. }
        );
    }
    #[test]
    fn old_samples_ignored() {
        assert_eq!(
            velocity_from(&[s(0., 0., 0.), s(500., 0., 10.)], 400.),
            Velocity::default()
        );
    }
    #[test]
    fn held_still_release_has_no_velocity() {
        assert_eq!(
            velocity_from(
                &[
                    s(0., 0., 0.),
                    s(200., 0., 30.),
                    s(200., 0., 300.),
                    s(200., 0., 330.)
                ],
                330.
            ),
            Velocity::default()
        );
    }
    #[test]
    fn too_few_samples() {
        assert_eq!(velocity_from(&[], 0.), Velocity::default());
        assert_eq!(velocity_from(&[s(5., 5., 0.)], 0.), Velocity::default());
    }
    #[test]
    fn identical_timestamps() {
        assert_eq!(
            velocity_from(&[s(0., 0., 100.), s(50., 0., 100.)], 100.),
            Velocity::default()
        );
    }
    #[test]
    fn sample_window_tracks_flick() {
        const {
            assert!(VELOCITY_SAMPLE_MS <= 100.);
        }
    }
    #[test]
    fn decay_frame_rate_independent() {
        assert!((decay_velocity(1., MOMENTUM_DECAY_MS) - (-1.0_f64).exp()).abs() < 1e-5);
        assert!(
            (decay_velocity(decay_velocity(1., 8.), 8.) - decay_velocity(1., 16.)).abs() < 1e-10
        );
    }
    #[test]
    fn minimum_glide_velocity() {
        assert!(should_glide(Velocity { vx: 2., vy: 0. }));
        assert!(should_glide(Velocity { vx: 0., vy: -2. }));
        assert!(!should_glide(Velocity::default()));
        assert!(!should_glide(Velocity {
            vx: MOMENTUM_MIN_VELOCITY / 2.,
            vy: 0.
        }));
    }
    #[test]
    fn diagonal_uses_magnitude() {
        let each = MOMENTUM_MIN_VELOCITY * 0.8;
        assert!(!should_glide(Velocity { vx: each, vy: 0. }));
        assert!(should_glide(Velocity { vx: each, vy: each }));
    }
}
