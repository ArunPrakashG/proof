//! What the evidence says now: each scenario's state, rolled up to its capability.

use std::collections::{BTreeSet, HashMap};

use serde::Serialize;

use crate::git::Tree;
use crate::map::{owns, Map};
use crate::store::{Entry, Store};

/// Ordered worst first: a capability is as good as its worst scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Failing,
    Missing,
    Stale,
    Blocked,
    Unverified,
    Passing,
}

impl State {
    pub const ALL: [State; 6] =
        [State::Failing, State::Missing, State::Stale, State::Blocked, State::Unverified, State::Passing];

    /// The same characters the CLI prints.
    pub fn glyph(self) -> &'static str {
        match self {
            State::Passing => "✓",
            State::Failing => "✗",
            State::Stale => "⚠",
            State::Missing => "?",
            State::Blocked => "⊘",
            State::Unverified => "○",
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            State::Passing => "passing",
            State::Failing => "failing",
            State::Stale => "stale",
            State::Missing => "missing",
            State::Blocked => "blocked",
            State::Unverified => "unverified",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScenarioVerdict {
    pub state: State,
    pub entry: Option<Entry>,
    /// For stale scenarios: the claimed files whose content changed since the evidence.
    pub changed: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Verdict {
    pub state: State,
    pub scenarios: Vec<ScenarioVerdict>,
}

impl Verdict {
    pub fn count(&self, state: State) -> usize {
        self.scenarios.iter().filter(|s| s.state == state).count()
    }

    pub fn passing(&self) -> usize {
        self.count(State::Passing)
    }

    /// Every changed file across stale scenarios.
    pub fn changed(&self) -> BTreeSet<&str> {
        self.scenarios.iter().flat_map(|s| s.changed.iter().map(String::as_str)).collect()
    }

    pub fn blocked_why(&self) -> Option<&str> {
        self.scenarios.iter().find_map(|s| s.entry.as_ref().and_then(|e| e.why.as_deref()).filter(|_| s.state == State::Blocked))
    }
}

/// capability id -> verdict
pub type Verdicts = HashMap<String, Verdict>;

pub fn assess(map: &Map, store: &Store, tree: &Tree) -> Verdicts {
    let paths: Vec<&String> = tree.keys().collect();
    let mut out = Verdicts::new();
    for cap in &map.caps {
        let mut rows = Vec::new();
        for s in &cap.scenarios {
            let Some(entry) = store.results.get(&s.id) else {
                rows.push(ScenarioVerdict { state: State::Unverified, entry: None, changed: vec![] });
                continue;
            };
            let state = match entry.state.as_str() {
                "blocked" => State::Blocked,
                "missing" => State::Missing,
                _ => {
                    let now = owns(&s.patterns, &s.traced, paths.iter().copied());
                    let then = entry.fingerprint.clone().unwrap_or_default();
                    let changed: Vec<String> = now
                        .iter()
                        .chain(then.keys())
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .filter(|p| tree.get(*p).filter(|_| now.contains(*p)) != then.get(*p))
                        .cloned()
                        .collect();
                    if !changed.is_empty() {
                        rows.push(ScenarioVerdict { state: State::Stale, entry: Some(entry.clone()), changed });
                        continue;
                    }
                    if entry.state == "pass" {
                        State::Passing
                    } else {
                        State::Failing
                    }
                }
            };
            rows.push(ScenarioVerdict { state, entry: Some(entry.clone()), changed: vec![] });
        }
        let state = rows.iter().map(|r| r.state).min().unwrap_or(State::Unverified);
        out.insert(cap.id.clone(), Verdict { state, scenarios: rows });
    }
    out
}
