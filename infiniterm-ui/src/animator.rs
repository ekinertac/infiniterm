//! Drives the viewport through animated pans, anchored zooms, fits and the
//! momentum glide after a pan drag. Port of `viewportAnimator.svelte.ts`
//! and the glide half of `Canvas.svelte`, on gpui's frame callback instead
//! of requestAnimationFrame.
//!
//! One animation slot, so pan, zoom and fit can never fight for the
//! viewport; a new one mid-flight retargets from wherever it has got to.
//! Repeated zoom presses chain from the PENDING target (`Model::pending_scale`)
//! rather than the passing scale, or a held key compounds unevenly. Every
//! direct manipulation (wheel, drag, a fit) cancels first. With animations
//! off (the setting, or reduce motion) every target is applied at once.
use infiniterm_core::grid::{Point, Size};
use infiniterm_core::momentum::{decay_velocity, should_glide, Velocity};
use infiniterm_core::viewport::{
    anchored_viewport, ease_out_cubic, fit_frame, scale_at, Viewport, FIT_DURATION_MS,
    PAN_DURATION_MS, ZOOM_DURATION_MS,
};

enum Anim {
    Pan {
        from: Viewport,
        to: Point,
        started: f64,
    },
    Zoom {
        from_scale: f64,
        to_scale: f64,
        anchor_world: Point,
        anchor_screen: Point,
        started: f64,
    },
    Fit {
        from: Viewport,
        to: Viewport,
        started: f64,
    },
    Glide {
        v: Velocity,
        last: f64,
    },
}

#[derive(Default)]
pub struct Animator {
    anim: Option<Anim>,
    pub animations_on: bool,
}

impl Animator {
    pub fn new() -> Animator {
        Animator {
            anim: None,
            animations_on: true,
        }
    }

    pub fn cancel(&mut self) {
        self.anim = None;
    }

    pub fn is_running(&self) -> bool {
        self.anim.is_some()
    }

    /// The scale the in-flight zoom or fit is heading for.
    pub fn pending_scale(&self) -> Option<f64> {
        match &self.anim {
            Some(Anim::Zoom { to_scale, .. }) => Some(*to_scale),
            Some(Anim::Fit { to, .. }) => Some(to.scale),
            _ => None,
        }
    }

    /// Eased position, unlike the multiplicative zoom.
    pub fn pan(&mut self, vp: &mut Viewport, to: Point, now: f64) {
        if !self.animations_on {
            vp.x = to.x;
            vp.y = to.y;
            self.anim = None;
            return;
        }
        self.anim = Some(Anim::Pan {
            from: *vp,
            to,
            started: now,
        });
    }

    pub fn zoom(
        &mut self,
        vp: &mut Viewport,
        to_scale: f64,
        anchor_world: Point,
        anchor_screen: Point,
        now: f64,
    ) {
        if !self.animations_on {
            *vp = anchored_viewport(anchor_world, anchor_screen, to_scale);
            self.anim = None;
            return;
        }
        self.anim = Some(Anim::Zoom {
            from_scale: vp.scale,
            to_scale,
            anchor_world,
            anchor_screen,
            started: now,
        });
    }

    pub fn fit(&mut self, vp: &mut Viewport, to: Viewport, now: f64) {
        if !self.animations_on {
            *vp = to;
            self.anim = None;
            return;
        }
        self.anim = Some(Anim::Fit {
            from: *vp,
            to,
            started: now,
        });
    }

    pub fn glide(&mut self, v: Velocity, now: f64) {
        if self.animations_on && should_glide(v) {
            self.anim = Some(Anim::Glide { v, last: now });
        }
    }

    /// Advances the viewport for this frame. True while something moves.
    pub fn step(&mut self, vp: &mut Viewport, view: Size, now: f64) -> bool {
        match &mut self.anim {
            None => false,
            Some(Anim::Pan { from, to, started }) => {
                let t = ease_out_cubic((now - *started) / PAN_DURATION_MS);
                vp.x = from.x + (to.x - from.x) * t;
                vp.y = from.y + (to.y - from.y) * t;
                if t >= 1. {
                    self.anim = None;
                }
                true
            }
            Some(Anim::Zoom {
                from_scale,
                to_scale,
                anchor_world,
                anchor_screen,
                started,
            }) => {
                let t = ease_out_cubic((now - *started) / ZOOM_DURATION_MS);
                // Only the scale is interpolated; x and y are derived so the
                // anchor stays pinned to its screen position.
                *vp = anchored_viewport(
                    *anchor_world,
                    *anchor_screen,
                    scale_at(*from_scale, *to_scale, t),
                );
                if t >= 1. {
                    self.anim = None;
                }
                true
            }
            Some(Anim::Fit { from, to, started }) => {
                let t = (now - *started) / FIT_DURATION_MS;
                if t >= 1. {
                    // Land exactly on the target rather than the last eased frame.
                    *vp = *to;
                    self.anim = None;
                } else {
                    *vp = fit_frame(*from, *to, view, t);
                }
                true
            }
            Some(Anim::Glide { v, last }) => {
                let dt = now - *last;
                *last = now;
                // Dividing by scale keeps the glide covering a constant screen
                // distance at any zoom.
                vp.x -= v.vx * dt / vp.scale;
                vp.y -= v.vy * dt / vp.scale;
                v.vx = decay_velocity(v.vx, dt);
                v.vy = decay_velocity(v.vy, dt);
                if !should_glide(*v) {
                    self.anim = None;
                }
                true
            }
        }
    }
}
