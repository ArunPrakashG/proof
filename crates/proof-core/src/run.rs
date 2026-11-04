//! Running runners and recording what they say, or recording a log made elsewhere.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;

use crate::evidence::{self, Item};
use crate::git::{self, Tree};
use crate::map::{matches, owns, Map, Runner};
use crate::store::{Entry, RunRecord, Store};

#[derive(Debug, Clone)]
pub enum Event {
    Started { runner: String, expected: Option<usize> },
    /// A test finished.
    Item { runner: String, item: Item },
    Blocked { runner: String, why: String },
    Finished { runner: String, record: RunRecord },
}

/// `$HOME/bin:$PATH` with this process's environment.
pub fn expand_vars(value: &str) -> String {
    static VAR: OnceLock<Regex> = OnceLock::new();
    let re = VAR.get_or_init(|| Regex::new(r"\$(?:\{(\w+)\}|(\w+))").unwrap());
    re.replace_all(value, |c: &regex::Captures| {
        let name = c.get(1).or(c.get(2)).unwrap().as_str();
        std::env::var(name).unwrap_or_else(|_| c[0].to_string())
    })
    .into_owned()
}

fn shell(map: &Map, r: &Runner, script: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(script).current_dir(map.runner_cwd(r));
    for (k, v) in &r.env {
        cmd.env(k, expand_vars(v));
    }
    cmd
}

/// Why this runner can't run here, from its `requires` check; None when it can.
pub fn check_requires(map: &Map, r: &Runner) -> Option<String> {
    let req = r.requires.as_deref()?;
    let out = shell(map, r, req).stdin(Stdio::null()).output();
    match out {
        Ok(o) if o.status.success() => None,
        Ok(o) => {
            let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
            Some(Some(text.trim().to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| format!("`{req}` failed")))
        }
        Err(e) => Some(e.to_string()),
    }
}

/// The runtime a runner's command starts, like "Node 24.14", asked in the runner's own cwd and env.
/// None for commands it doesn't know, such as a shell script.
pub fn runtime(map: &Map, r: &Runner) -> Option<String> {
    const KNOWN: &[(&[&str], &str, &str)] = &[
        (&["node", "npm", "npx", "pnpm", "yarn"], "node --version", "Node"),
        (&["bun", "bunx"], "bun --version", "Bun"),
        (&["deno"], "deno --version", "Deno"),
        (&["cargo"], "rustc --version", "Rust"),
        (&["python", "python3", "pytest", "uv"], "python3 --version", "Python"),
        (&["go"], "go version", "Go"),
    ];
    static VERSION: OnceLock<Regex> = OnceLock::new();
    let program = r.run.split_whitespace().find(|w| !w.contains('='))?;
    let program = program.rsplit('/').next().unwrap_or(program);
    let (_, ask, label) = KNOWN.iter().find(|(names, _, _)| names.contains(&program))?;
    let out = shell(map, r, ask).stdin(Stdio::null()).output().ok().filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&out.stdout);
    let v = VERSION.get_or_init(|| Regex::new(r"(\d+)\.(\d+)").unwrap()).captures(&text)?;
    Some(format!("{label} {}.{}", &v[1], &v[2]))
}

/// Run one runner, streaming events as tests finish, and record the results.
/// Setting `stop` ends the run early; nothing is recorded for a stopped run.
pub fn run_runner(map: &Map, name: &str, stop: Arc<AtomicBool>, on: &mut dyn FnMut(Event)) -> Result<RunRecord> {
    let r = map.runner(name).ok_or_else(|| anyhow!("no runner called {name:?}"))?;
    let head = git::head(&map.root)?;
    let commit = head.commit_label();
    let tree = git::blobs(&map.root, None)?;
    let previous = Store::load(&map.store_path)?.runs.get(name).map(|r| r.tests).filter(|n| *n > 0);

    if let Some(why) = check_requires(map, r) {
        on(Event::Blocked { runner: name.into(), why: why.clone() });
        let record = record(map, name, "", 1, &commit, &tree, "local", Some(&why), None, None)?;
        on(Event::Finished { runner: name.into(), record: record.clone() });
        return Ok(record);
    }

    on(Event::Started { runner: name.into(), expected: previous });
    std::fs::create_dir_all(&map.logs_dir)?;
    let stamp = time::OffsetDateTime::now_utc();
    let log_path = map.logs_dir.join(format!(
        "{name}-{:04}{:02}{:02}-{:02}{:02}{:02}.log",
        stamp.year(),
        stamp.month() as u8,
        stamp.day(),
        stamp.hour(),
        stamp.minute(),
        stamp.second()
    ));
    let mut log = std::fs::File::create(&log_path)?;

    let started = Instant::now();
    let mut child = shell(map, r, &format!("exec 2>&1; {}", r.run))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .process_group(0)
        .spawn()
        .with_context(|| format!("starting {name}"))?;
    let pgid = child.id() as i32;
    let done = Arc::new(AtomicBool::new(false));
    let killed = Arc::new(AtomicBool::new(false));
    let timeout = Duration::from_secs(r.timeout_secs);
    let watchdog = {
        let (done, killed, stop) = (done.clone(), killed.clone(), stop.clone());
        std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                if stop.load(Ordering::Relaxed) || started.elapsed() > timeout {
                    killed.store(true, Ordering::Relaxed);
                    let kill = |sig: &str| {
                        let _ = Command::new("kill").arg(sig).arg(format!("-{pgid}")).stderr(Stdio::null()).status();
                    };
                    kill("-TERM");
                    // give it two seconds to exit cleanly before forcing it
                    for _ in 0..20 {
                        if done.load(Ordering::Relaxed) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    kill("-KILL");
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        })
    };

    let mut text = String::new();
    let mut pending_step: Option<String> = None;
    let stdout = child.stdout.take().unwrap();
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        let _ = writeln!(log, "{line}");
        if let Some(item) = evidence::parse_line(&r.format, &line) {
            if r.format == "steps" {
                // a step passed once the next one starts
                if let Some(prev) = pending_step.replace(item.name.clone()) {
                    on(Event::Item { runner: name.into(), item: Item { name: prev, ok: true } });
                }
            } else {
                on(Event::Item { runner: name.into(), item });
            }
        }
        text.push_str(&line);
        text.push('\n');
    }
    let status = child.wait()?;
    done.store(true, Ordering::Relaxed);
    let _ = watchdog.join();
    if killed.load(Ordering::Relaxed) && stop.load(Ordering::Relaxed) {
        bail!("stopped");
    }
    let code = if killed.load(Ordering::Relaxed) { 124 } else { status.code().unwrap_or(1) };
    if let Some(step) = pending_step {
        on(Event::Item { runner: name.into(), item: Item { name: step, ok: code == 0 } });
    }
    let secs = started.elapsed().as_secs();
    let record = record(map, name, &text, code, &commit, &tree, "local", None, Some(secs), Some(&log_path))?;
    on(Event::Finished { runner: name.into(), record: record.clone() });
    Ok(record)
}

/// Record a runner's output against the code it ran on.
#[allow(clippy::too_many_arguments)]
pub fn record(
    map: &Map,
    runner: &str,
    text: &str,
    code: i32,
    commit: &str,
    tree: &Tree,
    source: &str,
    blocked: Option<&str>,
    secs: Option<u64>,
    log: Option<&Path>,
) -> Result<RunRecord> {
    let r = map.runner(runner).ok_or_else(|| anyhow!("no runner called {runner:?}"))?;
    let mut store = Store::load(&map.store_path)?;
    let (items, partial) = if blocked.is_some() { (vec![], false) } else { evidence::parse(&r.format, text, code) };
    let at = crate::now_rfc3339();
    let paths: Vec<&String> = tree.keys().collect();
    for cap in &map.caps {
        for s in cap.scenarios.iter().filter(|s| s.runner == runner) {
            let base = Entry {
                at: at.clone(),
                commit: commit.into(),
                source: source.into(),
                state: String::new(),
                fingerprint: None,
                why: None,
            };
            let entry = if let Some(why) = blocked {
                // a run that could not happen does not erase what is known
                if store.results.contains_key(&s.id) {
                    continue;
                }
                Entry { state: "blocked".into(), why: Some(why.into()), ..base }
            } else {
                let hits: Vec<bool> = items.iter().filter(|i| matches(&s.matcher, &i.name)).map(|i| i.ok).collect();
                if hits.is_empty() && partial {
                    continue; // the run stopped before reaching it
                }
                let state = if hits.is_empty() {
                    "missing"
                } else if hits.iter().all(|ok| *ok) {
                    "pass"
                } else {
                    "fail"
                };
                let fp: BTreeMap<String, String> =
                    owns(&s.patterns, &s.traced, paths.iter().copied())
                    .into_iter()
                    .map(|p| {
                        let id = tree[&p].clone();
                        (p, id)
                    })
                    .collect();
                Entry { state: state.into(), fingerprint: Some(fp), ..base }
            };
            store.results.insert(s.id.clone(), entry);
        }
    }
    let record = RunRecord {
        at,
        commit: commit.into(),
        source: source.into(),
        tests: items.len(),
        passed: items.iter().filter(|i| i.ok).count(),
        failed: items.iter().filter(|i| !i.ok).count(),
        blocked: blocked.map(String::from),
        secs,
        log: log.map(|p| p.display().to_string()),
    };
    // a blocked run keeps the last real run's record, with why it couldn't run now
    let keep = match (blocked, store.runs.get(runner)) {
        (Some(why), Some(prev)) if prev.blocked.is_none() => RunRecord { blocked: Some(why.into()), ..prev.clone() },
        _ => record.clone(),
    };
    store.runs.insert(runner.into(), keep.clone());
    store.save(&map.store_path)?;
    Ok(keep)
}

/// Commits a CI log says it checked out, most telling first. The runner image's own `Commit:` line
/// is not one of them.
pub fn commits_in_log(text: &str) -> Vec<String> {
    static RE: OnceLock<[Regex; 3]> = OnceLock::new();
    let res = RE.get_or_init(|| {
        [
            // actions/checkout fetches the exact commit
            Regex::new(r"fetch .* origin \+([0-9a-f]{40}):").unwrap(),
            Regex::new(r"HEAD is now at ([0-9a-f]{7,40})\b").unwrap(),
            Regex::new(r"Checking out commit ([0-9a-f]{7,40})\b").unwrap(),
        ]
    });
    let mut out: Vec<String> = Vec::new();
    for re in res {
        for c in re.captures_iter(text) {
            if !out.contains(&c[1].to_string()) {
                out.push(c[1].to_string());
            }
        }
    }
    out
}

/// The exit code a CI log reports for its failing step, if it reports one.
pub fn exit_in_log(text: &str) -> Option<i32> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"Process completed with exit code (\d+)").unwrap());
    re.captures_iter(text).filter_map(|c| c[1].parse().ok()).find(|c| *c != 0)
}

pub struct Imported {
    pub commit: String,
    pub record: RunRecord,
}

/// Record a log made elsewhere (CI) against the tree of the commit it ran on.
pub fn import_log(map: &Map, runner: &str, log: &Path, commit: Option<&str>, exit: i32, source: Option<&str>) -> Result<Imported> {
    let text = String::from_utf8_lossy(&std::fs::read(log).with_context(|| format!("reading {}", log.display()))?).into_owned();
    let exit = if exit == 0 { exit_in_log(&text).unwrap_or(0) } else { exit };
    let rev = match commit {
        Some(c) => c.to_string(),
        None => {
            let found = commits_in_log(&text);
            match found.iter().find(|c| git::exists(&map.root, c)) {
                Some(c) => c.clone(),
                None if found.is_empty() => "HEAD".into(),
                None => bail!(
                    "The log ran on {}, which isn't in this clone. Fetch it, or import with --commit and the commit it tested.",
                    &found[0][..7]
                ),
            }
        }
    };
    let r = map.runner(runner).ok_or_else(|| anyhow!("no runner called {runner:?}"))?;
    if !matches!(r.format.as_str(), "exit") && evidence::parse(&r.format, &text, exit).0.is_empty() {
        bail!("No {} test results in {}. Is it the {runner} runner's log?", r.format, log.display());
    }
    let tree = git::blobs(&map.root, Some(&rev))?;
    let sha = git::short(&map.root, &rev)?;
    let source = source.map(String::from).unwrap_or_else(|| {
        if text.contains("Current runner version") || text.contains("##[group]") {
            "ci".into()
        } else {
            "imported".into()
        }
    });
    let record = record(map, runner, &text, exit, &sha, &tree, &source, None, None, Some(log))?;
    Ok(Imported { commit: sha, record })
}

/// Runners the given capabilities' scenarios need, in map order.
pub fn runners_for<'a>(map: &'a Map, caps: &[&crate::Capability]) -> Vec<&'a Runner> {
    map.runners.iter().filter(|r| caps.iter().any(|c| c.scenarios.iter().any(|s| s.runner == r.name))).collect()
}
