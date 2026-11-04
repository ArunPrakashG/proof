//! Recorded results, in `.proof/<map>.json` beside the map.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub at: String,
    pub commit: String,
    /// `local`, or where imported evidence came from (`ci run 38215980266`).
    pub source: String,
    /// pass, fail, missing or blocked
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// The last time a runner ran or was imported.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct RunRecord {
    pub at: String,
    pub commit: String,
    pub source: String,
    /// Tests the run reported, passed or not: the denominator for the next run's progress.
    pub tests: usize,
    pub passed: usize,
    pub failed: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Store {
    /// scenario id -> its latest evidence
    #[serde(default)]
    pub results: BTreeMap<String, Entry>,
    /// runner name -> its latest run
    #[serde(default)]
    pub runs: BTreeMap<String, RunRecord>,
}

impl Store {
    pub fn load(path: &Path) -> Result<Store> {
        let Ok(text) = std::fs::read_to_string(path) else { return Ok(Store::default()) };
        let value: serde_json::Value = serde_json::from_str(&text)?;
        // the prototype stored the results map alone
        if value.get("results").is_none() && value.get("runs").is_none() {
            return Ok(Store { results: serde_json::from_value(value)?, runs: BTreeMap::new() });
        }
        Ok(serde_json::from_value(value)?)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)? + "\n")?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}
