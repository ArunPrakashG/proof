//! The map: `proof.toml`, plus the traces `proof trace` recorded next to it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};

use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Deserialize;
use toml::Spanned;

use crate::slug;

#[derive(Debug, Clone)]
pub enum MapError {
    /// There is no map yet: the empty state, not an error to shout about.
    NotFound(PathBuf),
    /// The map exists but cannot be used. `line` is 1-based when known.
    Invalid { path: PathBuf, line: Option<usize>, message: String },
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MapError::NotFound(p) => write!(f, "no map at {}", p.display()),
            MapError::Invalid { path, line: Some(line), message } => {
                write!(f, "{} line {line}: {message}", file_label(path))
            }
            MapError::Invalid { path, line: None, message } => write!(f, "{}: {message}", file_label(path)),
        }
    }
}

impl std::error::Error for MapError {}

fn file_label(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

#[derive(Debug, Clone)]
pub struct Runner {
    pub name: String,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    pub requires: Option<String>,
    pub run: String,
    /// tap, cargo, steps or exit
    pub format: String,
    /// `node`: `proof trace` can run each test alone under V8 coverage.
    pub coverage: Option<String>,
    /// Test files, for tracing.
    pub tests: Option<String>,
    pub timeout_secs: u64,
}

impl Runner {
    pub fn traced(&self) -> bool {
        self.coverage.as_deref() == Some("node")
    }
}

#[derive(Debug, Clone)]
pub struct Scenario {
    pub id: String,
    pub name: String,
    pub runner: String,
    /// `*` for every test, `=name` for an exact name, otherwise a case-insensitive substring.
    pub matcher: String,
    pub patterns: GlobSet,
    /// Source files this scenario's tests executed, from `proof trace`.
    pub traced: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct Capability {
    pub id: String,
    pub group: String,
    pub group_id: String,
    pub name: String,
    pub inferred: bool,
    /// The globs as written, the "named" claims.
    pub code: Vec<String>,
    pub patterns: GlobSet,
    pub scenarios: Vec<Scenario>,
    pub traced: BTreeSet<String>,
    /// 1-based line of the `[[capability]]` header, for "open at line".
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct Map {
    pub path: PathBuf,
    pub name: String,
    pub root: PathBuf,
    pub store_path: PathBuf,
    pub traces_path: PathBuf,
    pub logs_dir: PathBuf,
    pub runners: Vec<Runner>,
    pub caps: Vec<Capability>,
    /// The share of all traced tests that execute a file.
    pub reach: HashMap<String, f64>,
    /// A file run by at most this share of tests is owned by the capabilities whose tests run it.
    pub narrow: f64,
    pub trace_commit: Option<String>,
    source: GlobSet,
    ignore: GlobSet,
}

#[derive(Deserialize)]
struct RawMap {
    name: Option<String>,
    root: Option<String>,
    #[serde(default)]
    source: Vec<String>,
    #[serde(default)]
    ignore: Vec<String>,
    narrow_below: Option<f64>,
    #[serde(default)]
    runners: toml::Table,
    #[serde(default)]
    capability: Vec<Spanned<RawCap>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRunner {
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    requires: Option<String>,
    run: String,
    format: Option<String>,
    coverage: Option<String>,
    tests: Option<String>,
    timeout: Option<u64>,
}

#[derive(Deserialize)]
struct RawCap {
    group: String,
    name: String,
    #[serde(default)]
    inferred: bool,
    #[serde(default)]
    code: Vec<String>,
    #[serde(default)]
    scenario: Vec<RawScenario>,
}

#[derive(Deserialize)]
struct RawScenario {
    name: String,
    runner: Spanned<String>,
    #[serde(rename = "match")]
    matcher: String,
    #[serde(default)]
    test: Vec<String>,
}

#[derive(Deserialize, Default)]
struct RawTraces {
    commit: Option<String>,
    #[serde(default)]
    runs: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

const FORMATS: [&str; 4] = ["tap", "cargo", "steps", "exit"];

pub fn line_of(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].matches('\n').count() + 1
}

pub(crate) fn glob(pattern: &str) -> Result<Glob, globset::Error> {
    GlobBuilder::new(pattern).literal_separator(true).backslash_escape(true).build()
}

fn globset(patterns: &[String]) -> Result<GlobSet, String> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        b.add(glob(p).map_err(|e| format!("bad glob {p:?}: {e}"))?);
    }
    b.build().map_err(|e| e.to_string())
}

pub fn matches(matcher: &str, name: &str) -> bool {
    if matcher == "*" {
        return true;
    }
    if let Some(exact) = matcher.strip_prefix('=') {
        return name == exact;
    }
    name.to_lowercase().contains(&matcher.to_lowercase())
}

impl Map {
    /// The map at `path`, or `proof.toml` found by walking up from the current directory.
    pub fn locate(path: Option<&Path>) -> PathBuf {
        if let Some(p) = path {
            return p.to_path_buf();
        }
        if let Ok(env) = std::env::var("PROOF_MAP") {
            return PathBuf::from(env);
        }
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        for dir in cwd.ancestors() {
            let candidate = dir.join("proof.toml");
            if candidate.is_file() {
                return candidate;
            }
            if dir.join(".git").exists() {
                break;
            }
        }
        cwd.join("proof.toml")
    }

    pub fn load(path: &Path) -> Result<Map, MapError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(MapError::NotFound(path.to_path_buf())),
            Err(e) => return Err(invalid(path, None, e.to_string())),
        };
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let raw: RawMap = toml::from_str(&text).map_err(|e| {
            let line = e.span().map(|s| line_of(&text, s.start));
            invalid(&path, line, e.message().trim().to_string())
        })?;
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "proof".into());
        let root = dir.join(raw.root.as_deref().unwrap_or("."));
        let root = root.canonicalize().map_err(|_| {
            let line = text.lines().position(|l| l.trim_start().starts_with("root")).map(|i| i + 1);
            invalid(&path, line, format!("root {} doesn't exist", root.display()))
        })?;

        let mut runners = Vec::new();
        for (name, value) in raw.runners {
            let line = text.find(&format!("[runners.{name}]")).map(|o| line_of(&text, o));
            let r: RawRunner = value
                .try_into()
                .map_err(|e: toml::de::Error| invalid(&path, line, format!("runner “{name}”: {}", e.message().trim())))?;
            let format = r.format.unwrap_or_else(|| "exit".into());
            if !FORMATS.contains(&format.as_str()) {
                return Err(invalid(&path, line, format!("runner “{name}” has format “{format}”; use tap, cargo, steps or exit")));
            }
            runners.push(Runner {
                name,
                cwd: r.cwd.unwrap_or_else(|| ".".into()),
                env: r.env,
                requires: r.requires,
                run: r.run,
                format,
                coverage: r.coverage,
                tests: r.tests,
                timeout_secs: r.timeout.unwrap_or(1800),
            });
        }

        let traces: RawTraces = std::fs::read_to_string(dir.join(".proof").join(format!("{stem}.traces.json")))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let tests: Vec<&Vec<String>> = traces.runs.values().flat_map(|r| r.values()).collect();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for files in &tests {
            for f in files.iter().collect::<BTreeSet<_>>() {
                *counts.entry(f.clone()).or_default() += 1;
            }
        }
        let reach = counts.into_iter().map(|(f, k)| (f, k as f64 / tests.len() as f64)).collect();

        let mut caps = Vec::new();
        let mut seen = BTreeSet::new();
        for spanned in raw.capability {
            let line = line_of(&text, spanned.span().start);
            let c = spanned.into_inner();
            let id = slug(&[&c.group, &c.name]);
            if !seen.insert(id.clone()) {
                return Err(invalid(&path, Some(line), format!("capability “{}” is in the map twice", c.name)));
            }
            let mut scenarios = Vec::new();
            for s in c.scenario {
                let runner = s.runner.get_ref().clone();
                if !runners.iter().any(|r| r.name == runner) {
                    return Err(invalid(
                        &path,
                        Some(line_of(&text, s.runner.span().start)),
                        format!("scenario “{}” uses runner “{runner}”, which isn’t defined", s.name),
                    ));
                }
                let mut globs = c.code.clone();
                globs.extend(s.test.iter().cloned());
                let traced = traces
                    .runs
                    .get(&runner)
                    .map(|r| {
                        r.iter().filter(|(t, _)| matches(&s.matcher, t)).flat_map(|(_, f)| f.iter().cloned()).collect()
                    })
                    .unwrap_or_default();
                scenarios.push(Scenario {
                    id: slug(&[&c.group, &c.name, &s.name]),
                    name: s.name,
                    runner,
                    matcher: s.matcher,
                    patterns: globset(&globs).map_err(|e| invalid(&path, Some(line), e))?,
                    traced,
                });
            }
            let traced = scenarios.iter().flat_map(|s| s.traced.iter().cloned()).collect();
            caps.push(Capability {
                id,
                group_id: slug(&[&c.group]),
                group: c.group,
                name: c.name,
                inferred: c.inferred,
                patterns: globset(&c.code).map_err(|e| invalid(&path, Some(line), e))?,
                code: c.code,
                scenarios,
                traced,
                line,
            });
        }

        let source = if raw.source.is_empty() { vec!["**".to_string()] } else { raw.source };
        Ok(Map {
            name: raw.name.unwrap_or_else(|| stem.clone()),
            root,
            store_path: dir.join(".proof").join(format!("{stem}.json")),
            traces_path: dir.join(".proof").join(format!("{stem}.traces.json")),
            logs_dir: dir.join(".proof").join("logs"),
            runners,
            caps,
            reach,
            narrow: raw.narrow_below.unwrap_or(0.2),
            trace_commit: traces.commit,
            source: globset(&source).map_err(|e| invalid(&path, None, e))?,
            ignore: globset(&raw.ignore).map_err(|e| invalid(&path, None, e))?,
            path,
        })
    }

    pub fn runner(&self, name: &str) -> Option<&Runner> {
        self.runners.iter().find(|r| r.name == name)
    }

    pub fn cap(&self, id: &str) -> Option<&Capability> {
        self.caps.iter().find(|c| c.id == id)
    }

    /// Capabilities whose group or name contains `query`, or whose id is `query`.
    pub fn find(&self, query: Option<&str>) -> Vec<&Capability> {
        let Some(q) = query.filter(|q| !q.is_empty()) else { return self.caps.iter().collect() };
        let q = q.to_lowercase();
        let exact: Vec<_> = self.caps.iter().filter(|c| c.id == q || c.name.to_lowercase() == q).collect();
        if !exact.is_empty() {
            return exact;
        }
        self.caps
            .iter()
            .filter(|c| c.group.to_lowercase().contains(&q) || c.name.to_lowercase().contains(&q) || c.group_id == q)
            .collect()
    }

    /// Groups in map order: (id, name, capabilities).
    pub fn groups(&self) -> Vec<(String, String, Vec<&Capability>)> {
        let mut out: Vec<(String, String, Vec<&Capability>)> = Vec::new();
        for c in &self.caps {
            match out.iter_mut().find(|g| g.0 == c.group_id) {
                Some(g) => g.2.push(c),
                None => out.push((c.group_id.clone(), c.group.clone(), vec![c])),
            }
        }
        out
    }

    pub fn is_source(&self, path: &str) -> bool {
        self.source.is_match(path) && !self.ignore.is_match(path)
    }

    pub fn reach(&self, path: &str) -> f64 {
        self.reach.get(path).copied().unwrap_or(0.0)
    }

    /// Changed files that hit this capability directly: claimed by its globs, or run by few tests.
    pub fn direct<'a>(&self, cap: &Capability, files: impl IntoIterator<Item = &'a String>) -> BTreeSet<String> {
        files
            .into_iter()
            .filter(|f| cap.patterns.is_match(f.as_str()) || (cap.traced.contains(*f) && self.reach(f) <= self.narrow))
            .cloned()
            .collect()
    }

    /// Traced files few other tests run, so a change there is this capability's own.
    pub fn narrow_traced(&self, cap: &Capability) -> usize {
        cap.traced.iter().filter(|f| self.reach(f) <= self.narrow).count()
    }

    pub fn runner_cwd(&self, r: &Runner) -> PathBuf {
        self.root.join(&r.cwd)
    }

    /// How many of `paths` are source, and the ones no capability claims, sorted.
    pub fn unclaimed<'a, I>(&self, paths: I) -> (usize, Vec<String>)
    where
        I: IntoIterator<Item = &'a String>,
    {
        let mut source: Vec<&String> = paths.into_iter().filter(|p| self.is_source(p)).collect();
        source.sort();
        let claimed = |p: &String| self.caps.iter().any(|c| c.traced.contains(p) || c.patterns.is_match(p.as_str()));
        let orphans = source.iter().filter(|p| !claimed(p)).map(|p| (*p).clone()).collect();
        (source.len(), orphans)
    }
}

/// The paths an item depends on, out of `paths`: its globs, plus what its tests ran.
pub fn owns<'a, I>(patterns: &GlobSet, traced: &BTreeSet<String>, paths: I) -> BTreeSet<String>
where
    I: IntoIterator<Item = &'a String>,
{
    paths.into_iter().filter(|p| traced.contains(*p) || patterns.is_match(p.as_str())).cloned().collect()
}

fn invalid(path: &Path, line: Option<usize>, message: String) -> MapError {
    MapError::Invalid { path: path.to_path_buf(), line, message }
}
