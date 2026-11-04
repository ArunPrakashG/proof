//! design/tokens.json as code. Names map one to one to the CSS custom properties.

use std::time::Duration;

use gpui::{point, px, BoxShadow, Hsla, Pixels};
use proof_core::State;

pub const SANS: &str = "Geist";
pub const MONO: &str = "Geist Mono";

pub mod text {
    use gpui::{px, Pixels};
    pub const XS: Pixels = px(11.);
    pub const S: Pixels = px(12.);
    pub const BASE: Pixels = px(13.);
    pub const M: Pixels = px(14.);
    pub const XL: Pixels = px(20.);
}

pub mod space {
    use gpui::{px, Pixels};
    pub const S1: Pixels = px(4.);
    pub const S2: Pixels = px(8.);
    pub const S3: Pixels = px(12.);
    pub const S4: Pixels = px(16.);
    pub const S5: Pixels = px(24.);
    pub const S6: Pixels = px(32.);
    pub const S7: Pixels = px(48.);
    pub const S8: Pixels = px(64.);
}

pub mod radius {
    use gpui::{px, Pixels};
    pub const S: Pixels = px(4.);
    pub const M: Pixels = px(6.);
    pub const L: Pixels = px(10.);
    pub const FULL: Pixels = px(999.);
}

pub mod dur {
    use std::time::Duration;
    #[allow(dead_code)]
    pub const QUICK: Duration = Duration::from_millis(160);
    pub const BASE: Duration = Duration::from_millis(240);
}

/// Between staggered items, such as results settling one after another.
pub const STAGGER: Duration = Duration::from_millis(40);

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,
    pub surface: Hsla,
    pub surface_raised: Hsla,
    pub surface_sunken: Hsla,
    pub surface_hover: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_faint: Hsla,
    pub accent: Hsla,
    pub accent_subtle: Hsla,
    pub accent_text: Hsla,
    pub success: Hsla,
    pub danger: Hsla,
    pub warning: Hsla,
    pub missing: Hsla,
    pub blocked: Hsla,
    pub scrim: Hsla,
}

fn hex(v: u32) -> Hsla {
    gpui::rgb(v).into()
}

fn alpha(v: u32, a: f32) -> Hsla {
    let mut c: Hsla = gpui::rgb(v).into();
    c.a = a;
    c
}

impl Theme {
    pub fn dark() -> Self {
        Theme {
            dark: true,
            surface: hex(0x0f1012),
            surface_raised: hex(0x17181b),
            surface_sunken: hex(0x0a0a0c),
            surface_hover: hex(0x1c1e21),
            border: hex(0x26282c),
            border_strong: hex(0x34373c),
            text: hex(0xececee),
            text_muted: hex(0xa0a4ab),
            text_faint: hex(0x80848c),
            accent: hex(0x8ab4ff),
            accent_subtle: alpha(0x8ab4ff, 0.14),
            accent_text: hex(0x0f1012),
            success: hex(0x4cc38a),
            danger: hex(0xff6a5c),
            warning: hex(0xe5b544),
            missing: hex(0xb49cff),
            blocked: hex(0x5fb8c9),
            scrim: alpha(0x050506, 0.62),
        }
    }

    pub fn light() -> Self {
        Theme {
            dark: false,
            surface: hex(0xfbfbfa),
            surface_raised: hex(0xffffff),
            surface_sunken: hex(0xf3f3f1),
            surface_hover: hex(0xefefec),
            border: hex(0xe4e4e0),
            border_strong: hex(0xd2d2cd),
            text: hex(0x18191b),
            text_muted: hex(0x5b5f66),
            text_faint: hex(0x6c7077),
            accent: hex(0x2f6fe4),
            accent_subtle: alpha(0x2f6fe4, 0.1),
            accent_text: hex(0xffffff),
            success: hex(0x17834f),
            danger: hex(0xc8372b),
            warning: hex(0x95680a),
            missing: hex(0x6d4fd1),
            blocked: hex(0x1f7a8c),
            scrim: alpha(0x18191b, 0.32),
        }
    }

    pub fn state(&self, s: State) -> Hsla {
        match s {
            State::Passing => self.success,
            State::Failing => self.danger,
            State::Stale => self.warning,
            State::Missing => self.missing,
            State::Blocked => self.blocked,
            State::Unverified => self.text_faint,
        }
    }

    /// --shadow-2: dialogs.
    pub fn shadow_2(&self) -> Vec<BoxShadow> {
        let (ring, drop) = if self.dark { (alpha(0xffffff, 0.06), alpha(0x000000, 0.55)) } else { (alpha(0x18191b, 0.08), alpha(0x18191b, 0.18)) };
        vec![
            BoxShadow { color: ring, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.) },
            BoxShadow { color: drop, offset: point(px(0.), px(24.)), blur_radius: px(64.), spread_radius: px(0.) },
        ]
    }
}

/// --ease-out: cubic-bezier(0.2, 0.8, 0.2, 1)
pub fn ease_out(t: f32) -> f32 {
    cubic_bezier(0.2, 0.8, 0.2, 1.0, t)
}

fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let bez = |a: f32, b: f32, t: f32| 3.0 * a * t * (1.0 - t).powi(2) + 3.0 * b * t.powi(2) * (1.0 - t) + t.powi(3);
    // solve x(t) = x by bisection: monotonic for these control points
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if bez(x1, x2, mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bez(y1, y2, (lo + hi) / 2.0)
}

/// The signature motion: a verdict lands, the glyph grows from small into its shape.
/// Returns (scale, opacity) for eased progress `t`.
pub fn settle(t: f32, reduced: bool) -> (f32, f32) {
    if reduced {
        return (1.0, t);
    }
    if t < 0.6 {
        let k = t / 0.6;
        (0.55 + (1.08 - 0.55) * k, k)
    } else {
        let k = (t - 0.6) / 0.4;
        (1.08 + (1.0 - 1.08) * k, 1.0)
    }
}

pub fn scaled(size: Pixels, scale: f32) -> Pixels {
    px(f32::from(size) * scale)
}
