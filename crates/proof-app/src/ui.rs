//! The components the screens are built from (Foundations › Components).

use std::time::Duration;

use gpui::{
    div, prelude::*, px, svg, Animation, AnimationExt, Div, ElementId, FontWeight, Hsla, Pixels, SharedString, Stateful,
};
use proof_core::State;

use crate::theme::{self, dur, radius, space, text, Theme, MONO, STAGGER};

/// ⌘K on macOS, Ctrl+K elsewhere.
pub fn shortcut(keys: &[&str]) -> String {
    let mac = cfg!(target_os = "macos");
    let parts: Vec<String> = keys
        .iter()
        .map(|k| match (*k, mac) {
            ("mod", true) => "⌘".into(),
            ("mod", false) => "Ctrl".into(),
            ("shift", true) => "⇧".into(),
            ("shift", false) => "Shift".into(),
            (k, _) => k.to_uppercase(),
        })
        .collect();
    if mac {
        parts.concat()
    } else {
        parts.join("+")
    }
}

pub fn mono(t: impl Into<SharedString>, size: Pixels, color: Hsla) -> Div {
    div().font_family(MONO).text_size(size).text_color(color).child(t.into())
}

pub fn kbd(t: impl Into<SharedString>, color: Hsla) -> Div {
    mono(t, text::XS, color).flex_shrink_0()
}

/// A status glyph in its fixed 20px column.
pub fn glyph(state: State, th: &Theme) -> Div {
    glyph_text(state.glyph(), th.state(state))
}

pub fn glyph_text(g: &'static str, color: Hsla) -> Div {
    div()
        .w(px(20.))
        .flex_shrink_0()
        .flex()
        .justify_center()
        .font_family(MONO)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(g)
}

/// A glyph that settles into place the first time it renders under `id`,
/// `order` places it in a stagger after the ones before it.
pub fn settle(id: impl Into<ElementId>, g: &'static str, color: Hsla, size: Pixels, order: usize, reduced: bool) -> impl IntoElement {
    let delay = STAGGER.as_secs_f32() * order.min(8) as f32;
    let base = dur::BASE.as_secs_f32();
    let total = base + delay;
    div()
        .w(px(20.).max(size))
        .h(size * 1.45)
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .font_family(MONO)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .text_size(size)
                .line_height(size * 1.45)
                .child(g)
                .with_animation(id, Animation::new(Duration::from_secs_f32(total)), move |el, t| {
                    let local = ((t * total - delay) / base).clamp(0.0, 1.0);
                    let (scale, opacity) = theme::settle(theme::ease_out(local), reduced);
                    el.text_size(theme::scaled(size, scale)).opacity(opacity)
                }),
        )
}

pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, th: &Theme) -> Stateful<Div> {
    let hover = th.surface_hover;
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(space::S2)
        .h(px(32.))
        .px(space::S3)
        .rounded(radius::M)
        .border_1()
        .border_color(th.border)
        .text_color(th.text)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(label.into())
}

pub fn primary(id: impl Into<ElementId>, label: impl Into<SharedString>, keys: Option<String>, th: &Theme) -> Stateful<Div> {
    let fg = th.accent_text;
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(space::S2)
        .h(px(32.))
        .px(space::S3)
        .rounded(radius::M)
        .bg(th.accent)
        .text_color(fg)
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(|s| s.opacity(0.92))
        .child(label.into())
        .children(keys.map(|k| kbd(k, fg).opacity(0.75)))
}

pub fn icon_button(id: impl Into<ElementId>, icon: &'static str, th: &Theme) -> Stateful<Div> {
    let hover = th.surface_hover;
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(px(28.))
        .rounded(radius::M)
        .text_color(th.text_muted)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(svg().path(icon).size(px(14.)).text_color(th.text_muted))
}

pub fn inferred_tag(th: &Theme) -> Div {
    div()
        .flex_shrink_0()
        .px(px(6.))
        .rounded(radius::FULL)
        .border_1()
        .border_dashed()
        .border_color(th.border_strong)
        .text_size(text::XS)
        .text_color(th.text_muted)
        .child("inferred")
}

/// A section's heading row: title on the left, a mono summary on the right, a rule below.
pub fn section_head(title: impl Into<SharedString>, summary: impl Into<SharedString>, th: &Theme) -> Div {
    div()
        .flex()
        .items_end()
        .gap(space::S3)
        .px(px(10.))
        .pb(space::S2)
        .border_b_1()
        .border_color(th.border)
        .child(div().flex_1().text_size(text::M).font_weight(FontWeight::MEDIUM).child(title.into()))
        .child(mono(summary, text::XS, th.text_faint))
}

pub fn path_chip(path: impl Into<SharedString>, th: &Theme) -> Div {
    mono(path, text::S, th.text_muted)
        .px(px(6.))
        .py(px(2.))
        .rounded(radius::S)
        .bg(th.surface_sunken)
        .border_1()
        .border_color(th.border)
}

/// Sunken box of changed files with their line counts.
pub fn diff_box(rows: Vec<(String, u32, u32)>, th: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space::S1)
        .px(space::S3)
        .py(px(10.))
        .rounded(radius::M)
        .bg(th.surface_sunken)
        .border_1()
        .border_color(th.border)
        .font_family(MONO)
        .text_size(text::S)
        .overflow_hidden()
        .children(rows.into_iter().map(|(path, add, del)| {
            div()
                .flex()
                .gap(space::S4)
                .child(div().flex_1().min_w_0().truncate().text_color(th.text).child(path))
                .child(div().w(px(44.)).flex().justify_end().text_color(th.success).child(format!("+{add}")))
                .child(div().w(px(40.)).flex().justify_end().text_color(th.danger).child(format!("−{del}")))
        }))
}

/// A card for notices: the empty and error states.
pub fn card(th: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space::S3)
        .p(space::S5)
        .rounded(radius::L)
        .border_1()
        .border_color(th.border)
        .bg(th.surface_raised)
}

pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `Oct 12`, `today`, `yesterday` from an RFC 3339 time.
pub fn day(at: &str) -> String {
    let Ok(t) = time::OffsetDateTime::parse(at, &time::format_description::well_known::Rfc3339) else {
        return at.chars().take(10).collect();
    };
    let local = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let (d, today) = (t.to_offset(local).date(), time::OffsetDateTime::now_utc().to_offset(local).date());
    if d == today {
        "today".into()
    } else if today.previous_day() == Some(d) {
        "yesterday".into()
    } else {
        let m = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][d.month() as usize - 1];
        if d.year() == today.year() {
            format!("{m} {}", d.day())
        } else {
            format!("{m} {} {}", d.day(), d.year())
        }
    }
}
