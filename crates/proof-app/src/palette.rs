//! ⌘K: find a capability or run a command. Every item shows the CLI that does the same.

use gpui::{div, prelude::*, px, svg, Context, FontWeight, Hsla, KeyDownEvent, MouseButton, Window};
use proof_core::State;

use crate::app::{Cmd, Mode, PaletteClose, PaletteConfirm, PaletteDown, PaletteUp, ProofApp, Screen};
use crate::theme::{radius, space, text, Theme, MONO};
use crate::ui::{kbd, plural, shortcut};

pub struct Item {
    pub sec: &'static str,
    pub glyph: &'static str,
    /// None: muted
    pub state: Option<State>,
    pub label: String,
    pub hint: String,
    pub cli: String,
    pub cmd: Cmd,
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.to_lowercase())
}

/// Every query word appears somewhere in the item.
fn hit(q: &str, item: &Item) -> bool {
    let hay = format!("{} {} {} {}", item.sec, item.label, item.hint, item.cli).to_lowercase();
    q.split_whitespace().all(|w| hay.contains(w))
}

pub fn items(app: &ProofApp) -> Vec<Item> {
    let Some(p) = &app.palette else { return vec![] };
    let Some(snap) = &app.snap else { return vec![] };
    let q = p.query.trim().to_lowercase();
    let mut out = Vec::new();
    match &p.mode {
        Mode::Commands => {
            // what's on screen first, then everything a query can reach
            let here: Option<String> = match &app.screen {
                Screen::Capability(id) => Some(id.clone()),
                Screen::Map => app.selected.and_then(|i| app.visible_caps().get(i).cloned()),
                _ => None,
            };
            // "run " or "go " alone names a section, not a capability
            let rest = q.split_whitespace().filter(|w| !["run", "go", "show", "open"].contains(w)).count();
            let caps: Vec<_> = if rest == 0 {
                here.iter().filter_map(|id| snap.map.cap(id)).collect()
            } else {
                snap.map.caps.iter().collect()
            };
            for c in &caps {
                let v = &snap.verdicts[&c.id];
                let todo = v.scenarios.len() - v.passing();
                let hint = if todo == 0 { plural(v.scenarios.len(), "scenario", "scenarios") } else { format!("{todo} {}", v.state.word()) };
                out.push(Item {
                    sec: "Run",
                    glyph: v.state.glyph(),
                    state: Some(v.state),
                    label: format!("Run {}", c.name),
                    hint,
                    cli: format!("proof run {}", quote(&c.name)),
                    cmd: Cmd::Run(app.cap_run_target(&c.id)),
                });
            }
            for (gid, name, gcaps) in snap.map.groups() {
                if rest == 0 && !caps.first().is_some_and(|c| c.group_id == gid) {
                    continue;
                }
                let ids: Vec<String> = gcaps.iter().map(|c| c.id.clone()).collect();
                let worst = gcaps.iter().map(|c| snap.state(&c.id)).min().unwrap_or(State::Unverified);
                let mut runners = app.runners_needed(Some(&ids));
                if runners.is_empty() {
                    runners = ids.iter().flat_map(|id| app.runners_of(id)).collect();
                    runners.dedup();
                }
                out.push(Item {
                    sec: "Run",
                    glyph: worst.glyph(),
                    state: Some(worst),
                    label: format!("Run {name}"),
                    hint: plural(gcaps.len(), "capability", "capabilities"),
                    cli: format!("proof run {}", quote(&name)),
                    cmd: Cmd::Run(runners),
                });
            }
            let todo: usize = [State::Stale, State::Failing, State::Unverified].iter().map(|s| snap.total(*s)).sum();
            out.push(Item {
                sec: "Run",
                glyph: "▶",
                state: None,
                label: "Run everything stale".into(),
                hint: plural(todo, "scenario", "scenarios"),
                cli: "proof run --stale".into(),
                cmd: Cmd::RunStale,
            });
            for r in &snap.map.runners {
                out.push(Item {
                    sec: "Run",
                    glyph: "▶",
                    state: None,
                    label: format!("Run the {} runner", r.name),
                    hint: snap.blocked_now.get(&r.name).map(|_| "blocked here".to_string()).unwrap_or_else(|| r.run.clone()),
                    cli: format!("proof run --runner {}", r.name),
                    cmd: Cmd::Run(vec![r.name.clone()]),
                });
            }
            out.push(Item {
                sec: "Evidence",
                glyph: "↓",
                state: None,
                label: "Import a CI log".into(),
                hint: snap.map.runners.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(", "),
                cli: "proof import <runner> <log>".into(),
                cmd: Cmd::Import(None),
            });
            if snap.map.runners.iter().any(|r| r.traced()) {
                out.push(Item {
                    sec: "Evidence",
                    glyph: "↻",
                    state: None,
                    label: "Trace which code each test runs".into(),
                    hint: snap.map.trace_commit.as_ref().map(|c| format!("last at {c}")).unwrap_or_default(),
                    cli: "proof trace".into(),
                    cmd: Cmd::Trace,
                });
            }
            if let Some(c) = here.as_deref().and_then(|id| snap.map.cap(id)).filter(|c| c.inferred) {
                out.push(Item {
                    sec: "Evidence",
                    glyph: "✓",
                    state: None,
                    label: format!("Mark {} reviewed", c.name),
                    hint: String::new(),
                    cli: format!("proof review {}", c.id),
                    cmd: Cmd::Review(c.id.clone()),
                });
            }
            let loose: Vec<String> = snap.affected.incidental.iter().chain(&snap.affected.unmapped).cloned().collect();
            if !loose.is_empty() {
                let n = plural(loose.len(), "unowned file", "unowned files");
                out.push(Item {
                    sec: "Map",
                    glyph: "+",
                    state: None,
                    label: format!("Add {n} to a capability"),
                    hint: String::new(),
                    cli: format!("proof claim <id> {}", loose.join(" ")),
                    cmd: Cmd::Pick(Mode::AddTo(loose.clone())),
                });
                out.push(Item {
                    sec: "Map",
                    glyph: "+",
                    state: None,
                    label: format!("New capability for {n}"),
                    hint: String::new(),
                    cli: format!("proof new \"group › name\" {}", loose.join(" ")),
                    cmd: Cmd::Pick(Mode::NewCap(loose)),
                });
            }
            let go = |label: &str, hint: String, cli: &str, screen: Screen| Item {
                sec: "Go to",
                glyph: "→",
                state: None,
                label: label.into(),
                hint,
                cli: cli.into(),
                cmd: Cmd::Go(screen),
            };
            out.push(go("Map", String::new(), "proof status", Screen::Map));
            out.push(go("Changes", plural(snap.affected.files.len(), "file", "files"), "proof affected", Screen::Changes));
            out.push(go("Runners", plural(snap.map.runners.len(), "runner", "runners"), "proof run --json", Screen::Runners));
            if rest > 0 {
                for c in &snap.map.caps {
                    let s = snap.state(&c.id);
                    out.push(Item {
                        sec: "Go to",
                        glyph: s.glyph(),
                        state: Some(s),
                        label: c.name.clone(),
                        hint: c.group.clone(),
                        cli: format!("proof show {}", c.id),
                        cmd: Cmd::Go(Screen::Capability(c.id.clone())),
                    });
                }
            }
            let light = if app.theme.dark { "light" } else { "dark" };
            out.push(Item {
                sec: "Go to",
                glyph: "◐",
                state: None,
                label: format!("Use the {light} theme"),
                hint: String::new(),
                cli: String::new(),
                cmd: Cmd::ToggleTheme,
            });
        }
        Mode::AddTo(files) => {
            for c in &snap.map.caps {
                let s = snap.state(&c.id);
                out.push(Item {
                    sec: "Add to",
                    glyph: s.glyph(),
                    state: Some(s),
                    label: c.name.clone(),
                    hint: c.group.clone(),
                    cli: format!("proof claim {} {}", c.id, files.join(" ")),
                    cmd: Cmd::Claim(c.id.clone(), files.clone()),
                });
            }
        }
        Mode::NewCap(files) => {
            let title = p.query.trim().to_string();
            let named = proof_core::edit::split_title(&title).is_some();
            out.push(Item {
                sec: "New capability",
                glyph: "+",
                state: None,
                label: if named { format!("Create {title}") } else { "Type “Group › Name”".into() },
                hint: plural(files.len(), "file", "files"),
                cli: format!("proof new \"{}\" {}", if title.is_empty() { "Group › Name" } else { &title }, files.join(" ")),
                cmd: Cmd::NewCap(title, files.clone()),
            });
            return out;
        }
        Mode::ImportFor => {
            for r in &snap.map.runners {
                out.push(Item {
                    sec: "Import a log for",
                    glyph: "↓",
                    state: None,
                    label: r.name.clone(),
                    hint: snap.blocked_now.get(&r.name).map(|_| "blocked here".to_string()).unwrap_or_else(|| r.run.clone()),
                    cli: format!("proof import {} <log>", r.name),
                    cmd: Cmd::Import(Some(r.name.clone())),
                });
            }
        }
    }
    out.retain(|i| q.is_empty() || hit(&q, i));
    out
}

pub fn selected_cli(app: &ProofApp) -> Option<String> {
    let sel = app.palette.as_ref()?.sel;
    items(app).into_iter().nth(sel).map(|i| i.cli).filter(|c| !c.is_empty())
}

impl ProofApp {
    fn palette_len(&self) -> usize {
        items(self).len()
    }

    pub fn palette_up(&mut self, _: &PaletteUp, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.palette.as_mut() {
            p.sel = p.sel.saturating_sub(1);
            cx.notify();
        }
    }

    pub fn palette_down(&mut self, _: &PaletteDown, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.palette_len();
        if let Some(p) = self.palette.as_mut() {
            p.sel = (p.sel + 1).min(n.saturating_sub(1));
            cx.notify();
        }
    }

    pub fn palette_confirm(&mut self, _: &PaletteConfirm, window: &mut Window, cx: &mut Context<Self>) {
        let sel = self.palette.as_ref().map(|p| p.sel).unwrap_or(0);
        if let Some(item) = items(self).into_iter().nth(sel) {
            if let Cmd::NewCap(title, _) = &item.cmd {
                if proof_core::edit::split_title(title).is_none() {
                    return;
                }
            }
            self.run_cmd(item.cmd, window, cx);
        }
    }

    pub fn palette_close(&mut self, _: &PaletteClose, window: &mut Window, cx: &mut Context<Self>) {
        self.close_palette(window, cx);
    }

    fn palette_key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.palette.as_mut() else { return };
        let m = &e.keystroke.modifiers;
        if m.control || m.platform || m.alt {
            return;
        }
        match e.keystroke.key.as_str() {
            "backspace" => {
                p.query.pop();
            }
            _ => match &e.keystroke.key_char {
                Some(ch) if !ch.chars().any(char::is_control) => p.query.push_str(ch),
                _ => return,
            },
        }
        p.sel = 0;
        cx.stop_propagation();
        cx.notify();
    }

    pub fn render_palette(&mut self, th: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette.as_ref().unwrap();
        let (query, sel) = (p.query.clone(), p.sel);
        let placeholder = match &p.mode {
            Mode::Commands => "Find a capability or run a command".to_string(),
            Mode::AddTo(files) => format!("Add {} to which capability?", plural(files.len(), "file", "files")),
            Mode::NewCap(_) => "Group › Name".to_string(),
            Mode::ImportFor => "Which runner is the log from?".to_string(),
        };
        let list = items(self);
        let sel = sel.min(list.len().saturating_sub(1));
        let empty = list.is_empty();
        let mut sections: Vec<(&'static str, Vec<(usize, Item)>)> = Vec::new();
        for (i, item) in list.into_iter().enumerate().take(60) {
            match sections.iter_mut().find(|s| s.0 == item.sec) {
                Some(s) => s.1.push((i, item)),
                None => sections.push((item.sec, vec![(i, item)])),
            }
        }
        let th = *th;
        let caret = || div().w(px(1.5)).h(px(16.)).bg(th.accent);

        div()
            .id("scrim")
            .absolute()
            .inset_0()
            .bg(th.scrim)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_palette(window, cx)))
            .flex()
            .justify_center()
            .items_start()
            .pt(px(96.))
            .px(space::S4)
            .child(
                div()
                    .id("palette")
                    .key_context("Palette")
                    .track_focus(&self.palette_focus)
                    .on_action(cx.listener(Self::palette_up))
                    .on_action(cx.listener(Self::palette_down))
                    .on_action(cx.listener(Self::palette_confirm))
                    .on_action(cx.listener(Self::palette_close))
                    .on_key_down(cx.listener(Self::palette_key))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .w(px(640.))
                    .max_w_full()
                    .flex()
                    .flex_col()
                    .rounded(radius::L)
                    .bg(th.surface_raised)
                    .border_1()
                    .border_color(th.border)
                    .shadow(th.shadow_2())
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .h(px(52.))
                            .px(space::S4)
                            .border_b_1()
                            .border_color(th.border)
                            .child(svg().path("icons/search.svg").size(px(16.)).flex_shrink_0().text_color(th.text_faint))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .items_center()
                                    .text_size(text::M)
                                    .when(query.is_empty(), |d| d.child(caret()).child(div().text_color(th.text_faint).child(placeholder)))
                                    .when(!query.is_empty(), |d| d.child(div().whitespace_nowrap().child(query.clone())).child(caret())),
                            )
                            .child(kbd("esc", th.text_faint).px(px(6.)).py(px(2.)).rounded(radius::S).border_1().border_color(th.border_strong)),
                    )
                    .child(
                        div()
                            .id("results")
                            .flex()
                            .flex_col()
                            .p(px(6.))
                            .max_h(px(420.))
                            .overflow_y_scroll()
                            .children(sections.into_iter().map(|(title, rows)| {
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(div().pt(px(10.)).pb(space::S1).px(px(10.)).text_size(text::XS).text_color(th.text_faint).child(title))
                                    .children(rows.into_iter().map(|(i, it)| palette_row(i, i == sel, it, &th, cx)))
                            }))
                            .when(empty, |d| {
                                d.child(
                                    div()
                                        .py(space::S5)
                                        .px(px(10.))
                                        .flex()
                                        .justify_center()
                                        .text_color(th.text_muted)
                                        .child("Nothing matches. Try a capability, group or runner name."),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(space::S4)
                            .py(px(10.))
                            .px(space::S4)
                            .border_t_1()
                            .border_color(th.border)
                            .text_size(text::XS)
                            .text_color(th.text_faint)
                            .child(hint_key("↑↓", "select"))
                            .child(hint_key("↵", "run"))
                            .child(hint_key(&shortcut(&["mod", "shift", "c"]), "copy command")),
                    ),
            )
    }
}

fn hint_key(k: &str, label: &'static str) -> impl IntoElement {
    div().flex().gap(px(6.)).child(div().font_family(MONO).child(k.to_string())).child(label)
}

fn palette_row(i: usize, selected: bool, it: Item, th: &Theme, cx: &mut Context<ProofApp>) -> impl IntoElement {
    let color: Hsla = it.state.map(|s| th.state(s)).unwrap_or(th.text_muted);
    let hover = th.surface_hover;
    div()
        .id(("palette-item", i))
        .flex()
        .items_center()
        .gap(space::S3)
        .min_h(px(40.))
        .py(px(6.))
        .px(px(10.))
        .rounded(radius::M)
        .cursor_pointer()
        .when(selected, |d| d.bg(th.accent_subtle))
        .when(!selected, |d| d.hover(move |s| s.bg(hover)))
        .on_click(cx.listener(move |this, _, window, cx| {
            if let Some(p) = this.palette.as_mut() {
                p.sel = i;
            }
            this.palette_confirm(&PaletteConfirm, window, cx);
        }))
        .child(
            div()
                .w(px(20.))
                .flex_shrink_0()
                .flex()
                .justify_center()
                .font_family(MONO)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(it.glyph),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_baseline()
                .gap(space::S2)
                .overflow_hidden()
                .child(div().whitespace_nowrap().child(it.label))
                .child(div().whitespace_nowrap().text_size(text::S).text_color(th.text_faint).child(it.hint)),
        )
        .child(
            div()
                .flex_shrink_0()
                .max_w(px(240.))
                .truncate()
                .font_family(MONO)
                .text_size(text::XS)
                .text_color(th.text_muted)
                .child(it.cli),
        )
}
