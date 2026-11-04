//! `proof trace`: each test alone under V8 coverage; the files it executed become its claims.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};
use regex::Regex;
use serde::Serialize;

use crate::git;
use crate::map::{glob, Map, Runner};
use crate::run::expand_vars;

#[derive(Debug, Default, Serialize)]
pub struct TraceReport {
    pub tests: usize,
    /// Tests that couldn't be run on their own.
    pub alone_failed: Vec<String>,
}

fn node(map: &Map, r: &Runner) -> Command {
    let mut cmd = Command::new("node");
    cmd.current_dir(map.runner_cwd(r)).stdin(Stdio::null());
    for (k, v) in &r.env {
        cmd.env(k, expand_vars(v));
    }
    cmd
}

fn tap_names(stdout: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?m)^(?:not )?ok \d+ - (.*?)(?:\s+# (?:SKIP|TODO).*)?$").unwrap());
    re.captures_iter(stdout).map(|c| c[1].to_string()).collect()
}

fn test_files(dir: &Path, pattern: &str) -> Vec<PathBuf> {
    let Ok(g) = glob(pattern) else { return vec![] };
    let m = g.compile_matcher();
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name();
            if name == "node_modules" || name == ".git" {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(dir) {
                if m.is_match(rel) {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

/// Source files under `root` in which some function other than the module body ran.
fn executed(cov_dir: &Path, root: &Path) -> BTreeSet<String> {
    let prefix = format!("file://{}/", root.display());
    let mut out = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(cov_dir) else { return out };
    for e in entries.flatten() {
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        for script in v["result"].as_array().into_iter().flatten() {
            let Some(rel) = script["url"].as_str().and_then(|u| u.strip_prefix(&prefix)) else { continue };
            let ran = script["functions"].as_array().into_iter().flatten().any(|f| {
                let r0 = &f["ranges"][0];
                let body = f["functionName"].as_str() == Some("") && r0["startOffset"].as_u64() == Some(0);
                r0["count"].as_u64().unwrap_or(0) > 0 && !body
            });
            if ran {
                out.insert(rel.to_string());
            }
        }
    }
    out
}

pub fn trace(map: &Map, jobs: usize, progress: &(dyn Fn(usize, usize) + Sync)) -> Result<TraceReport> {
    std::fs::create_dir_all(map.store_path.parent().unwrap())?;
    let mut work: Vec<(&Runner, PathBuf, String)> = Vec::new();
    for r in map.runners.iter().filter(|r| r.traced()) {
        let cwd = map.runner_cwd(r);
        for f in test_files(&cwd, r.tests.as_deref().unwrap_or("**/*.test.*")) {
            let out = node(map, r).args(["--test", "--test-reporter=tap"]).arg(&f).output().context("running node")?;
            for name in tap_names(&String::from_utf8_lossy(&out.stdout)) {
                work.push((r, f.clone(), name));
            }
        }
    }
    let total = work.len();
    let results: Mutex<Vec<Option<(bool, BTreeSet<String>)>>> = Mutex::new(vec![None; total]);
    let next = Mutex::new(0usize);
    let done = Mutex::new(0usize);
    std::thread::scope(|s| {
        for _ in 0..jobs.max(1) {
            s.spawn(|| loop {
                let i = {
                    let mut n = next.lock().unwrap();
                    if *n >= total {
                        return;
                    }
                    *n += 1;
                    *n - 1
                };
                let (r, file, name) = &work[i];
                let out = trace_one(map, r, file, name);
                results.lock().unwrap()[i] = Some(out);
                let mut d = done.lock().unwrap();
                *d += 1;
                progress(*d, total);
            });
        }
    });
    let mut runs: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    let mut report = TraceReport { tests: total, ..Default::default() };
    for ((r, _, name), res) in work.iter().zip(results.into_inner().unwrap()) {
        let (ran, files) = res.unwrap_or_default();
        if !ran {
            report.alone_failed.push(name.clone());
        }
        runs.entry(r.name.clone()).or_default().insert(name.clone(), files.into_iter().collect());
    }
    let commit = git::head(&map.root)?.sha;
    let body = serde_json::json!({ "commit": commit, "runs": runs });
    std::fs::write(&map.traces_path, serde_json::to_string_pretty(&body)? + "\n")?;
    Ok(report)
}

fn trace_one(map: &Map, r: &Runner, file: &Path, name: &str) -> (bool, BTreeSet<String>) {
    let cov = map.store_path.parent().unwrap().join(format!("cov-{}-{}", std::process::id(), rand_suffix()));
    let _ = std::fs::create_dir_all(&cov);
    let out = node(map, r)
        .env("NODE_V8_COVERAGE", &cov)
        .args(["--test", "--test-reporter=tap"])
        .arg(format!("--test-name-pattern=^{}$", regex::escape(name)))
        .arg(file)
        .output();
    let ran = out.map(|o| tap_names(&String::from_utf8_lossy(&o.stdout)).iter().any(|n| n == name)).unwrap_or(false);
    let files = executed(&cov, &map.root).into_iter().filter(|f| map.is_source(f)).collect();
    let _ = std::fs::remove_dir_all(&cov);
    (ran, files)
}

fn rand_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}
