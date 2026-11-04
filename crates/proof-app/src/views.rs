//! The screens, built to design/canvas: Map, Capability, Changes, Runners, and the states between.

use std::time::Duration;

use gpui::{
    div, percentage, prelude::*, px, svg, Animation, AnimationExt, AnyElement, Context, Div, FontWeight,
    Render, Stateful, Transformation, Window,
};
use proof_core::{file_name, source_label, MapError, State};

use crate::app::*;
use crate::snapshot::{dir_name, Snapshot};
use crate::theme::{radius, space, text, Theme, MONO, SANS};
use crate::ui::*;

impl Render for ProofApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let th = self.theme;
        let wide = window.viewport_size().width >= px(760.);
        let body: Vec<AnyElement> = match self.snap.clone() {
            None if self.loading && self.error.is_none() => {
                let slow = self.opened.elapsed() >= SLOW_LOAD;
                vec![div().when(slow, |d| d.text_color(th.text_faint).child("Reading the map…")).into_any_element()]
            }
            None => vec![self.no_map(&th, cx)],
            Some(snap) => {
                let mut out = Vec::new();
                if let Some(e) = &self.error {
                    out.push(self.map_error(e, true, &th, cx));
                }
                out.extend(match self.screen.clone() {
                    Screen::Map => self.map_screen(&snap, &th, cx),
                    Screen::Capability(id) => self.cap_screen(&snap, &id, &th, cx),
                    Screen::Changes => self.changes_screen(&snap, &th, cx),
                    Screen::Runners => self.runners_screen(&snap, &th, cx),
                    Screen::Unclaimed => self.unclaimed_screen(&snap, &th, cx),
                });
                out
            }
        };
        let gap = if self.screen == Screen::Map { space::S6 } else { px(40.) };

        div()
            .id("proof")
            .key_context("ProofApp")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::open_palette))
            .on_action(cx.listener(Self::go_map))
            .on_action(cx.listener(Self::go_changes))
            .on_action(cx.listener(Self::go_runners))
            .on_action(cx.listener(Self::go_unclaimed))
            .on_action(cx.listener(Self::run_primary))
            .on_action(cx.listener(Self::run_all))
            .on_action(cx.listener(Self::import_log))
            .on_action(cx.listener(Self::copy_command))
            .on_action(cx.listener(Self::stop_run))
            .on_action(cx.listener(Self::next))
            .on_action(cx.listener(Self::prev))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::back))
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(|this, _: &PageDown, _, cx| this.scroll_page(1.0, cx)))
            .on_action(cx.listener(|this, _: &PageUp, _, cx| this.scroll_page(-1.0, cx)))
            .on_action(cx.listener(|this, _: &Top, _, cx| this.scroll_page(-1000.0, cx)))
            .on_action(cx.listener(|this, _: &Bottom, _, cx| this.scroll_page(1000.0, cx)))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(th.surface)
            .text_color(th.text)
            .font_family(SANS)
            .text_size(text::BASE)
            .child(self.header(&th, cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(wide && self.snap.is_some(), |d| d.child(self.nav(&th, cx)))
                    .child(
                        div()
                            .id("main")
                            .flex_1()
                            .min_w_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(gap)
                            .pt(if wide { px(40.) } else { px(20.) })
                            .px(if wide { space::S7 } else { space::S4 })
                            .pb(space::S8)
                            .children(body.into_iter().map(|el| div().w_full().max_w(px(960.)).flex_shrink_0().child(el))),
                    ),
            )
            .children(self.toast.as_ref().map(|(msg, _)| {
                div().absolute().bottom(space::S5).left_0().right_0().flex().justify_center().child(
                    div()
                        .max_w(px(640.))
                        .px(space::S3)
                        .py(space::S2)
                        .rounded(radius::M)
                        .bg(th.surface_raised)
                        .border_1()
                        .border_color(th.border)
                        .shadow(th.shadow_2())
                        .text_size(text::S)
                        .child(msg.clone()),
                )
            }))
            .when(self.palette.is_some(), |d| d.child(self.render_palette(&th, cx)))
    }
}

impl ProofApp {
    fn header(&self, th: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let (name, at) = match &self.snap {
            Some(s) => (s.map.name.clone(), format!("{} @ {}", s.head.branch.as_deref().unwrap_or("detached"), s.head.commit_label())),
            None if self.loading => (String::new(), String::new()),
            None => (dir_name(self.map_path.parent().unwrap_or(&self.map_path)), String::new()),
        };
        let hover = th.surface_hover;
        div()
            .flex()
            .flex_wrap()
            .flex_shrink_0()
            .items_center()
            .gap(space::S3)
            .min_h(px(48.))
            .py(px(6.))
            .px(space::S4)
            .border_b_1()
            .border_color(th.border)
            .child(
                div()
                    .relative()
                    .size(px(20.))
                    .flex_shrink_0()
                    .child(svg().absolute().size(px(20.)).path("icons/mark-frame.svg").text_color(th.text))
                    .child(svg().absolute().size(px(20.)).path("icons/mark-check.svg").text_color(th.success)),
            )
            .child(div().font_weight(FontWeight::MEDIUM).whitespace_nowrap().child(name))
            .child(mono(at, text::S, th.text_faint).whitespace_nowrap())
            .child(div().flex_1())
            .child(
                div()
                    .id("find")
                    .flex()
                    .items_center()
                    .gap(space::S2)
                    .h(px(32.))
                    .px(px(10.))
                    .flex_basis(px(300.))
                    .flex_shrink()
                    .min_w(px(180.))
                    .rounded(radius::M)
                    .border_1()
                    .border_color(th.border)
                    .bg(th.surface_sunken)
                    .text_color(th.text_faint)
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(|this, _, window, cx| this.open_palette(&OpenPalette, window, cx)))
                    .child(svg().path("icons/search.svg").size(px(14.)).text_color(th.text_faint))
                    .child(div().flex_1().child("Find or run"))
                    .child(kbd(shortcut(&["mod", "k"]), th.text_faint)),
            )
    }

    fn nav(&self, th: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let snap = self.snap.clone().unwrap();
        let th = *th;
        let hover = th.surface_hover;
        let link = |id: &'static str, label: &'static str, count: usize, active: bool, screen: Screen, cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(32.))
                .px(px(10.))
                .rounded(radius::M)
                .cursor_pointer()
                .text_color(if active { th.text } else { th.text_muted })
                .when(active, |d| d.bg(th.surface_hover))
                .when(!active, |d| d.hover(move |s| s.bg(hover)))
                .on_click(cx.listener(move |this, _, _, cx| this.go(screen.clone(), cx)))
                .child(div().flex_1().child(label))
                .child(mono(count.to_string(), text::XS, th.text_faint))
        };
        let on_cap = matches!(self.screen, Screen::Capability(_));
        let links = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(link("nav-map", "Map", snap.map.caps.len(), self.screen == Screen::Map || on_cap, Screen::Map, cx))
            .child(link("nav-changes", "Changes", snap.affected.files.len(), self.screen == Screen::Changes, Screen::Changes, cx))
            .child(link("nav-runners", "Runners", snap.map.runners.len(), self.screen == Screen::Runners, Screen::Runners, cx))
            .child(link("nav-unclaimed", "Unclaimed", snap.unclaimed.len(), self.screen == Screen::Unclaimed, Screen::Unclaimed, cx));

        let heading = |t: String| div().px(px(10.)).pb(space::S1).text_size(text::XS).text_color(th.text_faint).child(t);
        let below: Option<Div> = match &self.screen {
            Screen::Map => {
                let visible = self.visible_caps();
                let groups: Vec<_> = snap.map.groups().into_iter().filter(|(_, _, caps)| caps.iter().any(|c| visible.contains(&c.id))).collect();
                Some(div().flex().flex_col().gap(px(2.)).child(heading("Groups".into())).children(groups.into_iter().enumerate().map(
                    |(i, (gid, name, caps))| {
                        let worst = caps.iter().map(|c| snap.state(&c.id)).min().unwrap_or(State::Unverified);
                        div()
                            .id(gpui::SharedString::from(format!("nav-group-{gid}")))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .h(px(32.))
                            .px(px(10.))
                            .rounded(radius::M)
                            .text_color(th.text_muted)
                            .cursor_pointer()
                            .hover(move |s| s.bg(hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.scroll.scroll_to_item(i + 1);
                                cx.notify();
                            }))
                            .child(glyph(worst, &th))
                            .child(div().flex_1().min_w_0().truncate().child(name))
                            .child(mono(caps.len().to_string(), text::XS, th.text_faint))
                    },
                )))
            }
            Screen::Capability(id) => snap.map.cap(id).map(|cur| {
                let group: Vec<_> = snap.map.caps.iter().filter(|c| c.group_id == cur.group_id).collect();
                div().flex().flex_col().gap(px(2.)).child(heading(cur.group.clone())).children(group.into_iter().map(|c| {
                    let here = c.id == cur.id;
                    let id = c.id.clone();
                    div()
                        .id(gpui::SharedString::from(format!("nav-cap-{}", c.id)))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .min_h(px(32.))
                        .px(px(10.))
                        .rounded(radius::M)
                        .cursor_pointer()
                        .text_color(if here { th.text } else { th.text_muted })
                        .when(here, |d| d.bg(th.accent_subtle))
                        .when(!here, |d| d.hover(move |s| s.bg(hover)))
                        .on_click(cx.listener(move |this, _, _, cx| this.go(Screen::Capability(id.clone()), cx)))
                        .child(glyph(snap.state(&c.id), &th))
                        .child(div().flex_1().min_w_0().truncate().child(c.name.clone()))
                }))
            }),
            Screen::Changes => {
                let chosen = snap.affected.since.clone();
                Some(div().flex().flex_col().gap(px(2.)).child(heading("Since".into())).children(snap.since_choices.iter().enumerate().map(|(i, s)| {
                    let here = s.rev == chosen;
                    let rev = s.rev.clone();
                    div()
                        .id(("nav-since", i))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .h(px(32.))
                        .px(px(10.))
                        .rounded(radius::M)
                        .cursor_pointer()
                        .when(here, |d| d.bg(th.accent_subtle))
                        .when(!here, |d| d.hover(move |st| st.bg(hover)))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_since(rev.clone(), cx)))
                        .child(
                            div()
                                .flex_1()
                                .when(s.rev.is_some(), |d| d.font_family(MONO).text_size(text::S))
                                .text_color(if here { th.text } else { th.text_muted })
                                .child(s.label.clone()),
                        )
                        .child(div().text_size(text::XS).text_color(th.text_faint).child(s.note.clone()))
                })))
            }
            Screen::Runners | Screen::Unclaimed => None,
        };
        div()
            .id("nav")
            .w(px(220.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(space::S5)
            .py(space::S4)
            .px(space::S3)
            .border_r_1()
            .border_color(th.border)
            .overflow_y_scroll()
            .child(links)
            .children(below)
    }

    // ------------------------------------------------------------ Map

    fn map_screen(&self, snap: &Snapshot, th: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let th = *th;
        let todo: usize = [State::Stale, State::Failing, State::Unverified].iter().map(|s| snap.total(*s)).sum();
        let run_label = if todo > 0 { format!("Run {todo} stale") } else { "Run all".into() };
        let all = snap.map.caps.len();
        let inferred = snap.map.caps.iter().filter(|c| c.inferred).count();
        let note = match inferred {
            0 => None,
            n if n == all => Some(format!("All {all} mapped by an agent, none reviewed")),
            n => Some(format!("{n} of {all} mapped by an agent, not reviewed yet")),
        };

        let mut filters: Vec<(Option<State>, &'static str, &'static str, usize)> = vec![(None, "", "All", all)];
        for s in [State::Failing, State::Stale, State::Missing, State::Blocked, State::Unverified, State::Passing] {
            let n = snap.caps_in(s);
            if n > 0 {
                let word = match s {
                    State::Failing => "Failing",
                    State::Stale => "Stale",
                    State::Missing => "Missing",
                    State::Blocked => "Blocked",
                    State::Unverified => "Unverified",
                    State::Passing => "Passing",
                };
                filters.push((Some(s), s.glyph(), word, n));
            }
        }
        let current = self.filter;
        let segmented = div()
            .flex()
            .p(px(2.))
            .gap(px(2.))
            .rounded(radius::M)
            .bg(th.surface_sunken)
            .border_1()
            .border_color(th.border)
            .children(filters.into_iter().enumerate().map(|(i, (f, g, label, n))| {
                let on = f == current;
                div()
                    .id(("filter", i))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(26.))
                    .px(px(10.))
                    .rounded(radius::S)
                    .text_size(text::S)
                    .cursor_pointer()
                    .text_color(if on { th.text } else { th.text_muted })
                    .when(on, |d| d.bg(th.surface_hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.filter = f;
                        this.selected = None;
                        cx.notify();
                    }))
                    .when(!g.is_empty(), |d| {
                        d.child(div().font_family(MONO).font_weight(FontWeight::SEMIBOLD).text_color(f.map(|s| th.state(s)).unwrap_or(th.text)).child(g))
                    })
                    .child(label)
                    .child(div().font_family(MONO).text_color(th.text_faint).child(n.to_string()))
            }));

        let head = div()
            .flex()
            .flex_col()
            .gap(space::S4)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::S4)
                    .child(h1("Capabilities"))
                    .child(primary("run-stale", run_label, Some(shortcut(&["mod", "shift", "r"])), &th).on_click(cx.listener(|this, _, w, cx| this.run_all(&RunAll, w, cx)))),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(space::S4)
                    .child(segmented)
                    .children(note.map(|n| {
                        div().flex().items_center().gap(px(6.)).text_size(text::S).text_color(th.text_faint).child(inferred_tag(&th)).child(n)
                    })),
            );

        let visible = self.visible_caps();
        let mut ix = 0usize;
        let mut out = vec![head.into_any_element()];
        for (gid, name, caps) in snap.map.groups() {
            let shown: Vec<_> = caps.iter().filter(|c| visible.contains(&c.id)).collect();
            if shown.is_empty() {
                continue;
            }
            let summary = State::ALL
                .iter()
                .filter_map(|s| {
                    let n = caps.iter().filter(|c| snap.state(&c.id) == *s).count();
                    (n > 0).then(|| format!("{n} {}", s.word()))
                })
                .collect::<Vec<_>>()
                .join(" · ");
            let mut rows = Vec::new();
            for c in shown {
                let v = &snap.verdicts[&c.id];
                let sub = match v.state {
                    State::Passing => String::new(),
                    State::Missing => "evidence not found".into(),
                    State::Stale => {
                        let changed: Vec<&str> = v.changed().into_iter().map(file_name).collect();
                        let more = changed.len().saturating_sub(3);
                        let mut s = format!("changed {}", changed[..changed.len().min(3)].join(", "));
                        if more > 0 {
                            s.push_str(&format!(" +{more}"));
                        }
                        s
                    }
                    State::Failing => {
                        let at = v.scenarios.iter().find(|s| s.state == State::Failing).and_then(|s| s.entry.as_ref()).map(|e| e.commit.clone());
                        format!("failed at {}", at.unwrap_or_default())
                    }
                    State::Blocked => v.blocked_why().unwrap_or("runner blocked").to_string(),
                    State::Unverified => "never run".into(),
                };
                let selected = self.selected == Some(ix);
                let id = c.id.clone();
                rows.push(
                    list_row(gpui::SharedString::from(format!("cap-{}", c.id)), selected, &th)
                        .min_h(px(44.))
                        .py(px(6.))
                        .on_click(cx.listener(move |this, _, _, cx| this.go(Screen::Capability(id.clone()), cx)))
                        .child(glyph(v.state, &th))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .items_baseline()
                                .gap(space::S3)
                                .overflow_hidden()
                                .child(div().flex_shrink_0().child(c.name.clone()))
                                .child(mono(sub, text::XS, th.text_faint).min_w_0().truncate()),
                        )
                        .child(mono(format!("{}/{}", v.passing(), v.scenarios.len()), text::S, th.text_muted).w(px(48.)).flex().justify_end()),
                );
                ix += 1;
            }
            out.push(
                div()
                    .id(gpui::SharedString::from(format!("group-{gid}")))
                    .flex()
                    .flex_col()
                    .child(section_head(name, summary, &th))
                    .children(rows)
                    .into_any_element(),
            );
        }
        if visible.is_empty() {
            out.push(div().px(px(10.)).text_color(th.text_muted).child("No capabilities here.").into_any_element());
        }
        out
    }

    // ------------------------------------------------------------ Capability

    fn cap_screen(&self, snap: &Snapshot, id: &str, th: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let th = *th;
        let Some(cap) = snap.map.cap(id) else {
            return vec![div().text_color(th.text_muted).child(format!("No capability {id} in the map.")).into_any_element()];
        };
        let v = &snap.verdicts[id];
        let n = v.scenarios.len();
        let todo = n - v.passing();
        let sentence = match v.state {
            State::Stale => format!("Stale. {} passed on code that has changed since.", plural(v.count(State::Stale), "scenario", "scenarios")),
            State::Passing => format!("Passing. All {} passed on the code as it is.", plural(n, "scenario", "scenarios")),
            State::Missing => format!("Missing. {} no evidence: their tests didn’t show up in the last run.", plural(v.count(State::Missing), "scenario has", "scenarios have")),
            State::Failing => format!("Failing. {} failed.", plural(v.count(State::Failing), "scenario", "scenarios")),
            State::Blocked => format!("Blocked. {}", v.blocked_why().unwrap_or("Its runner can’t run on this machine.")),
            State::Unverified => "Unverified. Nothing has run yet.".into(),
        };
        let run_label = if todo > 0 { format!("Run {todo} {}", v.state.word()) } else { "Run again".into() };
        let cap_id = cap.id.clone();
        let cmd = format!("proof show {}", cap.id);
        let reduced = self.reduced;

        let head = div()
            .flex()
            .flex_col()
            .gap(space::S3)
            .child(
                div()
                    .id("crumb")
                    .text_size(text::S)
                    .text_color(th.text_faint)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.go(Screen::Map, cx)))
                    .child(format!("{} ›", cap.group)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(space::S3)
                    .child(settle(gpui::SharedString::from(format!("settle-{id}-{:?}", v.state)), v.state.glyph(), th.state(v.state), text::XL, 0, reduced))
                    .child(h1(cap.name.clone()).flex_initial())
                    .when(cap.inferred, |d| d.child(inferred_tag(&th)))
                    .child(div().flex_1())
                    .when(cap.inferred, |d| {
                        let id = cap_id.clone();
                        d.child(button("review", "Mark reviewed", &th).on_click(cx.listener(move |this, _, w, cx| this.run_cmd(Cmd::Review(id.clone()), w, cx))))
                    })
                    .child(primary("run-cap", run_label, Some(shortcut(&["mod", "r"])), &th).on_click(cx.listener(|this, _, w, cx| this.run_primary(&RunPrimary, w, cx)))),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(space::S3)
                    .text_color(th.text_muted)
                    .child(div().min_w_0().child(sentence))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(space::S1)
                            .child(mono(cap.id.clone(), text::S, th.text_faint))
                            .child(icon_button("copy-cli", "icons/terminal.svg", &th).on_click(cx.listener(move |this, _, _, cx| this.copy(cmd.clone(), cx)))),
                    ),
            );

        // scenarios
        let summary = State::ALL
            .iter()
            .filter_map(|s| (v.count(*s) > 0).then(|| format!("{} {}", v.count(*s), s.word())))
            .collect::<Vec<_>>()
            .join(" · ");
        let mut rows = Vec::new();
        for (i, (s, sv)) in cap.scenarios.iter().zip(&v.scenarios).enumerate() {
            let open = self.open_scenario.as_deref() == Some(s.id.as_str());
            let at = match &sv.entry {
                Some(e) => format!("{} · {} · {}", e.commit, source_label(&e.source), day(&e.at)),
                None => "no evidence".into(),
            };
            let sid = s.id.clone();
            let row = list_row(gpui::SharedString::from(format!("scn-{}", s.id)), self.selected == Some(i), &th)
                .min_h(px(52.))
                .py(px(8.))
                .when(open && self.selected != Some(i), |d| d.bg(th.surface_raised))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected = Some(i);
                    this.toggle_scenario(sid.clone(), cx)
                }))
                .child(settle(gpui::SharedString::from(format!("settle-{}-{:?}", s.id, sv.state)), sv.state.glyph(), th.state(sv.state), text::BASE, i, reduced))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child({
                            // one text, so the state wraps along with the name
                            let text = format!("{}, {}", s.name, sv.state.word());
                            let faint = gpui::HighlightStyle { color: Some(th.text_faint), ..Default::default() };
                            gpui::StyledText::new(text.clone()).with_highlights([(s.name.len()..text.len(), faint)])
                        })
                        .child(
                            div()
                                .flex()
                                .gap(space::S2)
                                .text_size(text::XS)
                                .text_color(th.text_faint)
                                .overflow_hidden()
                                .child(div().font_family(MONO).child(s.runner.clone()))
                                .child(div().truncate().child(format!("“{}”", s.matcher.trim_start_matches('=')))),
                        ),
                )
                .child(mono(at, text::XS, th.text_faint).whitespace_nowrap().flex_none());
            let mut block = div().flex().flex_col().border_b_1().border_color(th.border).child(row.border_b_0());
            if open {
                block = block.bg(th.surface_raised).child(self.scenario_detail(snap, s, sv, &th));
            }
            rows.push(block);
        }
        let scenarios = div().flex().flex_col().child(section_head("Scenarios", summary, &th)).children(rows);

        // code it claims
        let traced = cap.traced.len();
        let narrow = snap.map.narrow_traced(cap);
        let mut claims = div()
            .flex()
            .flex_col()
            .gap(space::S3)
            .child(section_head("Code it claims", format!("{} named · {traced} traced", cap.code.len()), &th))
            .child(div().flex().flex_wrap().gap(space::S2).px(px(10.)).children(cap.code.iter().map(|p| path_chip(p.clone(), &th))));
        if traced > 0 {
            let show = self.show_traced;
            claims = claims.child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::S4)
                    .px(px(10.))
                    .child(
                        div().flex_1().min_w_0().text_color(th.text_muted).child(format!(
                            "Its tests also run {traced} files. {narrow} of them few other tests touch, so a change there hits this capability directly. The other {} are shared.",
                            traced - narrow
                        )),
                    )
                    .child(
                        button("show-traced", if show { "Hide files".to_string() } else { format!("Show {traced} files") }, &th).on_click(cx.listener(|this, _, _, cx| {
                            this.show_traced = !this.show_traced;
                            cx.notify();
                        })),
                    ),
            );
            if show {
                let mut files: Vec<(&String, f64)> = cap.traced.iter().map(|f| (f, snap.map.reach(f))).collect();
                files.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(b.0)));
                claims = claims.child(
                    div()
                        .flex()
                        .flex_col()
                        .px(space::S3)
                        .py(px(10.))
                        .gap(px(2.))
                        .rounded(radius::M)
                        .bg(th.surface_sunken)
                        .border_1()
                        .border_color(th.border)
                        .font_family(MONO)
                        .text_size(text::S)
                        .children(files.into_iter().map(|(f, reach)| {
                            let own = reach <= snap.map.narrow;
                            div()
                                .flex()
                                .gap(space::S4)
                                .child(div().flex_1().min_w_0().truncate().text_color(if own { th.text } else { th.text_muted }).child(f.clone()))
                                .child(div().text_color(th.text_faint).child(format!("{:.0}%", reach * 100.)))
                        })),
                );
            }
        } else if snap.map.runners.iter().any(|r| r.traced()) {
            claims = claims.child(div().px(px(10.)).text_color(th.text_muted).child("Not traced yet. Trace to see which files its tests run."));
        }
        vec![head.into_any_element(), scenarios.into_any_element(), claims.into_any_element()]
    }

    fn scenario_detail(&self, snap: &Snapshot, s: &proof_core::Scenario, sv: &proof_core::assess::ScenarioVerdict, th: &Theme) -> Div {
        let d = div().flex().flex_col().gap(space::S2).pt(px(2.)).pb(space::S4).pl(px(42.)).pr(px(10.)).text_size(text::S).text_color(th.text_muted);
        let commit = sv.entry.as_ref().map(|e| e.commit.trim_end_matches("+dirty").to_string()).unwrap_or_default();
        match sv.state {
            State::Stale => {
                let rows: Vec<(String, u32, u32)> = sv
                    .changed
                    .iter()
                    .map(|p| {
                        let (a, r) = snap.stale_lines.get(&(commit.clone(), p.clone())).copied().unwrap_or((0, 0));
                        (p.clone(), a, r)
                    })
                    .collect();
                let mut rows = rows;
                rows.sort_by_key(|r| std::cmp::Reverse(r.1 + r.2));
                d.child(format!("Changed since {commit}, in the {} this scenario claims:", plural(rows.len(), "file", "files")))
                    .child(diff_box(rows, th))
                    .children(snap.latest.get(&commit).map(|(sha, subject)| {
                        div().flex().gap(px(6.)).child("Latest:").child(mono(sha.clone(), text::S, th.text)).child(div().truncate().child(subject.clone()))
                    }))
            }
            State::Failing => d.child(format!("Failed at {commit}. The log is in .proof/logs.")),
            State::Missing => d.child(format!("No test matching “{}” ran in the last {} run.", s.matcher.trim_start_matches('='), s.runner)),
            State::Blocked => d.child(sv.entry.as_ref().and_then(|e| e.why.clone()).unwrap_or_else(|| "Its runner can’t run on this machine.".into())),
            State::Passing => d.child(format!("Passed at {commit}. None of the files it claims has changed since.")),
            State::Unverified => d.child("Nothing has run yet."),
        }
    }

    // ------------------------------------------------------------ Changes

    fn changes_screen(&self, snap: &Snapshot, th: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let th = *th;
        let a = &snap.affected;
        let title = match &a.since {
            Some(s) => format!("Changes since {s}"),
            None => "Uncommitted changes".into(),
        };
        let direct: Vec<_> = a.direct().collect();
        let indirect: Vec<_> = a.indirect().collect();
        let summary = if a.files.is_empty() {
            "Nothing changed.".to_string()
        } else {
            let mut s = match &a.since {
                Some(_) => format!("{} in {}.", plural(a.files.len(), "file", "files"), plural(a.commits, "commit", "commits")),
                None => format!("{} changed, not committed yet.", plural(a.files.len(), "file", "files")),
            };
            if a.source.is_empty() {
                s.push_str(" None are source.");
            } else {
                s.push_str(&format!(
                    " {} source: they hit {} directly and run inside {} more.",
                    if a.source.len() == 1 { "1 is".to_string() } else { format!("{} are", a.source.len()) },
                    plural(direct.len(), "capability", "capabilities"),
                    indirect.len()
                ));
            }
            s
        };
        let mut out = vec![div()
            .flex()
            .flex_col()
            .gap(space::S3)
            .child(
                div().flex().items_center().gap(space::S4).child(h1(title)).when(!a.hits.is_empty(), |d| {
                    d.child(
                        primary("run-affected", format!("Run {} affected", a.hits.len()), Some(shortcut(&["mod", "shift", "r"])), &th)
                            .on_click(cx.listener(|this, _, w, cx| this.run_all(&RunAll, w, cx))),
                    )
                }),
            )
            .child(div().text_color(th.text_muted).child(summary))
            .into_any_element()];

        let mut ix = 0usize;
        let mut hit_row = |h: &proof_core::affected::Hit, files: bool, this: &Self, cx: &mut Context<Self>| {
            let cap = snap.map.cap(&h.cap).unwrap();
            let s = snap.state(&h.cap);
            let id = h.cap.clone();
            let selected = this.selected == Some(ix);
            ix += 1;
            list_row(gpui::SharedString::from(format!("hit-{}", h.cap)), selected, &th)
                .min_h(px(44.))
                .py(px(6.))
                .on_click(cx.listener(move |this, _, _, cx| this.go(Screen::Capability(id.clone()), cx)))
                .child(glyph(s, &th))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .gap(space::S2)
                        .child(div().text_color(th.text_faint).child(format!("{} ›", cap.group)))
                        .child(div().truncate().child(cap.name.clone())),
                )
                .when(files, |d| d.child(mono(h.direct.iter().map(|f| file_name(f)).collect::<Vec<_>>().join(", "), text::XS, th.text_muted)))
        };
        if !direct.is_empty() {
            let rows: Vec<_> = direct.iter().map(|h| hit_row(h, true, self, cx)).collect();
            out.push(div().flex().flex_col().child(section_head("Hit directly", direct.len().to_string(), &th)).children(rows).into_any_element());
        }

        let loose: Vec<String> = a.incidental.iter().chain(&a.unmapped).cloned().collect();
        if !loose.is_empty() {
            let text_ = if a.incidental.is_empty() {
                "Nothing claims these source files, and no traced test runs them.".to_string()
            } else {
                format!(
                    "{} of all tests run these, on the way to something else. Likely a capability the map doesn’t have yet.",
                    share(a.incidental_reach)
                )
            };
            let mut rows: Vec<(String, u32, u32)> = loose.iter().map(|f| (f.clone(), a.lines.get(f).map(|l| l.0).unwrap_or(0), a.lines.get(f).map(|l| l.1).unwrap_or(0))).collect();
            rows.sort_by_key(|r| std::cmp::Reverse(r.1 + r.2));
            let (f1, f2) = (loose.clone(), loose.clone());
            out.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(space::S3)
                    .p(space::S4)
                    .rounded(radius::L)
                    .bg(th.surface_raised)
                    .border_1()
                    .border_color(th.border)
                    .child(div().flex().items_end().child(div().flex_1().text_size(text::M).font_weight(FontWeight::MEDIUM).child("No capability owns these")).child(mono(loose.len().to_string(), text::XS, th.text_faint)))
                    .child(div().text_color(th.text_muted).child(text_))
                    .children(snap.subjects.iter().next().map(|(sha, subject)| {
                        div().flex().gap(px(6.)).text_size(text::S).text_color(th.text_muted).child("From").child(mono(sha.clone(), text::S, th.text)).child(div().truncate().child(subject.clone()))
                    }))
                    .child(diff_box(rows, &th))
                    .child(
                        div()
                            .flex()
                            .gap(space::S2)
                            .child(button("add-to", "Add to a capability", &th).on_click(cx.listener(move |this, _, w, cx| this.open_palette_mode(Mode::AddTo(f1.clone()), w, cx))))
                            .child(button("new-cap", "New capability", &th).on_click(cx.listener(move |this, _, w, cx| this.open_palette_mode(Mode::NewCap(f2.clone()), w, cx)))),
                    )
                    .into_any_element(),
            );
        }

        if !indirect.is_empty() {
            let rows: Vec<_> = indirect.iter().map(|h| hit_row(h, false, self, cx)).collect();
            out.push(
                div()
                    .flex()
                    .flex_col()
                    .child(section_head("Run the changed code", format!("{} · via shared files", indirect.len()), &th))
                    .children(rows)
                    .into_any_element(),
            );
        }
        let outside = a.outside();
        if outside > 0 {
            let claimed: std::collections::BTreeSet<&String> = a.hits.iter().flat_map(|h| h.files.iter()).collect();
            let names: Vec<&str> = a.files.iter().filter(|f| !a.source.contains(f) && !claimed.contains(f)).map(|f| file_name(f)).take(4).collect();
            let more = if outside > names.len() { "…" } else { "" };
            out.push(
                div()
                    .px(px(10.))
                    .text_size(text::S)
                    .text_color(th.text_faint)
                    .child(format!("{} changed outside the map’s source: {}{more}", plural(outside, "more file", "more files"), names.join(", ")))
                    .into_any_element(),
            );
        }
        out
    }

    // ------------------------------------------------------------ Unclaimed

    fn unclaimed_screen(&self, snap: &Snapshot, th: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let th = *th;
        let n = snap.unclaimed.len();
        let summary = match (snap.source, n) {
            (0, _) => "No files match the map’s source globs.".to_string(),
            (all, 0) => format!("All {all} source files are claimed by a capability."),
            (all, n) => format!("{} of {all} source files are claimed by a capability. {} not.", all - n, if n == 1 { "1 is".to_string() } else { format!("{n} are") }),
        };
        let head = div().flex().flex_col().gap(space::S3).child(h1("Unclaimed source")).child(div().text_color(th.text_muted).child(summary));
        let dirs = self.unclaimed_dirs();
        if dirs.is_empty() {
            return vec![head.into_any_element()];
        }

        let mut rows = Vec::new();
        for (i, (dir, files)) in dirs.into_iter().enumerate() {
            let selected = self.selected == Some(i);
            let names: Vec<&str> = files.iter().map(|f| file_name(f)).collect();
            let row = list_row(gpui::SharedString::from(format!("unclaimed-{i}")), selected, &th)
                .min_h(px(44.))
                .py(px(6.))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected = if this.selected == Some(i) { None } else { Some(i) };
                    cx.notify();
                }))
                .child(mono(format!("{dir}/"), text::S, th.text).flex_none().max_w(px(320.)).truncate())
                .child(div().flex_1().min_w_0().truncate().text_size(text::S).text_color(th.text_faint).child(names.join(", ")))
                .child(mono(files.len().to_string(), text::XS, th.text_faint).flex_none());
            let mut block = div().flex().flex_col().border_b_1().border_color(th.border).child(row.border_b_0());
            if selected {
                let (f1, f2) = (files.clone(), files.clone());
                block = block.bg(th.surface_raised).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(space::S3)
                        .pt(px(2.))
                        .pb(space::S4)
                        .px(px(10.))
                        .child(div().flex().flex_wrap().gap(space::S2).children(names.iter().map(|n| path_chip(n.to_string(), &th))))
                        .child(
                            div()
                                .flex()
                                .gap(space::S2)
                                .child(
                                    button("add-to", "Add to a capability", &th)
                                        .child(kbd("↵", th.text_faint))
                                        .on_click(cx.listener(move |this, _, w, cx| this.open_palette_mode(Mode::AddTo(f1.clone()), w, cx))),
                                )
                                .child(button("new-cap", "New capability", &th).on_click(cx.listener(move |this, _, w, cx| this.open_palette_mode(Mode::NewCap(f2.clone()), w, cx)))),
                        ),
                );
            }
            rows.push(block);
        }
        let list = div().flex().flex_col().child(section_head("By folder", plural(n, "file", "files"), &th)).children(rows);
        vec![head.into_any_element(), list.into_any_element()]
    }

    // ------------------------------------------------------------ Runners

    fn runners_screen(&self, snap: &Snapshot, th: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let th = *th;
        let head = div().flex().items_center().gap(space::S4).child(h1("Runners")).child(if self.running.is_some() {
            button("stop", "Stop", &th)
                .child(kbd(shortcut(&["mod", "."]), th.text_faint))
                .on_click(cx.listener(|this, _, w, cx| this.stop_run(&StopRun, w, cx)))
        } else {
            primary("run-all", "Run everything stale", Some(shortcut(&["mod", "shift", "r"])), &th).on_click(cx.listener(|this, _, w, cx| this.run_all(&RunAll, w, cx)))
        });

        let mut list = div().flex().flex_col().border_t_1().border_color(th.border);
        if let Some(run) = self.running.as_ref().filter(|r| r.tracing) {
            list = list.child(self.running_card(run, "proof trace", &th));
        }
        let mut first_blocked: Option<&str> = None;
        for (i, r) in snap.map.runners.iter().enumerate() {
            let name = r.name.clone();
            let selected = self.selected == Some(i);
            if let Some(run) = self.running.as_ref().filter(|run| !run.tracing && run.runner == r.name) {
                list = list.child(self.running_card(run, &format!("{}{}", r.run, runner_suffix(snap, r)), &th));
                continue;
            }
            let queued = self.running.as_ref().is_some_and(|run| run.queue.contains(&r.name));
            let row = list_row(gpui::SharedString::from(format!("runner-{}", r.name)), selected, &th).min_h(px(56.)).py(space::S2);
            let row = row.on_click(cx.listener(move |this, _, _, cx| {
                this.selected = Some(i);
                cx.notify();
            }));
            if let Some(why) = snap.blocked_now.get(&r.name) {
                let same = first_blocked == Some(why.as_str());
                let rn = name.clone();
                let mut block = div()
                    .flex()
                    .flex_col()
                    .border_b_1()
                    .border_color(th.border)
                    .child(
                        row.border_b_0()
                            .child(glyph(State::Blocked, &th))
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .gap(space::S2)
                                    .child(div().font_family(MONO).child(r.name.clone()))
                                    .child(div().text_color(th.text_muted).child(if same { "blocked, same reason" } else { "blocked, can’t run on this machine" })),
                            )
                            .child(button(gpui::SharedString::from(format!("import-{}", r.name)), "Import CI log", &th).on_click(cx.listener(move |this, _, _, cx| this.import_for(rn.clone(), cx)))),
                    );
                if !same {
                    block = block.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(space::S3)
                            .pl(px(42.))
                            .pr(px(10.))
                            .pb(space::S4)
                            .child(mono(why.clone(), text::S, th.text_muted).px(space::S3).py(px(10.)).rounded(radius::M).bg(th.surface_sunken).border_1().border_color(th.border))
                            .when(first_blocked.is_none(), |d| {
                                d.child(div().text_size(text::S).text_color(th.text_faint).child("Blocked is not failing. Its scenarios keep the last verdict, marked with the commit it came from."))
                            }),
                    );
                }
                if first_blocked.is_none() {
                    first_blocked = Some(why.as_str());
                }
                list = list.child(block);
                continue;
            }
            let (g, color, said, at) = match snap.store.runs.get(&r.name) {
                _ if queued => ("○", th.text_faint, "queued".to_string(), String::new()),
                Some(rec) if rec.failed > 0 => ("✗", th.danger, format!("{} passed, {} failed", rec.passed, rec.failed), format!("{} · {} · {}", rec.commit, source_label(&rec.source), day(&rec.at))),
                Some(rec) => ("✓", th.success, format!("{} passed", rec.passed), format!("{} · {} · {}", rec.commit, source_label(&rec.source), day(&rec.at))),
                None => ("○", th.text_faint, "never run here".to_string(), String::new()),
            };
            list = list.child(
                row.border_b_1()
                    .border_color(th.border)
                    .child(glyph_text(g, color))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().flex().gap(space::S2).child(div().font_family(MONO).child(name)).child(div().text_color(th.text_muted).child(said)))
                            .child(
                                mono(String::new(), text::XS, th.text_faint)
                                    .flex()
                                    .overflow_hidden()
                                    .child(div().min_w_0().truncate().child(r.run.clone()))
                                    .child(div().flex_none().whitespace_nowrap().child(runner_suffix(snap, r))),
                            ),
                    )
                    .child(mono(at, text::XS, th.text_faint).whitespace_nowrap()),
            );
        }
        vec![head.into_any_element(), list.into_any_element()]
    }

    fn running_card(&self, run: &Running, cmd: &str, th: &Theme) -> Div {
        let elapsed = run.started.elapsed().as_secs();
        let progress = match run.expected {
            Some(n) if n > 0 => format!("{}, {} of {} tests", if run.tracing { "tracing" } else { "running" }, run.seen.min(n), n),
            _ if run.tracing => "tracing, finding the tests".to_string(),
            _ => format!("{}, {} so far", if run.tracing { "tracing" } else { "running" }, plural(run.seen, "test", "tests")),
        };
        let frac = run.expected.filter(|n| *n > 0).map(|n| (run.seen as f32 / n as f32).min(1.0));
        let spinner: AnyElement = if self.reduced {
            svg().path("icons/ring.svg").size(px(14.)).text_color(th.accent).into_any_element()
        } else {
            svg()
                .path("icons/arc.svg")
                .size(px(14.))
                .text_color(th.accent)
                .with_animation("spin", Animation::new(Duration::from_millis(900)).repeat(), |s, t| s.with_transformation(Transformation::rotate(percentage(t))))
                .into_any_element()
        };
        let reduced = self.reduced;
        div()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(th.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::S3)
                    .min_h(px(56.))
                    .py(space::S2)
                    .px(px(10.))
                    .child(div().w(px(20.)).flex().justify_center().child(spinner))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().flex().gap(space::S2).child(div().font_family(MONO).child(run.runner.clone())).child(div().text_color(th.text_muted).child(progress)))
                            .child(mono(cmd.to_string(), text::XS, th.text_faint).truncate()),
                    )
                    .child(mono(format!("{}:{:02}", elapsed / 60, elapsed % 60), text::S, th.text_muted)),
            )
            .child(
                div().h(px(2.)).mx(px(10.)).bg(th.border).rounded(px(1.)).overflow_hidden().child(
                    div().h_full().bg(th.accent).when_some(frac, |d, f| d.w(gpui::relative(f))).when(frac.is_none(), |d| d.w(px(0.))),
                ),
            )
            .child(
                div().flex().flex_col().gap(px(2.)).pt(space::S2).pb(px(14.)).pl(px(42.)).pr(px(10.)).children(run.live.iter().map(|l| {
                    let (g, c) = if l.ok { ("✓", th.success) } else { ("✗", th.danger) };
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .min_h(px(26.))
                        .text_size(text::S)
                        .child(settle(("live", l.n), g, c, text::S, 0, reduced))
                        .child(div().truncate().text_color(th.text_muted).child(l.test.clone()))
                        .child(mono(l.cap.clone(), text::XS, th.text_faint).whitespace_nowrap())
                })),
            )
    }

    // ------------------------------------------------------------ states

    fn no_map(&self, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        match &self.error {
            Some(e @ MapError::Invalid { .. }) => self.map_error(e, false, th, cx),
            _ => {
                let dir = dir_name(self.map_path.parent().unwrap_or(&self.map_path));
                let ask = "Map this project with proof.";
                card(th)
                    .child(div().flex().items_center().gap(space::S2).child(glyph_text("○", th.text_faint)).child(div().text_size(text::M).font_weight(FontWeight::MEDIUM).child(format!("No map in {dir} yet"))))
                    .child(div().text_color(th.text_muted).child("Your coding agent writes proof.toml from the code and tests. Paste this to it:"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(space::S2)
                            .pl(space::S3)
                            .pr(space::S1)
                            .py(space::S1)
                            .rounded(radius::M)
                            .bg(th.surface_sunken)
                            .border_1()
                            .border_color(th.border)
                            .child(mono(ask, text::S, th.text).flex_1())
                            .child(icon_button("copy-ask", "icons/copy.svg", th).on_click(cx.listener(move |this, _, _, cx| this.copy(ask.to_string(), cx)))),
                    )
                    .into_any_element()
            }
        }
    }

    fn map_error(&self, e: &MapError, kept: bool, th: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (line, message) = match e {
            MapError::Invalid { line, message, .. } => (*line, message.clone()),
            MapError::NotFound(p) => (None, format!("{} is gone", p.display())),
        };
        let file = self.map_path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| "proof.toml".into());
        card(th)
            .child(div().flex().items_center().gap(space::S2).child(glyph(State::Failing, th)).child(div().text_size(text::M).font_weight(FontWeight::MEDIUM).child(format!("{file} can’t be read"))))
            .child(mono(match line {
                Some(n) => format!("line {n}: {message}"),
                None => message,
            }, text::S, th.text_muted).px(space::S3).py(px(10.)).rounded(radius::M).bg(th.surface_sunken).border_1().border_color(th.border))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::S3)
                    .child(button("open-map", match line {
                        Some(n) => format!("Open at line {n}"),
                        None => "Open the map".into(),
                    }, th).on_click(cx.listener(move |this, _, _, cx| this.open_in_editor(line, cx))))
                    .when(kept, |d| d.child(div().text_size(text::S).text_color(th.text_faint).child("The last good map stays on screen until this is fixed."))),
            )
            .into_any_element()
    }
}

/// " · Node 24.14 · traced": what a runner's command runs on here, and whether it's traced.
fn runner_suffix(snap: &Snapshot, r: &proof_core::Runner) -> String {
    let mut out = String::new();
    if let Some(v) = snap.runtimes.get(&r.name) {
        out.push_str(&format!(" · {v}"));
    }
    if r.traced() {
        out.push_str(" · traced");
    }
    out
}

fn h1(t: impl Into<gpui::SharedString>) -> Div {
    div().flex_1().min_w_0().text_size(text::XL).font_weight(FontWeight::MEDIUM).child(t.into())
}

/// A row in a bordered list; selected rows get the accent wash and a 2px bar on the left.
fn list_row(id: gpui::SharedString, selected: bool, th: &Theme) -> Stateful<Div> {
    let hover = th.surface_hover;
    div()
        .id(id)
        .relative()
        .flex()
        .items_center()
        .gap(space::S3)
        .px(px(10.))
        .border_b_1()
        .border_color(th.border)
        .cursor_pointer()
        .when(selected, |d| d.bg(th.accent_subtle).child(div().absolute().left_0().top_0().bottom_0().w(px(2.)).bg(th.accent)))
        .when(!selected, |d| d.hover(move |s| s.bg(hover)))
}

/// 0.25 -> "A quarter"
fn share(f: f64) -> &'static str {
    let words = [(0.1, "A tenth"), (0.2, "A fifth"), (0.25, "A quarter"), (1. / 3., "A third"), (0.5, "Half"), (2. / 3., "Two thirds"), (0.75, "Three quarters"), (1.0, "All")];
    words.iter().min_by(|a, b| (a.0 - f).abs().total_cmp(&(b.0 - f).abs())).map(|w| w.1).unwrap_or("Some")
}

