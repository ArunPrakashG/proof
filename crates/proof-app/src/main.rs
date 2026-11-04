//! proof, the desktop app: the map, what changed, and the runners, for a person to read.

mod app;
mod palette;
mod snapshot;
mod theme;
mod ui;
mod views;

use std::borrow::Cow;
use std::path::PathBuf;

use gpui::{px, size, App, AppContext, Application, AssetSource, Bounds, SharedString, TitlebarOptions, WindowBounds, WindowOptions};
use proof_core::Map;

use crate::app::{bind_keys, Options, ProofApp};

struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        let bytes: &'static [u8] = match path {
            "icons/mark-frame.svg" => include_bytes!("../assets/icons/mark-frame.svg"),
            "icons/mark-check.svg" => include_bytes!("../assets/icons/mark-check.svg"),
            "icons/search.svg" => include_bytes!("../assets/icons/search.svg"),
            "icons/terminal.svg" => include_bytes!("../assets/icons/terminal.svg"),
            "icons/copy.svg" => include_bytes!("../assets/icons/copy.svg"),
            "icons/arc.svg" => include_bytes!("../assets/icons/arc.svg"),
            "icons/ring.svg" => include_bytes!("../assets/icons/ring.svg"),
            _ => return Ok(None),
        };
        Ok(Some(Cow::Borrowed(bytes)))
    }

    fn list(&self, _: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(vec![])
    }
}

const USAGE: &str = "usage: proof-app [--map proof.toml] [--since REV|uncommitted] [--theme dark|light]
                 [--screen map|changes|runners|<capability id>] [--palette QUERY] [--size WxH]";

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut map, mut since, mut theme, mut screen, mut palette, mut win) = (None, None, None, None, None, (1440., 900.));
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| exit(&format!("{a} needs a value")));
        match a.as_str() {
            "--map" => map = Some(PathBuf::from(val())),
            "--since" => since = Some(val()),
            "--theme" => theme = Some(val()),
            "--screen" => screen = Some(val()),
            "--palette" => palette = Some(val()),
            "--size" => {
                let v = val();
                let parsed = v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
                win = parsed.unwrap_or_else(|| exit("--size is WIDTHxHEIGHT, like 1440x900"));
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            _ => exit(&format!("unknown argument {a}")),
        }
    }
    let opts = Options { map: Map::locate(map.as_deref()), since, theme, screen, palette };

    Application::new().with_assets(Assets).run(move |cx: &mut App| {
        cx.text_system()
            .add_fonts(vec![
                Cow::Borrowed(include_bytes!("../assets/fonts/Geist.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../assets/fonts/GeistMono.ttf").as_slice()),
            ])
            .expect("fonts");
        bind_keys(cx);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(win.0), px(win.1)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(360.), px(480.))),
            titlebar: Some(TitlebarOptions { title: Some("proof".into()), ..Default::default() }),
            app_id: Some("proof".into()),
            ..Default::default()
        };
        cx.open_window(options, |window, cx| cx.new(|cx| ProofApp::new(opts, window, cx))).expect("window");
        cx.activate(true);
    });
}

fn exit(msg: &str) -> ! {
    eprintln!("proof-app: {msg}\n{USAGE}");
    std::process::exit(2)
}
