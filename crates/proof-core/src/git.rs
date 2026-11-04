//! The few git questions proof asks, through the git CLI.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

pub type Tree = HashMap<String, String>;

pub fn git(root: &Path, args: &[&str]) -> Result<String> {
    git_input(root, args, None)
}

fn git_input(root: &Path, args: &[&str], input: Option<&str>) -> Result<String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("git isn't installed")?;
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn zsplit(s: &str) -> impl Iterator<Item = &str> {
    s.split('\0').filter(|p| !p.is_empty())
}

/// path -> blob id, for a commit, or for the working tree as it is now when `rev` is None.
pub fn blobs(root: &Path, rev: Option<&str>) -> Result<Tree> {
    let mut out = Tree::new();
    if let Some(rev) = rev {
        for line in zsplit(&git(root, &["ls-tree", "-r", "-z", rev])?) {
            if let Some((meta, path)) = line.split_once('\t') {
                if let Some(id) = meta.split_whitespace().nth(2) {
                    out.insert(path.to_string(), id.to_string());
                }
            }
        }
        return Ok(out);
    }
    for line in zsplit(&git(root, &["ls-files", "-s", "-z"])?) {
        if let Some((meta, path)) = line.split_once('\t') {
            if let Some(id) = meta.split_whitespace().nth(1) {
                out.insert(path.to_string(), id.to_string());
            }
        }
    }
    let mut touched: Vec<String> = zsplit(&git(root, &["diff", "--name-only", "-z"])?).map(String::from).collect();
    touched.extend(zsplit(&git(root, &["ls-files", "-o", "--exclude-standard", "-z"])?).map(String::from));
    touched.sort();
    touched.dedup();
    let (present, gone): (Vec<String>, Vec<String>) = touched.into_iter().partition(|p| root.join(p).is_file());
    for p in gone {
        out.remove(&p);
    }
    if !present.is_empty() {
        let ids = git_input(root, &["hash-object", "--stdin-paths"], Some(&present.join("\n")))?;
        out.extend(present.into_iter().zip(ids.split_whitespace().map(String::from)));
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct Head {
    pub branch: Option<String>,
    pub sha: String,
    pub dirty: bool,
}

impl Head {
    /// `7c6eb24`, or `7c6eb24+dirty` for results recorded on uncommitted code.
    pub fn commit_label(&self) -> String {
        if self.dirty {
            format!("{}+dirty", self.sha)
        } else {
            self.sha.clone()
        }
    }
}

pub fn head(root: &Path) -> Result<Head> {
    let sha = git(root, &["rev-parse", "--short", "HEAD"])?.trim().to_string();
    let branch = git(root, &["symbolic-ref", "--short", "-q", "HEAD"]).ok().map(|b| b.trim().to_string()).filter(|b| !b.is_empty());
    let dirty = !git(root, &["status", "--porcelain", "--untracked-files=no"])?.trim().is_empty();
    Ok(Head { branch, sha, dirty })
}

pub fn short(root: &Path, rev: &str) -> Result<String> {
    Ok(git(root, &["rev-parse", "--short", rev])?.trim().to_string())
}

pub fn exists(root: &Path, rev: &str) -> bool {
    git(root, &["cat-file", "-e", &format!("{rev}^{{commit}}")]).is_ok()
}

/// Files changed between `since` (default HEAD) and the working tree, untracked ones included.
pub fn changed_files(root: &Path, since: Option<&str>) -> Result<Vec<String>> {
    let mut out: Vec<String> =
        zsplit(&git(root, &["diff", "--name-only", "-z", since.unwrap_or("HEAD")])?).map(String::from).collect();
    out.extend(zsplit(&git(root, &["ls-files", "-o", "--exclude-standard", "-z"])?).map(String::from));
    out.sort();
    out.dedup();
    Ok(out)
}

/// path -> (lines added, lines removed) between `since` and the working tree.
pub fn numstat(root: &Path, since: &str, paths: &[String]) -> Result<HashMap<String, (u32, u32)>> {
    let mut out = HashMap::new();
    if paths.is_empty() {
        return Ok(out);
    }
    let mut args = vec!["diff", "--numstat", "-z", since, "--"];
    args.extend(paths.iter().map(String::as_str));
    let text = git(root, &args)?;
    for rec in zsplit(&text) {
        let mut parts = rec.splitn(3, '\t');
        let (Some(a), Some(d), Some(p)) = (parts.next(), parts.next(), parts.next()) else { continue };
        out.insert(p.to_string(), (a.parse().unwrap_or(0), d.parse().unwrap_or(0)));
    }
    for p in paths {
        if !out.contains_key(p) {
            if let Ok(text) = std::fs::read_to_string(root.join(p)) {
                out.insert(p.clone(), (text.lines().count() as u32, 0));
            }
        }
    }
    Ok(out)
}

/// Commits from `since` (exclusive) to HEAD.
pub fn count_commits(root: &Path, since: &str) -> usize {
    git(root, &["rev-list", "--count", &format!("{since}..HEAD")]).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}

/// The newest commit after `since` that touched any of `paths`: (short sha, subject).
pub fn latest_touching(root: &Path, since: &str, paths: &[String]) -> Option<(String, String)> {
    if paths.is_empty() {
        return None;
    }
    // walking back from HEAD stops at the first hit; a `since..HEAD` range makes git walk all of it first
    let mut args = vec!["log", "-1", "--format=%h%x00%s", "HEAD", "--"];
    args.extend(paths.iter().map(String::as_str));
    let out = git(root, &args).ok()?;
    let (sha, subject) = out.trim().split_once('\0')?;
    if git(root, &["merge-base", "--is-ancestor", sha, since]).is_ok() {
        return None;
    }
    Some((sha.to_string(), subject.to_string()))
}

pub fn subject(root: &Path, rev: &str) -> Option<String> {
    git(root, &["log", "-1", "--format=%s", rev]).ok().map(|s| s.trim().to_string())
}
