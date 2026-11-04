//! The window's one view: state, actions, and running things in the background.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::channel::mpsc;
use futures::StreamExt;
use gpui::{
    actions, App, ClipboardItem, Context, FocusHandle, KeyBinding, PathPromptOptions, ScrollHandle, Task, Window,
    WindowAppearance,
};
use proof_core::map::matches;
use proof_core::run::{self, Event};
use proof_core::{edit, Map, MapError, State};

use crate::snapshot::{signature, Snapshot};
use crate::theme::Theme;

actions!(proof, [OpenPalette, GoMap, GoChanges, GoRunners, GoUnclaimed, RunPrimary, RunAll, ImportLog, CopyCommand, StopRun, Next, Prev, Open, Back, Quit, PageDown, PageUp, Top, Bottom]);
actions!(palette, [PaletteUp, PaletteDown, PaletteConfirm, PaletteClose]);

pub fn bind_keys(cx: &mut App) {
    let main = Some("ProofApp && !Palette");
    cx.bind_keys([
        KeyBinding::new("secondary-k", OpenPalette, Some("ProofApp")),
        KeyBinding::new("secondary-1", GoMap, Some("ProofApp")),
        KeyBinding::new("secondary-2", GoChanges, Some("ProofApp")),
        KeyBinding::new("secondary-3", GoRunners, Some("ProofApp")),
        KeyBinding::new("secondary-4", GoUnclaimed, Some("ProofApp")),
        KeyBinding::new("secondary-r", RunPrimary, Some("ProofApp")),
        KeyBinding::new("secondary-shift-r", RunAll, Some("ProofApp")),
        KeyBinding::new("secondary-i", ImportLog, Some("ProofApp")),
        KeyBinding::new("secondary-shift-c", CopyCommand, Some("ProofApp")),
        KeyBinding::new("secondary-.", StopRun, Some("ProofApp")),
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("j", Next, main),
        KeyBinding::new("down", Next, main),
        KeyBinding::new("k", Prev, main),
        KeyBinding::new("up", Prev, main),
        KeyBinding::new("enter", Open, main),
        KeyBinding::new("escape", Back, main),
        KeyBinding::new("pagedown", PageDown, main),
        KeyBinding::new("space", PageDown, main),
        KeyBinding::new("pageup", PageUp, main),
        KeyBinding::new("shift-space", PageUp, main),
        KeyBinding::new("home", Top, main),
        KeyBinding::new("end", Bottom, main),
        KeyBinding::new("up", PaletteUp, Some("Palette")),
        KeyBinding::new("down", PaletteDown, Some("Palette")),
        KeyBinding::new("enter", PaletteConfirm, Some("Palette")),
        KeyBinding::new("escape", PaletteClose, Some("Palette")),
    ]);
}

/// How long the first load can take before the window says it's reading the map.
pub const SLOW_LOAD: Duration = Duration::from_millis(600);

#[derive(Clone, Debug, PartialEq)]
pub enum Screen {
    Map,
    Capability(String),
    Changes,
    Runners,
    Unclaimed,
}

/// What the palette is asking for.
#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    Commands,
    /// Pick a capability to claim these files.
    AddTo(Vec<String>),
    /// Name a new capability that claims these files.
    NewCap(Vec<String>),
    /// Pick the runner a log belongs to.
    ImportFor,
}

pub struct Palette {
    pub query: String,
    pub sel: usize,
    pub mode: Mode,
}

/// Something the palette can do, with the CLI command that does the same.
#[derive(Clone, Debug)]
pub enum Cmd {
    Run(Vec<String>),
    RunStale,
    Import(Option<String>),
    Trace,
    Go(Screen),
    Review(String),
    Claim(String, Vec<String>),
    NewCap(String, Vec<String>),
    ToggleTheme,
    /// Ask a follow-up question in the palette.
    Pick(Mode),
}

pub struct LiveResult {
    pub n: usize,
    pub ok: bool,
    pub test: String,
    pub cap: String,
}

pub struct Running {
    /// The runner going now; `trace` while tracing.
    pub runner: String,
    pub tracing: bool,
    pub seen: usize,
    pub failed: usize,
    pub expected: Option<usize>,
    pub started: Instant,
    pub live: Vec<LiveResult>,
    pub queue: Vec<String>,
    pub stop: Arc<AtomicBool>,
}

enum Msg {
    Event(Event),
    Trace(usize, usize),
    /// A trace finished; whether any claims changed.
    Traced(bool),
    Done(Option<String>),
}

pub struct ProofApp {
    pub map_path: PathBuf,
    pub snap: Option<Arc<Snapshot>>,
    pub error: Option<MapError>,
    pub theme: Theme,
    theme_forced: bool,
    pub reduced: bool,
    pub screen: Screen,
    pub filter: Option<State>,
    pub selected: Option<usize>,
    pub open_scenario: Option<String>,
    pub show_traced: bool,
    /// None: the default base; Some(None): uncommitted only; Some(Some(rev)).
    pub since: Option<Option<String>>,
    pub palette: Option<Palette>,
    pub running: Option<Running>,
    pub toast: Option<(String, Instant)>,
    pub focus: FocusHandle,
    pub palette_focus: FocusHandle,
    pub scroll: ScrollHandle,
    sig: String,
    pub loading: bool,
    pub opened: Instant,
    _tasks: Vec<Task<()>>,
}

pub struct Options {
    pub map: PathBuf,
    pub since: Option<String>,
    pub theme: Option<String>,
    pub screen: Option<String>,
    pub palette: Option<String>,
}

impl ProofApp {
    pub fn new(opts: Options, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let theme_forced = opts.theme.is_some();
        let dark = match opts.theme.as_deref() {
            Some("light") => false,
            Some(_) => true,
            None => !matches!(window.appearance(), WindowAppearance::Light | WindowAppearance::VibrantLight),
        };
        let reduced = reduced_motion();
        let mut this = ProofApp {
            map_path: opts.map,
            snap: None,
            error: None,
            theme: if dark { Theme::dark() } else { Theme::light() },
            theme_forced,
            reduced,
            screen: match opts.screen.as_deref() {
                Some("changes") => Screen::Changes,
                Some("runners") => Screen::Runners,
                Some("unclaimed") => Screen::Unclaimed,
                Some(id) if id != "map" => Screen::Capability(id.to_string()),
                _ => Screen::Map,
            },
            filter: None,
            selected: None,
            open_scenario: None,
            show_traced: false,
            since: opts.since.map(|s| Some(s).filter(|s| s != "uncommitted")),
            palette: opts.palette.map(|q| Palette { query: q, sel: 0, mode: Mode::Commands }),
            running: None,
            toast: None,
            palette_focus: cx.focus_handle(),
            focus,
            scroll: ScrollHandle::new(),
            sig: String::new(),
            loading: true,
            opened: Instant::now(),
            _tasks: Vec::new(),
        };
        if this.palette.is_some() {
            window.focus(&this.palette_focus);
        }
        // the first load runs off the main thread, so the window opens at once even when git is slow
        let (path, since) = (this.map_path.clone(), this.since.clone());
        this._tasks.push(cx.spawn_in(window, async move |this, cx| {
            let (r, sig) = cx
                .background_executor()
                .spawn(async move {
                    let r = Snapshot::load(&path, since, true, None);
                    let sig = signature(&path, r.as_ref().ok().map(|s| s.map.root.as_path()));
                    (r, sig)
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.loading = false;
                this.sig = sig;
                match r {
                    Ok(s) => {
                        window.set_window_title(&format!("proof · {}", s.map.name));
                        this.snap = Some(Arc::new(s));
                        if let Screen::Capability(id) = &this.screen {
                            this.open_scenario = this.first_stale(id);
                        }
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        }));
        // a slow first load says so, after a beat
        this._tasks.push(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SLOW_LOAD).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        }));
        cx.observe_window_appearance(window, |this, window, cx| {
            if !this.theme_forced {
                let light = matches!(window.appearance(), WindowAppearance::Light | WindowAppearance::VibrantLight);
                this.theme = if light { Theme::light() } else { Theme::dark() };
                cx.notify();
            }
        })
        .detach();
        this._tasks.push(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let Ok((path, root, old)) = this.read_with(cx, |this, _| {
                (this.map_path.clone(), this.snap.as_ref().map(|s| s.map.root.clone()), this.sig.clone())
            }) else {
                return;
            };
            let sig = cx.background_executor().spawn(async move { signature(&path, root.as_deref()) }).await;
            if sig != old && this.update(cx, |this, cx| this.reload(false, cx)).is_err() {
                return;
            }
        }));
        this
    }

    pub fn reload(&mut self, check_runners: bool, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        let (path, since, prev) = (self.map_path.clone(), self.since.clone(), self.snap.clone());
        let started = Instant::now();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let r = Snapshot::load(&path, since, check_runners, prev.as_deref());
                    let sig = signature(&path, r.as_ref().ok().map(|s| s.map.root.as_path()));
                    (r, sig)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                let (r, sig) = result;
                this.sig = sig;
                match r {
                    Ok(s) => {
                        this.snap = Some(Arc::new(s));
                        this.error = None;
                    }
                    // the last good map stays on screen
                    Err(e) => this.error = Some(e),
                }
                if std::env::var_os("PROOF_DEBUG").is_some() {
                    eprintln!("refreshed in {:?}", started.elapsed());
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn say(&mut self, msg: impl Into<String>, cx: &mut Context<Self>) {
        self.toast = Some((msg.into(), Instant::now()));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(2600)).await;
            let _ = this.update(cx, |this, cx| {
                if this.toast.as_ref().is_some_and(|(_, at)| at.elapsed() >= Duration::from_millis(2500)) {
                    this.toast = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub fn copy(&mut self, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        self.say(format!("Copied {text}"), cx);
    }

    pub fn go(&mut self, screen: Screen, cx: &mut Context<Self>) {
        let back_to = match (&self.screen, &screen) {
            (Screen::Capability(id), Screen::Map) => self.visible_caps().iter().position(|c| c == id),
            _ => None,
        };
        if let Screen::Capability(id) = &screen {
            self.open_scenario = self.first_stale(id);
            self.show_traced = false;
        }
        self.screen = screen;
        self.selected = back_to;
        self.scroll.set_offset(gpui::point(gpui::px(0.), gpui::px(0.)));
        if let Some(ix) = back_to {
            self.scroll_to_cap(ix);
        }
        cx.notify();
    }

    /// Scrolls the main column by `pages` of its height, clamped to the ends.
    pub fn scroll_page(&mut self, pages: f32, cx: &mut Context<Self>) {
        let page = self.scroll.bounds().size.height * 0.85;
        let max = self.scroll.max_offset().height;
        let mut at = self.scroll.offset();
        at.y = (at.y - page * pages).clamp(-max, gpui::px(0.));
        self.scroll.set_offset(at);
        cx.notify();
    }

    fn first_stale(&self, id: &str) -> Option<String> {
        let snap = self.snap.as_ref()?;
        let cap = snap.map.cap(id)?;
        let v = snap.verdicts.get(id)?;
        cap.scenarios.iter().zip(&v.scenarios).find(|(_, s)| s.state == State::Stale).map(|(s, _)| s.id.clone())
    }

    /// Capability ids on the Map screen, in order, under the current filter.
    pub fn visible_caps(&self) -> Vec<String> {
        let Some(snap) = &self.snap else { return vec![] };
        snap.map.caps.iter().filter(|c| self.filter.is_none_or(|f| snap.state(&c.id) == f)).map(|c| c.id.clone()).collect()
    }

    fn scroll_to_cap(&self, ix: usize) {
        let Some(snap) = &self.snap else { return };
        let ids = self.visible_caps();
        let Some(id) = ids.get(ix) else { return };
        let groups: Vec<String> = snap
            .map
            .groups()
            .into_iter()
            .filter(|(_, _, caps)| caps.iter().any(|c| ids.contains(&c.id)))
            .map(|(gid, _, _)| gid)
            .collect();
        let gid = &snap.map.cap(id).unwrap().group_id;
        if let Some(g) = groups.iter().position(|g| g == gid) {
            // child 0 is the heading block
            self.scroll.scroll_to_item(g + 1);
        }
    }

    fn list_len(&self) -> usize {
        let Some(snap) = &self.snap else { return 0 };
        match &self.screen {
            Screen::Map => self.visible_caps().len(),
            Screen::Capability(id) => snap.map.cap(id).map(|c| c.scenarios.len()).unwrap_or(0),
            Screen::Changes => snap.affected.hits.len(),
            Screen::Runners => snap.map.runners.len(),
            Screen::Unclaimed => self.unclaimed_dirs().len(),
        }
    }

    /// Hits in the order the Changes screen lists them.
    pub fn ordered_hits(&self) -> Vec<String> {
        let Some(snap) = &self.snap else { return vec![] };
        snap.affected.direct().chain(snap.affected.indirect()).map(|h| h.cap.clone()).collect()
    }

    /// Unclaimed source by directory, in the order the Unclaimed screen lists them.
    pub fn unclaimed_dirs(&self) -> Vec<(String, Vec<String>)> {
        let Some(snap) = &self.snap else { return vec![] };
        let mut out: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for f in &snap.unclaimed {
            let dir = f.rsplit_once('/').map(|(d, _)| d).unwrap_or(".");
            out.entry(dir.to_string()).or_default().push(f.clone());
        }
        out.into_iter().collect()
    }

    // ------------------------------------------------------------ actions

    pub fn open_palette(&mut self, _: &OpenPalette, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.close_palette(window, cx);
            return;
        }
        self.palette = Some(Palette { query: String::new(), sel: 0, mode: Mode::Commands });
        window.focus(&self.palette_focus);
        cx.notify();
    }

    pub fn open_palette_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = Some(Palette { query: String::new(), sel: 0, mode });
        window.focus(&self.palette_focus);
        cx.notify();
    }

    pub fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        window.focus(&self.focus);
        cx.notify();
    }

    pub fn go_map(&mut self, _: &GoMap, _: &mut Window, cx: &mut Context<Self>) {
        self.go(Screen::Map, cx);
    }
    pub fn go_changes(&mut self, _: &GoChanges, _: &mut Window, cx: &mut Context<Self>) {
        self.go(Screen::Changes, cx);
    }
    pub fn go_runners(&mut self, _: &GoRunners, _: &mut Window, cx: &mut Context<Self>) {
        self.go(Screen::Runners, cx);
    }
    pub fn go_unclaimed(&mut self, _: &GoUnclaimed, _: &mut Window, cx: &mut Context<Self>) {
        self.go(Screen::Unclaimed, cx);
    }

    pub fn next(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.list_len();
        if n == 0 {
            return;
        }
        self.selected = Some(self.selected.map(|s| (s + 1).min(n - 1)).unwrap_or(0));
        if self.screen == Screen::Map {
            self.scroll_to_cap(self.selected.unwrap());
        }
        cx.notify();
    }

    pub fn prev(&mut self, _: &Prev, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.list_len();
        if n == 0 {
            return;
        }
        self.selected = Some(self.selected.map(|s| s.saturating_sub(1)).unwrap_or(0));
        if self.screen == Screen::Map {
            self.scroll_to_cap(self.selected.unwrap());
        }
        cx.notify();
    }

    pub fn open(&mut self, _: &Open, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sel) = self.selected else { return };
        let Some(snap) = self.snap.clone() else { return };
        match self.screen.clone() {
            Screen::Map => {
                if let Some(id) = self.visible_caps().get(sel) {
                    self.go(Screen::Capability(id.clone()), cx);
                }
            }
            Screen::Capability(id) => {
                if let Some(s) = snap.map.cap(&id).and_then(|c| c.scenarios.get(sel)) {
                    self.toggle_scenario(s.id.clone(), cx);
                }
            }
            Screen::Changes => {
                if let Some(id) = self.ordered_hits().get(sel) {
                    self.go(Screen::Capability(id.clone()), cx);
                }
            }
            Screen::Runners => {
                if let Some(r) = snap.map.runners.get(sel) {
                    self.start_run(vec![r.name.clone()], cx);
                }
            }
            Screen::Unclaimed => {
                if let Some((_, files)) = self.unclaimed_dirs().into_iter().nth(sel) {
                    self.open_palette_mode(Mode::AddTo(files), window, cx);
                }
            }
        }
    }

    pub fn toggle_scenario(&mut self, id: String, cx: &mut Context<Self>) {
        self.open_scenario = if self.open_scenario.as_deref() == Some(&id) { None } else { Some(id) };
        cx.notify();
    }

    pub fn back(&mut self, _: &Back, _: &mut Window, cx: &mut Context<Self>) {
        match &self.screen {
            Screen::Map => {
                self.filter = None;
                self.selected = None;
                cx.notify();
            }
            _ => self.go(Screen::Map, cx),
        }
    }

    pub fn quit(&mut self, _: &Quit, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(r) = &self.running {
            r.stop.store(true, Ordering::Relaxed);
        }
        cx.quit();
    }

    /// The CLI command for what's on screen or selected.
    pub fn current_command(&self) -> String {
        let Some(snap) = &self.snap else { return "proof guide".into() };
        match &self.screen {
            Screen::Map => match self.selected.and_then(|i| self.visible_caps().get(i).cloned()) {
                Some(id) => format!("proof show {id}"),
                None => "proof status".into(),
            },
            Screen::Capability(id) => format!("proof show {id}"),
            Screen::Changes => match &snap.affected.since {
                Some(s) => format!("proof affected --since {s}"),
                None => "proof affected".into(),
            },
            Screen::Runners => match self.selected.and_then(|i| snap.map.runners.get(i)) {
                Some(r) => format!("proof run --runner {}", r.name),
                None => "proof run --stale".into(),
            },
            Screen::Unclaimed => "proof gaps".into(),
        }
    }

    pub fn copy_command(&mut self, _: &CopyCommand, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(cmd) = self.palette.as_ref().and_then(|_| crate::palette::selected_cli(self)) {
            self.copy(cmd, cx);
            return;
        }
        let cmd = self.current_command();
        self.copy(cmd, cx);
    }

    /// Runners with scenarios that aren't passing, across the given capabilities (all when None).
    pub fn runners_needed(&self, caps: Option<&[String]>) -> Vec<String> {
        let Some(snap) = &self.snap else { return vec![] };
        let mut out: Vec<String> = Vec::new();
        for r in &snap.map.runners {
            let needed = snap.map.caps.iter().filter(|c| caps.is_none_or(|ids| ids.contains(&c.id))).any(|c| {
                c.scenarios.iter().zip(&snap.verdicts[&c.id].scenarios).any(|(s, v)| s.runner == r.name && v.state != State::Passing)
            });
            if needed {
                out.push(r.name.clone());
            }
        }
        out
    }

    /// Every runner a capability's scenarios use.
    pub fn runners_of(&self, id: &str) -> Vec<String> {
        let Some(snap) = &self.snap else { return vec![] };
        let Some(cap) = snap.map.cap(id) else { return vec![] };
        snap.map.runners.iter().filter(|r| cap.scenarios.iter().any(|s| s.runner == r.name)).map(|r| r.name.clone()).collect()
    }

    pub fn run_primary(&mut self, _: &RunPrimary, _: &mut Window, cx: &mut Context<Self>) {
        let Some(snap) = self.snap.clone() else { return };
        let runners = match &self.screen {
            Screen::Capability(id) => self.cap_run_target(id),
            Screen::Map => match self.selected.and_then(|i| self.visible_caps().get(i).cloned()) {
                Some(id) => self.cap_run_target(&id),
                None => self.runners_needed(None),
            },
            Screen::Runners => match self.selected.and_then(|i| snap.map.runners.get(i)) {
                Some(r) => vec![r.name.clone()],
                None => self.runners_needed(None),
            },
            Screen::Changes => self.affected_runners(),
            Screen::Unclaimed => self.runners_needed(None),
        };
        self.start_run(runners, cx);
    }

    pub fn run_all(&mut self, _: &RunAll, _: &mut Window, cx: &mut Context<Self>) {
        let runners = if self.screen == Screen::Changes { self.affected_runners() } else { self.runners_needed(None) };
        self.start_run(runners, cx);
    }

    /// What ⌘R runs for a capability: its runners that have something to say, or all of them.
    pub fn cap_run_target(&self, id: &str) -> Vec<String> {
        let needed = self.runners_needed(Some(&[id.to_string()]));
        if needed.is_empty() {
            self.runners_of(id)
        } else {
            needed
        }
    }

    pub fn affected_runners(&self) -> Vec<String> {
        let Some(snap) = &self.snap else { return vec![] };
        let ids: Vec<String> = snap.affected.hits.iter().map(|h| h.cap.clone()).collect();
        let mut out: Vec<String> = Vec::new();
        for id in &ids {
            for r in self.runners_of(id) {
                if !out.contains(&r) {
                    out.push(r);
                }
            }
        }
        // in map order
        snap.map.runners.iter().filter(|r| out.contains(&r.name)).map(|r| r.name.clone()).collect()
    }

    pub fn stop_run(&mut self, _: &StopRun, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(r) = &self.running {
            r.stop.store(true, Ordering::Relaxed);
            self.say("Stopping", cx);
        }
    }

    pub fn start_run(&mut self, runners: Vec<String>, cx: &mut Context<Self>) {
        if self.running.is_some() {
            self.say("A run is going already", cx);
            return;
        }
        let Some(snap) = self.snap.clone() else { return };
        if runners.is_empty() {
            self.say("Nothing to run: everything here is passing", cx);
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        self.running = Some(Running {
            runner: runners[0].clone(),
            tracing: false,
            seen: 0,
            failed: 0,
            expected: None,
            started: Instant::now(),
            live: vec![],
            queue: runners[1..].to_vec(),
            stop: stop.clone(),
        });
        if self.screen != Screen::Runners && !matches!(self.screen, Screen::Capability(_)) {
            self.go(Screen::Runners, cx);
        }
        let map: Map = snap.map.clone();
        let (tx, rx) = mpsc::unbounded();
        std::thread::spawn(move || {
            let mut error = None;
            for name in runners {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let tx2 = tx.clone();
                if let Err(e) = run::run_runner(&map, &name, stop.clone(), &mut |e| {
                    let _ = tx2.unbounded_send(Msg::Event(e));
                }) {
                    error = Some(format!("{name}: {e:#}"));
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                }
            }
            let _ = tx.unbounded_send(Msg::Done(error));
        });
        self.listen(rx, cx);
        cx.notify();
    }

    pub fn start_trace(&mut self, cx: &mut Context<Self>) {
        if self.running.is_some() {
            self.say("A run is going already", cx);
            return;
        }
        let Some(snap) = self.snap.clone() else { return };
        let stop = Arc::new(AtomicBool::new(false));
        self.running = Some(Running {
            runner: "trace".into(),
            tracing: true,
            seen: 0,
            failed: 0,
            expected: None,
            started: Instant::now(),
            live: vec![],
            queue: vec![],
            stop: stop.clone(),
        });
        self.go(Screen::Runners, cx);
        let map: Map = snap.map.clone();
        let (tx, rx) = mpsc::unbounded();
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let r = proof_core::trace::trace(&map, 4, &stop, &move |done, total| {
                let _ = progress_tx.unbounded_send(Msg::Trace(done, total));
            });
            let error = match r {
                Ok(report) => {
                    let _ = tx.unbounded_send(Msg::Traced(report.changed));
                    None
                }
                Err(e) => Some(format!("trace: {e:#}")),
            };
            let _ = tx.unbounded_send(Msg::Done(error));
        });
        self.listen(rx, cx);
    }

    fn listen(&mut self, mut rx: mpsc::UnboundedReceiver<Msg>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(msg) = rx.next().await {
                if this.update(cx, |this, cx| this.on_msg(msg, cx)).is_err() {
                    return;
                }
            }
        })
        .detach();
        // the elapsed time ticks while running
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let Ok(running) = this.update(cx, |this, cx| {
                cx.notify();
                this.running.is_some()
            }) else {
                return;
            };
            if !running {
                return;
            }
        })
        .detach();
    }

    fn on_msg(&mut self, msg: Msg, cx: &mut Context<Self>) {
        let snap = self.snap.clone();
        let Some(run) = self.running.as_mut() else { return };
        match msg {
            Msg::Event(Event::Started { runner, expected }) => {
                run.runner = runner.clone();
                run.queue.retain(|r| *r != runner);
                run.seen = 0;
                run.failed = 0;
                // TAP prints its plan last: until then, last time's count
                run.expected = expected.or_else(|| snap.as_ref().and_then(|s| s.store.runs.get(&runner)).map(|r| r.tests).filter(|n| *n > 0));
                run.started = Instant::now();
            }
            Msg::Event(Event::Item { runner, item }) => {
                run.seen += 1;
                if !item.ok {
                    run.failed += 1;
                }
                let cap = snap
                    .as_ref()
                    .and_then(|s| {
                        s.map
                            .caps
                            .iter()
                            .find(|c| c.scenarios.iter().any(|sc| sc.runner == runner && matches(&sc.matcher, &item.name)))
                            .map(|c| c.name.clone())
                    })
                    .unwrap_or_default();
                let n = run.live.last().map(|l| l.n + 1).unwrap_or(0);
                run.live.push(LiveResult { n, ok: item.ok, test: item.name, cap });
                if run.live.len() > 4 {
                    run.live.remove(0);
                }
            }
            Msg::Event(Event::Blocked { runner, .. }) => {
                run.queue.retain(|r| *r != runner);
            }
            Msg::Event(Event::Finished { runner, .. }) => {
                run.live.clear();
                run.queue.retain(|r| *r != runner);
                self.reload(true, cx);
            }
            Msg::Trace(done, total) => {
                run.seen = done;
                run.expected = Some(total);
            }
            Msg::Traced(changed) => {
                self.say(
                    if changed {
                        "Traced. Some claims changed, so results from before may now be stale."
                    } else {
                        "Traced. No claims changed."
                    },
                    cx,
                );
            }
            Msg::Done(error) => {
                let stopped = run.stop.load(Ordering::Relaxed);
                self.running = None;
                if stopped {
                    self.say("Stopped. Nothing from the stopped runner was recorded.", cx);
                } else if let Some(e) = error {
                    self.say(e, cx);
                }
                self.reload(true, cx);
            }
        }
        cx.notify();
    }

    pub fn import_log(&mut self, _: &ImportLog, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette_mode(Mode::ImportFor, window, cx);
    }

    pub fn import_for(&mut self, runner: String, cx: &mut Context<Self>) {
        let Some(snap) = self.snap.clone() else { return };
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(format!("Import a {runner} log").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let map = snap.map.clone();
            let r = cx
                .background_executor()
                .spawn(async move { run::import_log(&map, &runner, &path, None, 0, None).map(|i| (runner, i)) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok((runner, i)) => this.say(
                        format!("{runner} at {}: {} passed, {} failed", i.commit, i.record.passed, i.record.failed),
                        cx,
                    ),
                    Err(e) => this.say(format!("{e:#}"), cx),
                }
                this.reload(false, cx);
            });
        })
        .detach();
    }

    pub fn run_cmd(&mut self, cmd: Cmd, window: &mut Window, cx: &mut Context<Self>) {
        self.close_palette(window, cx);
        let path = self.map_path.clone();
        let edited = |this: &mut Self, r: anyhow::Result<String>, cx: &mut Context<Self>| {
            match r {
                Ok(msg) => this.say(msg, cx),
                Err(e) => this.say(format!("{e:#}"), cx),
            }
            this.reload(false, cx);
        };
        match cmd {
            Cmd::Run(runners) => self.start_run(runners, cx),
            Cmd::RunStale => {
                let r = self.runners_needed(None);
                self.start_run(r, cx)
            }
            Cmd::Import(Some(r)) => self.import_for(r, cx),
            Cmd::Import(None) => self.open_palette_mode(Mode::ImportFor, window, cx),
            Cmd::Trace => self.start_trace(cx),
            Cmd::Go(s) => self.go(s, cx),
            Cmd::Review(id) => {
                let r = edit::review(&path, &id).map(|_| "Marked reviewed".to_string());
                edited(self, r, cx)
            }
            Cmd::Claim(id, files) => {
                let name = self.snap.as_ref().and_then(|s| s.map.cap(&id)).map_or(id.clone(), |c| c.name.clone());
                let r = edit::claim(&path, &id, &files).map(|n| format!("{name} now claims {}", crate::ui::plural(n, "more file", "more files")));
                edited(self, r, cx)
            }
            Cmd::NewCap(title, files) => {
                let r = match edit::split_title(&title) {
                    Some((g, n)) => edit::new_capability(&path, &g, &n, &files, false).map(|id| format!("Added {id}. It needs scenarios.")),
                    None => Err(anyhow::anyhow!("Name it “Group › Name”")),
                };
                edited(self, r, cx)
            }
            Cmd::Pick(mode) => self.open_palette_mode(mode, window, cx),
            Cmd::ToggleTheme => {
                self.theme_forced = true;
                self.theme = if self.theme.dark { Theme::light() } else { Theme::dark() };
                cx.notify();
            }
        }
    }

    pub fn set_since(&mut self, rev: Option<String>, cx: &mut Context<Self>) {
        self.since = Some(rev);
        self.selected = None;
        self.reload(false, cx);
    }

    pub fn open_in_editor(&mut self, line: Option<usize>, cx: &mut Context<Self>) {
        let path = self.map_path.display().to_string();
        let target = format!("{path}:{}", line.unwrap_or(1));
        let ok = std::process::Command::new("code").args(["-g", &target]).spawn().is_ok()
            || std::process::Command::new("xdg-open").arg(&path).spawn().is_ok();
        if !ok {
            self.copy(target, cx);
        }
    }
}

fn reduced_motion() -> bool {
    if let Some(v) = std::env::var_os("PROOF_REDUCED_MOTION") {
        return v != "0";
    }
    std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "enable-animations"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "false")
        .unwrap_or(false)
}
