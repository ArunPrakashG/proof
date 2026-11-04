//! Everything the screens show, computed off the UI thread in one go.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use proof_core::affected::{since_choices, Since};
use proof_core::git::{self, Head};
use proof_core::run::{check_requires, runtime};
use proof_core::{affected, assess, Affected, Map, MapError, State, Store, Verdicts};

pub struct Snapshot {
    pub map: Map,
    pub store: Store,
    pub verdicts: Verdicts,
    pub head: Head,
    pub affected: Affected,
    pub since_choices: Vec<Since>,
    /// runner -> why it can't run on this machine now
    pub blocked_now: HashMap<String, String>,
    /// runner -> the runtime its command starts here, like "Node 24.14"
    pub runtimes: HashMap<String, String>,
    /// (commit, path) -> lines (added, removed) from that commit to now, for stale scenarios
    pub stale_lines: HashMap<(String, String), (u32, u32)>,
    /// (commit, a stale scenario's changed files, sorted) -> the newest commit since then that touched them
    pub latest: HashMap<(String, Vec<String>), (String, String)>,
    /// short sha -> subject, for commits named on screen
    pub subjects: HashMap<String, String>,
    /// how many files are source
    pub source: usize,
    /// source files no capability claims, sorted
    pub unclaimed: Vec<String>,
}

impl Snapshot {
    pub fn load(path: &Path, since: Option<Option<String>>, check_runners: bool, previous: Option<&Snapshot>) -> Result<Snapshot, MapError> {
        let map = Map::load(path)?;
        let other = |e: anyhow::Error| MapError::Invalid { path: path.to_path_buf(), line: None, message: format!("{e:#}") };
        let store = Store::load(&map.store_path).map_err(other)?;
        let tree = git::blobs(&map.root, None).map_err(other)?;
        let verdicts = assess(&map, &store, &tree);
        let (source, unclaimed) = map.unclaimed(tree.keys());
        let head = git::head(&map.root).map_err(other)?;
        let since = since.unwrap_or_else(|| default_since(&map, &head));
        let affected = affected::affected(&map, since.as_deref()).map_err(other)?;
        let since_choices = since_choices(&map, &store, since.as_deref());

        let (blocked_now, runtimes) = match previous {
            Some(p) if !check_runners => (p.blocked_now.clone(), p.runtimes.clone()),
            _ => (
                map.runners.iter().filter_map(|r| check_requires(&map, r).map(|why| (r.name.clone(), why))).collect(),
                map.runners.iter().filter_map(|r| runtime(&map, r).map(|v| (r.name.clone(), v))).collect(),
            ),
        };

        // what changed under each stale scenario, measured from the commit its evidence came from
        let mut by_commit: HashMap<String, Vec<String>> = HashMap::new();
        let mut per_scenario = std::collections::BTreeSet::new();
        for v in verdicts.values() {
            for s in v.scenarios.iter().filter(|s| s.state == State::Stale) {
                let commit = s.entry.as_ref().map(|e| e.commit.trim_end_matches("+dirty").to_string()).unwrap_or_default();
                by_commit.entry(commit.clone()).or_default().extend(s.changed.iter().cloned());
                per_scenario.insert(latest_key(&commit, &s.changed));
            }
        }
        let mut stale_lines = HashMap::new();
        for (commit, mut files) in by_commit {
            files.sort();
            files.dedup();
            if let Ok(lines) = git::numstat(&map.root, &commit, &files) {
                stale_lines.extend(lines.into_iter().map(|(p, l)| ((commit.clone(), p), l)));
            }
        }
        // per scenario, so its "Latest" commit touched its own files, not another scenario's
        let mut latest = HashMap::new();
        for key in per_scenario {
            if let Some(l) = git::latest_touching(&map.root, &key.0, &key.1) {
                latest.insert(key, l);
            }
        }
        let mut subjects = HashMap::new();
        if !affected.incidental.is_empty() || !affected.unmapped.is_empty() {
            let files: Vec<String> = affected.incidental.iter().chain(&affected.unmapped).cloned().collect();
            if let Some((sha, subject)) = since.as_deref().and_then(|s| git::latest_touching(&map.root, s, &files)) {
                subjects.insert(sha, subject);
            }
        }

        Ok(Snapshot { map, store, verdicts, head, affected, since_choices, blocked_now, runtimes, stale_lines, latest, subjects, source, unclaimed })
    }

    pub fn total(&self, state: State) -> usize {
        self.verdicts.values().map(|v| v.count(state)).sum()
    }

    pub fn caps_in(&self, state: State) -> usize {
        self.verdicts.values().filter(|v| v.state == state).count()
    }

    pub fn state(&self, cap_id: &str) -> State {
        self.verdicts.get(cap_id).map(|v| v.state).unwrap_or(State::Unverified)
    }

}

/// The key into `Snapshot::latest` for a stale scenario.
pub fn latest_key(commit: &str, changed: &[String]) -> (String, Vec<String>) {
    let mut files = changed.to_vec();
    files.sort();
    (commit.to_string(), files)
}

/// Uncommitted changes when there are any, otherwise the last commit.
fn default_since(map: &Map, head: &Head) -> Option<String> {
    if head.dirty || !git::changed_files(&map.root, None).unwrap_or_default().is_empty() {
        return None;
    }
    git::short(&map.root, "HEAD~1").ok()
}

pub fn dir_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// Cheap to compute; when it changes, the snapshot is stale.
pub fn signature(map_path: &Path, root: Option<&Path>) -> String {
    let mtime = |p: PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).map(|t| format!("{t:?}")).unwrap_or_default();
    let dir = map_path.parent().unwrap_or(Path::new("."));
    let stem = map_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut sig = format!(
        "{}|{}|{}",
        mtime(map_path.to_path_buf()),
        mtime(dir.join(".proof").join(format!("{stem}.json"))),
        mtime(dir.join(".proof").join(format!("{stem}.traces.json")))
    );
    if let Some(root) = root {
        sig.push_str(&git::git(root, &["rev-parse", "HEAD"]).unwrap_or_default());
        let status = git::git(root, &["status", "--porcelain", "-z"]).unwrap_or_default();
        // a file already modified stays " M" as it changes again: its mtime and size tell
        for rec in status.split('\0').filter(|r| r.len() > 3) {
            let meta = std::fs::metadata(root.join(&rec[3..]));
            let stamp = meta.map(|m| format!("{:?}{}", m.modified().ok(), m.len())).unwrap_or_default();
            sig.push_str(rec);
            sig.push_str(&stamp);
        }
    }
    sig
}
