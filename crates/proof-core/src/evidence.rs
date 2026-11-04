//! Reading a runner's output: which tests ran, and whether they passed.

use std::sync::OnceLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub ok: bool,
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).unwrap())
}

static TAP: OnceLock<Regex> = OnceLock::new();
static SPEC: OnceLock<Regex> = OnceLock::new();
static CARGO: OnceLock<Regex> = OnceLock::new();
static STEP: OnceLock<Regex> = OnceLock::new();

/// One line of output, as it arrives. Lines may carry a CI prefix (job, step, timestamp) first.
pub fn parse_line(format: &str, line: &str) -> Option<Item> {
    let line = line.trim_end_matches('\r');
    match format {
        "tap" => {
            if let Some(c) = re(&TAP, r"(?:^|\s)(not )?ok \d+ - (.*?)(?:\s+# (?:SKIP|TODO).*)?$").captures(line) {
                return Some(Item { name: c[2].trim().to_string(), ok: c.get(1).is_none() });
            }
            // node's default spec reporter, which CI logs often have instead: `✔ name (12.3ms)`
            let c = re(&SPEC, r"(?:^|\s)(✔|✖) (.*?) \([\d.]+m?s\)$").captures(line)?;
            Some(Item { name: c[2].trim().to_string(), ok: &c[1] == "✔" })
        }
        "cargo" => {
            let c = re(&CARGO, r"(?:^|\s)test (\S+) \.\.\. (ok|FAILED|ignored)$").captures(line)?;
            (&c[2] != "ignored").then(|| Item { name: c[1].to_string(), ok: &c[2] == "ok" })
        }
        // '· step' printed as each step starts; whether it passed is known once the next starts
        "steps" => {
            let c = re(&STEP, r"(?:^|\s)· (.+)$").captures(line)?;
            Some(Item { name: c[1].trim().to_string(), ok: true })
        }
        _ => None,
    }
}

/// A whole log -> the tests in it, and whether a test missing from it means "not reached".
pub fn parse(format: &str, text: &str, exit_code: i32) -> (Vec<Item>, bool) {
    let mut items: Vec<Item> = text.lines().filter_map(|l| parse_line(format, l)).collect();
    match format {
        "steps" => {
            // the run stops at the first failure: the last step started is the one that failed
            if exit_code != 0 {
                if let Some(last) = items.last_mut() {
                    last.ok = false;
                }
            }
            (items, exit_code != 0)
        }
        "exit" => (vec![Item { name: "*".into(), ok: exit_code == 0 }], false),
        _ => (items, false),
    }
}

/// The TAP plan (`1..68`), when the runner prints one.
pub fn tap_plan(text: &str) -> Option<usize> {
    text.lines().rev().find_map(|l| l.trim().strip_prefix("1..").and_then(|n| n.trim().parse().ok()))
}
