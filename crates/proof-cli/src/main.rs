//! proof: what your app can do, and whether it still does, at this commit.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use proof_core::assess::ScenarioVerdict;
use proof_core::run::{self, Event};
use proof_core::{affected, assess, edit, file_name, git, source_label, Capability, Map, MapError, State, Store, Verdicts};
use serde_json::json;

const GUIDE: &str = include_str!("../../../skill/SKILL.md");

#[derive(Parser)]
#[command(name = "proof", version, about = "What your app can do, and whether it still does, at this commit.")]
#[command(after_help = "A coding agent maps the project and keeps it current; run `proof guide` for how.\nEvery command that reports takes --json.")]
struct Cli {
    /// The map; defaults to $PROOF_MAP, or proof.toml in this directory or above
    #[arg(long, global = true)]
    map: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// The map, with what is passing, failing and stale
    Status {
        query: Option<String>,
        #[arg(short, long)]
        verbose: bool,
        #[arg(long)]
        json: bool,
    },
    /// One capability's scenarios and their evidence
    Show {
        query: String,
        #[arg(long)]
        json: bool,
    },
    /// Run the runners a capability or group needs, and record the results
    Run {
        query: Option<String>,
        /// Run one runner, whatever the query
        #[arg(long)]
        runner: Option<String>,
        /// Only runners with stale, missing or unverified scenarios in scope
        #[arg(long)]
        stale: bool,
        #[arg(long)]
        json: bool,
    },
    /// Record results from a log made elsewhere, such as CI
    Import {
        runner: String,
        log: PathBuf,
        /// The commit the log ran on; read from the log when it says
        #[arg(long)]
        commit: Option<String>,
        /// The run's exit code, for runners that report by exit code or steps; read from the log when it says
        #[arg(long, default_value_t = 0)]
        exit: i32,
        /// Where the log came from, e.g. "ci run 38215980266"
        #[arg(long)]
        source: Option<String>,
    },
    /// Capabilities a change touches, and changed code no capability owns
    Affected {
        /// Measure from this commit instead of HEAD
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        json: bool,
        /// A Markdown summary, for a PR description
        #[arg(long)]
        md: bool,
    },
    /// Source files no capability claims
    Gaps {
        #[arg(long)]
        json: bool,
    },
    /// Run each test alone under coverage; what it executes joins its scenario's claims
    Trace {
        #[arg(long, default_value_t = 4)]
        jobs: usize,
    },
    /// Mark a capability as read by a human: drops `inferred`
    Review { query: String },
    /// Add files to a capability's claimed code
    Claim {
        query: String,
        #[arg(required = true)]
        files: Vec<String>,
    },
    /// Add a capability: proof new "Story runs › Extra life" [files...]
    New {
        title: String,
        files: Vec<String>,
        /// Written by an agent and not yet reviewed
        #[arg(long)]
        inferred: bool,
    },
    /// How a coding agent maps a project and keeps the map current
    Guide,
}

struct Paint(bool);

impl Paint {
    fn new() -> Self {
        Paint(std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none())
    }
    fn code(&self, text: &str, code: &str) -> String {
        if self.0 {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
    fn state(&self, s: State, text: &str) -> String {
        let code = match s {
            State::Passing => "32",
            State::Failing => "31",
            State::Stale => "33",
            State::Missing => "35",
            State::Blocked => "36",
            State::Unverified => "90",
        };
        self.code(text, code)
    }
    fn glyph(&self, s: State) -> String {
        self.state(s, s.glyph())
    }
    fn dim(&self, text: &str) -> String {
        self.code(text, "2")
    }
    fn bold(&self, text: &str) -> String {
        self.code(text, "1")
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("proof: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn load(path: Option<PathBuf>) -> Result<Map> {
    let path = Map::locate(path.as_deref());
    match Map::load(&path) {
        Ok(m) => Ok(m),
        Err(MapError::NotFound(p)) => {
            bail!("no map at {}. Ask your coding agent: “Map this project with proof.” (`proof guide` has how)", p.display())
        }
        Err(e) => Err(e.into()),
    }
}

fn dispatch(cli: Cli) -> Result<ExitCode> {
    if let Cmd::Guide = cli.cmd {
        print!("{GUIDE}");
        return Ok(ExitCode::SUCCESS);
    }
    let map = load(cli.map)?;
    let p = Paint::new();
    match cli.cmd {
        Cmd::Status { query, verbose, json } => status(&map, query.as_deref(), verbose, json, &p),
        Cmd::Show { query, json } => show(&map, &query, json, &p),
        Cmd::Run { query, runner, stale, json } => run_cmd(&map, query.as_deref(), runner.as_deref(), stale, json, &p),
        Cmd::Import { runner, log, commit, exit, source } => {
            let done = run::import_log(&map, &runner, &log, commit.as_deref(), exit, source.as_deref())?;
            let r = &done.record;
            println!("{runner} at {}: {} passed, {} failed, {} tests", done.commit, r.passed, r.failed, r.tests);
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Affected { since, json, md } => affected_cmd(&map, since.as_deref(), json, md, &p),
        Cmd::Gaps { json } => gaps(&map, json, &p),
        Cmd::Trace { jobs } => {
            let report = proof_core::trace::trace(&map, jobs, &Default::default(), &|done, total| {
                if std::io::stderr().is_terminal() {
                    eprint!("\r  tracing {done} of {total} tests");
                }
            })?;
            eprintln!();
            for t in &report.alone_failed {
                println!("{}", p.state(State::Missing, &format!("  could not run alone: {t}")));
            }
            let map = Map::load(&map.path)?;
            let mut sizes: Vec<usize> = map.caps.iter().flat_map(|c| &c.scenarios).map(|s| s.traced.len()).filter(|n| *n > 0).collect();
            sizes.sort();
            let wide = map.reach.values().filter(|r| **r > map.narrow).count();
            println!(
                "{} tests traced; {} scenarios now claim what they run (median {} files, most {}); {wide} files run by over {:.0}% of tests count as shared",
                report.tests,
                sizes.len(),
                sizes.get(sizes.len() / 2).copied().unwrap_or(0),
                sizes.last().copied().unwrap_or(0),
                map.narrow * 100.0
            );
            println!("{}", p.dim("Results recorded before this trace may go stale: their claims grew. Run `proof run` once."));
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Review { query } => {
            let cap = one(&map, &query)?;
            edit::review(&map.path, &cap.id)?;
            println!("{} › {} reviewed", cap.group, cap.name);
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Claim { query, files } => {
            let cap = one(&map, &query)?;
            let n = edit::claim(&map.path, &cap.id, &files)?;
            println!("{} › {} claims {n} more file{}", cap.group, cap.name, if n == 1 { "" } else { "s" });
            Ok(ExitCode::SUCCESS)
        }
        Cmd::New { title, files, inferred } => {
            let Some((group, name)) = edit::split_title(&title) else { bail!("name it “Group › Name”") };
            let id = edit::new_capability(&map.path, &group, &name, &files, inferred)?;
            println!("added {id}; give it scenarios that point at existing tests");
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Guide => unreachable!(),
    }
}

fn one<'a>(map: &'a Map, query: &str) -> Result<&'a Capability> {
    match map.find(Some(query)).as_slice() {
        [c] => Ok(c),
        [] => bail!("no capability matches {query:?}"),
        many => bail!(
            "{query:?} matches {} capabilities: {}",
            many.len(),
            many.iter().take(5).map(|c| c.id.as_str()).collect::<Vec<_>>().join(", ")
        ),
    }
}

fn verdicts(map: &Map) -> Result<(Store, Verdicts)> {
    let store = Store::load(&map.store_path)?;
    let tree = git::blobs(&map.root, None)?;
    let v = assess(map, &store, &tree);
    Ok((store, v))
}

fn scenario_json(map: &Map, c: &Capability, i: usize, v: &ScenarioVerdict) -> serde_json::Value {
    let s = &c.scenarios[i];
    let _ = map;
    json!({
        "id": s.id,
        "name": s.name,
        "status": v.state,
        "runner": s.runner,
        "match": s.matcher,
        "changed": v.changed,
        "evidence": v.entry.as_ref().map(|e| json!({"at": e.at, "commit": e.commit, "source": e.source, "state": e.state, "why": e.why})),
    })
}

fn cap_json(map: &Map, c: &Capability, verdicts: &Verdicts, detail: bool) -> serde_json::Value {
    let v = &verdicts[&c.id];
    let mut out = json!({
        "id": c.id,
        "group": c.group,
        "name": c.name,
        "inferred": c.inferred,
        "status": v.state,
        "passing": v.passing(),
        "scenarios": v.scenarios.iter().enumerate().map(|(i, s)| scenario_json(map, c, i, s)).collect::<Vec<_>>(),
    });
    if detail {
        out["code"] = json!(c.code);
        out["traced"] = json!(c.traced);
        out["traced_narrow"] = json!(map.narrow_traced(c));
    }
    out
}

fn note(v: &proof_core::Verdict) -> String {
    match v.state {
        State::Stale => {
            let changed: Vec<&str> = v.changed().into_iter().map(file_name).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
            let more = if changed.len() > 3 { format!(" +{}", changed.len() - 3) } else { String::new() };
            format!("changed {}{more}", changed.iter().take(3).copied().collect::<Vec<_>>().join(", "))
        }
        State::Blocked => v.blocked_why().unwrap_or_default().lines().next().unwrap_or_default().chars().take(60).collect(),
        State::Missing => "evidence not found".into(),
        State::Failing => {
            let n = v.count(State::Failing);
            format!("{n} scenario{} failing", if n == 1 { "" } else { "s" })
        }
        _ => String::new(),
    }
}

fn status(map: &Map, query: Option<&str>, verbose: bool, json: bool, p: &Paint) -> Result<ExitCode> {
    let (_, verdicts) = verdicts(map)?;
    let caps = map.find(query);
    if json {
        println!("{}", serde_json::to_string_pretty(&caps.iter().map(|c| cap_json(map, c, &verdicts, false)).collect::<Vec<_>>())?);
        return Ok(ExitCode::SUCCESS);
    }
    let head = git::head(&map.root)?;
    let mut tally = [0usize; 6];
    for c in &caps {
        for s in &verdicts[&c.id].scenarios {
            tally[s.state as usize] += 1;
        }
    }
    let summary: Vec<String> = State::ALL
        .iter()
        .rev()
        .filter(|s| tally[**s as usize] > 0)
        .map(|s| p.state(*s, &format!("{} {}", s.glyph(), tally[*s as usize])))
        .collect();
    let at = format!("at {}{}", head.sha, if head.dirty { " +uncommitted" } else { "" });
    println!("{}  {}   {}", p.bold(&map.name), summary.join("  "), p.dim(&at));
    let inferred = caps.iter().filter(|c| c.inferred).count();
    let scenarios: usize = caps.iter().map(|c| c.scenarios.len()).sum();
    let mut line = format!("{} capabilities, {scenarios} scenarios", caps.len());
    if inferred > 0 {
        line.push_str(&format!(", {inferred} inferred and not yet reviewed"));
    }
    println!("{}", p.dim(&line));
    let width = caps.iter().map(|c| c.name.chars().count()).max().unwrap_or(0) + 2;
    let mut group = "";
    for c in &caps {
        if c.group != group {
            group = &c.group;
            println!("\n{}", p.bold(group));
        }
        let v = &verdicts[&c.id];
        let score = format!("{}/{}", v.passing(), v.scenarios.len());
        let tag = if c.inferred && verbose { p.dim("  inferred") } else { String::new() };
        let pad = width.saturating_sub(c.name.chars().count());
        println!("  {} {}{}{}  {}{tag}", p.glyph(v.state), c.name, " ".repeat(pad), p.dim(&score), p.dim(&note(v)));
        if verbose {
            for (s, sv) in c.scenarios.iter().zip(&v.scenarios) {
                println!("      {} {}", p.glyph(sv.state), p.dim(&s.name));
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn show(map: &Map, query: &str, json: bool, p: &Paint) -> Result<ExitCode> {
    let caps = map.find(Some(query));
    if caps.is_empty() {
        bail!("no capability matches {query:?}");
    }
    let (_, verdicts) = verdicts(map)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&caps.iter().map(|c| cap_json(map, c, &verdicts, true)).collect::<Vec<_>>())?);
        return Ok(ExitCode::SUCCESS);
    }
    for c in caps {
        let v = &verdicts[&c.id];
        let inferred = if c.inferred { p.dim("  (inferred, not reviewed)") } else { String::new() };
        println!("{} {}{inferred}", p.glyph(v.state), p.bold(&format!("{} › {}", c.group, c.name)));
        println!("{}", p.dim(&format!("  {}", c.id)));
        println!("{}", p.dim(&format!("  code: {}", c.code.join(", "))));
        if !c.traced.is_empty() {
            println!(
                "{}",
                p.dim(&format!("  its tests also run {} files; {} of them few other tests run", c.traced.len(), map.narrow_traced(c)))
            );
        }
        for (s, sv) in c.scenarios.iter().zip(&v.scenarios) {
            println!("  {} {}", p.glyph(sv.state), s.name);
            println!("{}", p.dim(&format!("      {}: “{}”", s.runner, s.matcher)));
            if let Some(e) = &sv.entry {
                println!("{}", p.dim(&format!("      {} at {} ({}, {})", e.state, e.commit, source_label(&e.source), e.at)));
                if let Some(why) = e.why.as_deref().filter(|_| sv.state == State::Blocked) {
                    println!("{}", p.dim(&format!("      blocked: {}", why.lines().next().unwrap_or_default())));
                }
            }
            for f in &sv.changed {
                println!("{}", p.state(State::Stale, &format!("      changed since: {f}")));
            }
        }
        println!();
    }
    Ok(ExitCode::SUCCESS)
}

fn run_cmd(map: &Map, query: Option<&str>, runner: Option<&str>, stale: bool, json: bool, p: &Paint) -> Result<ExitCode> {
    let caps = map.find(query);
    if caps.is_empty() {
        bail!("no capability matches {:?}", query.unwrap_or_default());
    }
    let names: Vec<String> = match runner {
        Some(r) => {
            map.runner(r).ok_or_else(|| anyhow::anyhow!("no runner called {r:?}"))?;
            vec![r.to_string()]
        }
        None if stale => {
            let (_, verdicts) = verdicts(map)?;
            map.runners
                .iter()
                .filter(|r| {
                    caps.iter().any(|c| {
                        c.scenarios.iter().zip(&verdicts[&c.id].scenarios).any(|(s, v)| {
                            s.runner == r.name && matches!(v.state, State::Stale | State::Missing | State::Unverified | State::Failing)
                        })
                    })
                })
                .map(|r| r.name.clone())
                .collect()
        }
        None => run::runners_for(map, &caps).into_iter().map(|r| r.name.clone()).collect(),
    };
    let stop = Arc::new(AtomicBool::new(false));
    let mut records = Vec::new();
    for name in &names {
        let tty = !json && std::io::stderr().is_terminal();
        let mut seen = 0usize;
        let mut expected = None;
        let record = run::run_runner(map, name, stop.clone(), &mut |e| match e {
            Event::Started { expected: n, .. } => {
                expected = n;
                if !json {
                    eprintln!("{}", p.dim(&format!("  running {name}: {}", map.runner(name).map(|r| r.run.as_str()).unwrap_or(""))));
                }
            }
            Event::Item { .. } => {
                seen += 1;
                if tty {
                    match expected {
                        Some(n) => eprint!("\r  {seen} of {n} tests"),
                        None => eprint!("\r  {seen} tests"),
                    }
                }
            }
            _ => {}
        })?;
        if tty && seen > 0 {
            eprint!("\r\x1b[2K");
        }
        if !json {
            match &record.blocked {
                Some(why) => println!(
                    "  {} {name} blocked: {}",
                    p.glyph(State::Blocked),
                    why.lines().next().unwrap_or_default()
                ),
                None => println!(
                    "  {name}: {} passed, {} failed{}",
                    record.passed,
                    record.failed,
                    p.dim(&format!("  log {}", record.log.as_deref().unwrap_or("")))
                ),
            }
        }
        records.push((name.clone(), record));
    }
    let (_, verdicts) = verdicts(map)?;
    let failing = caps.iter().any(|c| verdicts[&c.id].state == State::Failing);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "runs": records.iter().map(|(n, r)| json!({"runner": n, "record": r})).collect::<Vec<_>>(),
                "capabilities": caps.iter().map(|c| cap_json(map, c, &verdicts, false)).collect::<Vec<_>>(),
            }))?
        );
    } else {
        println!();
        status(map, query, false, false, p)?;
    }
    Ok(if failing { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

fn affected_cmd(map: &Map, since: Option<&str>, json: bool, md: bool, p: &Paint) -> Result<ExitCode> {
    let a = affected::affected(map, since)?;
    let (_, verdicts) = verdicts(map)?;
    let cap = |id: &str| map.cap(id).expect("hit for a capability in the map");
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "since": a.since,
                "commits": a.commits,
                "files": a.files,
                "unmapped": a.unmapped,
                "incidental": a.incidental,
                "capabilities": a.hits.iter().map(|h| {
                    let c = cap(&h.cap);
                    json!({"id": c.id, "group": c.group, "name": c.name, "status": verdicts[&c.id].state, "files": h.files, "direct": !h.direct.is_empty(), "direct_files": h.direct})
                }).collect::<Vec<_>>(),
            }))?
        );
        return Ok(ExitCode::SUCCESS);
    }
    let direct: Vec<_> = a.direct().collect();
    let indirect: Vec<_> = a.indirect().collect();
    let names = |files: &[String]| files.iter().take(4).map(|f| file_name(f)).collect::<Vec<_>>().join(", ");
    if md {
        println!(
            "**{} of {} capabilities hit directly**, {} more run the changed code; {} changed files\n",
            direct.len(),
            map.caps.len(),
            indirect.len(),
            a.files.len()
        );
        println!("| | Capability | Status | Changed |\n|---|---|---|---|");
        for h in &direct {
            let c = cap(&h.cap);
            let st = verdicts[&c.id].state;
            println!("| {} | {} › {} | {} | {} |", st.glyph(), c.group, c.name, st.word(), names(&h.direct));
        }
        if !indirect.is_empty() {
            println!("\nAlso run the changed code: {}", indirect.iter().map(|h| cap(&h.cap).name.as_str()).collect::<Vec<_>>().join(", "));
        }
        let unowned: Vec<&String> = a.incidental.iter().chain(&a.unmapped).collect();
        if !unowned.is_empty() {
            println!(
                "\n{} changed source files belong to no capability: {}",
                unowned.len(),
                unowned.iter().take(10).map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ")
            );
        }
        return Ok(ExitCode::SUCCESS);
    }
    let mut head = format!("{} changed files hit {} of {} capabilities directly", a.files.len(), p.bold(&direct.len().to_string()), map.caps.len());
    if !indirect.is_empty() {
        head.push_str(&format!(", {} more indirectly", indirect.len()));
    }
    println!("{head}{}", p.dim(&format!("  (since {})", since.unwrap_or("HEAD"))));
    for h in &direct {
        let c = cap(&h.cap);
        println!("  {} {} › {}  {}", p.glyph(verdicts[&c.id].state), c.group, c.name, p.dim(&names(&h.direct)));
    }
    if !indirect.is_empty() {
        println!("{}", p.dim("\n  also run the changed code (widely shared files):"));
        for h in &indirect {
            let c = cap(&h.cap);
            println!("  {} {}", p.glyph(verdicts[&c.id].state), p.dim(&format!("{} › {}", c.group, c.name)));
        }
    }
    if !a.incidental.is_empty() {
        println!("{}", p.state(State::Missing, &format!("\n  {} changed files only run along the way; no capability owns them:", a.incidental.len())));
        for f in a.incidental.iter().take(15) {
            println!("{}", p.dim(&format!("    {f}")));
        }
    }
    if !a.unmapped.is_empty() {
        println!("{}", p.state(State::Missing, &format!("\n  {} changed source files no capability claims:", a.unmapped.len())));
        for f in a.unmapped.iter().take(15) {
            println!("{}", p.dim(&format!("    {f}")));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn gaps(map: &Map, json: bool, p: &Paint) -> Result<ExitCode> {
    let tree = git::blobs(&map.root, None)?;
    let (source, orphans) = map.unclaimed(tree.keys());
    if json {
        println!("{}", serde_json::to_string_pretty(&json!({"source": source, "unclaimed": orphans}))?);
        return Ok(ExitCode::SUCCESS);
    }
    println!("{} of {} source files are claimed by a capability; {} are not:", source - orphans.len(), source, orphans.len());
    let mut by_dir: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
    for o in &orphans {
        let (dir, name) = o.rsplit_once('/').unwrap_or((".", o.as_str()));
        by_dir.entry(dir).or_default().push(name);
    }
    for (dir, names) in by_dir {
        let more = if names.len() > 8 { " …" } else { "" };
        println!("  {dir}/  {}", p.dim(&format!("{}{more}", names.iter().take(8).copied().collect::<Vec<_>>().join(", "))));
    }
    Ok(ExitCode::SUCCESS)
}
