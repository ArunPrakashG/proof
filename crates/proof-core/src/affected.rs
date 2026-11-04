//! What a change touches: capabilities hit directly, ones whose tests run the changed code,
//! and changed source no capability owns.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use crate::git;
use crate::map::{owns, Map};
use crate::store::Store;

#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub cap: String,
    /// Changed files the capability depends on.
    pub files: Vec<String>,
    /// The subset that hits it directly; empty means it only runs the changed code.
    pub direct: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Affected {
    /// The base the change is measured from; None means HEAD (uncommitted changes only).
    pub since: Option<String>,
    pub commits: usize,
    pub files: Vec<String>,
    /// Changed files inside the map's `source`.
    pub source: Vec<String>,
    pub hits: Vec<Hit>,
    /// Changed source no capability claims and no test runs.
    pub unmapped: Vec<String>,
    /// Changed files tests run only along the way: likely a capability the map doesn't have yet.
    pub incidental: Vec<String>,
    /// The share of all tests that run the incidental files, at most.
    pub incidental_reach: f64,
    /// path -> (added, removed)
    pub lines: HashMap<String, (u32, u32)>,
}

impl Affected {
    pub fn direct(&self) -> impl Iterator<Item = &Hit> {
        self.hits.iter().filter(|h| !h.direct.is_empty())
    }

    pub fn indirect(&self) -> impl Iterator<Item = &Hit> {
        self.hits.iter().filter(|h| h.direct.is_empty())
    }

    /// Changed files that are neither source nor claimed: docs, the website, tests.
    pub fn outside(&self) -> usize {
        let claimed: BTreeSet<&String> = self.hits.iter().flat_map(|h| h.files.iter()).collect();
        self.files.iter().filter(|f| !self.source.contains(f) && !claimed.contains(f)).count()
    }
}

pub fn affected(map: &Map, since: Option<&str>) -> Result<Affected> {
    let root = &map.root;
    let files = git::changed_files(root, since)?;
    let mut hits = Vec::new();
    let mut mapped = BTreeSet::new();
    for cap in &map.caps {
        let mine = owns(&cap.patterns, &cap.traced, files.iter());
        if mine.is_empty() {
            continue;
        }
        let direct = map.direct(cap, mine.iter());
        mapped.extend(mine.iter().cloned());
        hits.push(Hit { cap: cap.id.clone(), files: mine.into_iter().collect(), direct: direct.into_iter().collect() });
    }
    let owned: BTreeSet<&String> = hits.iter().flat_map(|h| h.direct.iter()).collect();
    let unmapped: Vec<String> = files.iter().filter(|f| !mapped.contains(*f) && map.is_source(f)).cloned().collect();
    let incidental: Vec<String> = files
        .iter()
        .filter(|f| mapped.contains(*f) && !owned.contains(f) && map.reach(f) <= map.narrow * 2.0)
        .cloned()
        .collect();
    let incidental_reach = incidental.iter().map(|f| map.reach(f)).fold(0.0, f64::max);
    let source: Vec<String> = files.iter().filter(|f| map.is_source(f)).cloned().collect();
    let shown: Vec<String> = unmapped.iter().chain(&incidental).cloned().collect();
    let lines = git::numstat(root, since.unwrap_or("HEAD"), &shown)?;
    Ok(Affected {
        since: since.map(String::from),
        commits: since.map(|s| git::count_commits(root, s)).unwrap_or(0),
        files,
        source,
        hits,
        unmapped,
        incidental,
        incidental_reach,
        lines,
    })
}

/// A base to measure changes from, as the Changes view offers them.
#[derive(Debug, Clone, Serialize)]
pub struct Since {
    /// None: the working tree against HEAD.
    pub rev: Option<String>,
    pub label: String,
    pub note: String,
}

/// Uncommitted changes, the last commit, and the commit of the newest imported evidence.
pub fn since_choices(map: &Map, store: &Store, chosen: Option<&str>) -> Vec<Since> {
    let root: &Path = &map.root;
    let mut out = Vec::new();
    let add = |rev: String, note: String, out: &mut Vec<Since>| {
        let Ok(sha) = git::short(root, &rev) else { return };
        if out.iter().any(|s: &Since| s.rev.as_deref() == Some(sha.as_str())) {
            return;
        }
        out.push(Since { rev: Some(sha.clone()), label: sha, note });
    };
    let ago = |rev: &str| match git::count_commits(root, rev) {
        0 => "HEAD".to_string(),
        1 => "1 commit ago".to_string(),
        n => format!("{n} commits ago"),
    };
    if let Some(c) = chosen {
        add(c.to_string(), ago(c), &mut out);
    }
    if git::exists(root, "HEAD~1") {
        add("HEAD~1".into(), "1 commit ago".into(), &mut out);
    }
    let ci = store
        .results
        .values()
        .filter(|e| e.source != "local")
        .max_by(|a, b| a.at.cmp(&b.at))
        .map(|e| e.commit.trim_end_matches("+dirty").to_string());
    if let Some(ci) = ci.filter(|c| git::exists(root, c)) {
        add(ci, "last CI run".into(), &mut out);
    }
    out.push(Since { rev: None, label: "Uncommitted".into(), note: "working tree".into() });
    out
}
