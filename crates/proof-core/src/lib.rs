//! proof: a living map of what an application does, and whether it still does it.
//!
//! The map (`proof.toml`) lists capabilities in human terms. Each capability claims the code it
//! depends on (globs) and has scenarios, each backed by evidence from a test runner the project
//! already has. A result is pinned to the git blob ids of the code it covered, so when that code
//! changes the result goes stale on its own.

pub mod affected;
pub mod assess;
pub mod edit;
pub mod evidence;
pub mod git;
pub mod map;
pub mod run;
pub mod store;
pub mod trace;

pub use affected::{affected, Affected, Hit, Since};
pub use assess::{assess, State, Verdict, Verdicts};
pub use map::{Capability, Map, MapError, Runner, Scenario};
pub use store::{Entry, Store};

/// `Story runs`, `Cheating is refused` -> `story-runs/cheating-is-refused`
pub fn slug(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|p| {
            let mut out = String::new();
            let mut dash = false;
            for ch in p.to_lowercase().chars() {
                if ch.is_ascii_alphanumeric() {
                    out.push(ch);
                    dash = false;
                } else if !dash {
                    out.push('-');
                    dash = true;
                }
            }
            out.trim_matches('-').to_string()
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The last path segment, for compact lists of changed files.
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `ci run 38215980266` -> `CI`; `local` stays `local`.
pub fn source_label(source: &str) -> &str {
    if source == "local" {
        "local"
    } else if source.to_lowercase().starts_with("ci") {
        "CI"
    } else {
        source
    }
}

pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}
