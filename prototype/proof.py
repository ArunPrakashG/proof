#!/usr/bin/env python3
"""proof: a living map of what an application does, and whether it still works.

The map (proof.toml) lists capabilities in human terms. Each capability claims the
code it depends on (globs) and has scenarios, each backed by evidence from a test
runner the project already has. A result is pinned to the git blob ids of the code
it covered, so when that code changes the result goes stale on its own.

  proof status [-v] [query]        the map, with what is verified, failing, stale
  proof show <query>               one capability's scenarios and their evidence
  proof run [query]                run the runners those scenarios need, record results
  proof import <runner> <log>      record results from a log made elsewhere (CI)
  proof affected [--since REF]     capabilities a change touches, and changed code no one claims
  proof gaps                       source files no capability claims
  proof trace                      run each test alone under coverage; what it ran becomes its claims
"""
import argparse
import glob
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
from pathlib import Path

SEVERITY = ["failing", "missing", "stale", "blocked", "unverified", "passing"]
MARK = {"passing": "✓", "failing": "✗", "stale": "⚠", "missing": "?", "blocked": "⊘", "unverified": "○"}
COLOR = {"passing": "32", "failing": "31", "stale": "33", "missing": "35", "blocked": "36", "unverified": "90"}
TTY = sys.stdout.isatty() and not os.environ.get("NO_COLOR")


def paint(text, code):
    return f"\033[{code}m{text}\033[0m" if TTY else text


def dim(text):
    return paint(text, "2")


def slug(*parts):
    return "/".join(re.sub(r"[^a-z0-9]+", "-", p.lower()).strip("-") for p in parts)


# ---------------------------------------------------------------- the map

class Map:
    def __init__(self, path):
        self.path = Path(path).resolve()
        data = tomllib.loads(self.path.read_text())
        self.name = data.get("name", self.path.stem)
        self.root = (self.path.parent / data.get("root", ".")).resolve()
        self.store = self.path.parent / ".proof" / f"{self.path.stem}.json"
        self.traces = self.path.parent / ".proof" / f"{self.path.stem}.traces.json"
        self.source = [compile_glob(g) for g in data.get("source", ["**"])]
        self.ignore = [compile_glob(g) for g in data.get("ignore", [])]
        self.runners = data.get("runners", {})
        self.caps = []
        for c in data.get("capability", []):
            cap = {
                "id": slug(c["group"], c["name"]),
                "group": c["group"],
                "name": c["name"],
                "inferred": c.get("inferred", False),
                "code": c.get("code", []),
                "patterns": [compile_glob(g) for g in c.get("code", [])],
                "scenarios": [],
            }
            for s in c.get("scenario", []):
                if s["runner"] not in self.runners:
                    sys.exit(f"{cap['name']} / {s['name']}: no runner called {s['runner']!r}")
                cap["scenarios"].append({
                    "id": slug(c["group"], c["name"], s["name"]),
                    "name": s["name"],
                    "runner": s["runner"],
                    "match": s["match"],
                    "patterns": cap["patterns"] + [compile_glob(g) for g in s.get("test", [])],
                })
            self.caps.append(cap)
        # what each scenario's tests actually executed (proof trace), on top of the globs
        runs = json.loads(self.traces.read_text())["runs"] if self.traces.exists() else {}
        tests = [set(files) for r in runs.values() for files in r.values()]
        # reach: the share of all tests that execute a file. A change to a file few tests run hits
        # those capabilities directly; one to a file most tests run (boot, the game loop) hits them indirectly.
        self.reach = {f: k / len(tests) for f, k in Counter(f for t in tests for f in t).items()}
        self.narrow = float(data.get("narrow_below", 0.2))
        for cap in self.caps:
            for s in cap["scenarios"]:
                hits = [set(files) for name, files in runs.get(s["runner"], {}).items() if matches(s["match"], name)]
                s["traced"] = set().union(*hits)
            cap["traced"] = set().union(*(s["traced"] for s in cap["scenarios"]))

    def direct(self, cap, files):
        """Changed files that hit this capability directly: claimed by its globs, or run by few tests."""
        return {f for f in files if any(r.match(f) for r in cap["patterns"])
                or (f in cap["traced"] and self.reach.get(f, 0) <= self.narrow)}

    def results(self):
        return json.loads(self.store.read_text()) if self.store.exists() else {}

    def save(self, results):
        self.store.parent.mkdir(parents=True, exist_ok=True)
        self.store.write_text(json.dumps(results, indent=1, sort_keys=True))

    def find(self, query):
        if not query:
            return self.caps
        q = query.lower()
        return [c for c in self.caps if q in c["group"].lower() or q in c["name"].lower() or q == c["id"]]

    def is_source(self, path):
        return any(p.match(path) for p in self.source) and not any(p.match(path) for p in self.ignore)


def compile_glob(pattern):
    return re.compile(glob.translate(pattern, recursive=True, include_hidden=True))


def owns(item, paths):
    """The paths a capability or scenario depends on: its globs, plus what its tests ran."""
    return {p for p in paths if p in item["traced"] or any(r.match(p) for r in item["patterns"])}


# ---------------------------------------------------------------- git

def git(root, *args, input=None):
    return subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True,
                          check=True, input=input).stdout


def blobs(root, rev=None):
    """path -> blob id, for a commit or for the working tree as it is now."""
    out = {}
    if rev:
        for line in git(root, "ls-tree", "-r", "-z", rev).split("\0"):
            if line:
                meta, path = line.split("\t", 1)
                out[path] = meta.split()[2]
        return out
    for line in git(root, "ls-files", "-s", "-z").split("\0"):
        if line:
            meta, path = line.split("\t", 1)
            out[path] = meta.split()[1]
    touched = [p for p in git(root, "diff", "--name-only", "-z").split("\0") if p]
    touched += [p for p in git(root, "ls-files", "-o", "--exclude-standard", "-z").split("\0") if p]
    present = [p for p in touched if (root / p).is_file()]
    for p in set(touched) - set(present):
        out.pop(p, None)
    if present:
        ids = git(root, "hash-object", "--stdin-paths", input="\n".join(present)).split()
        out.update(zip(present, ids))
    return out


def head(root):
    sha = git(root, "rev-parse", "--short", "HEAD").strip()
    dirty = bool(git(root, "status", "--porcelain", "--untracked-files=no").strip())
    return sha, dirty


# ---------------------------------------------------------------- evidence

def parse(fmt, text, code):
    """A runner's log -> [(name, ok)], and whether a missing name means 'not reached'."""
    items = []
    text = text.replace("\r", "")  # lines may carry a CI prefix (job, step, timestamp) before the runner's own
    if fmt == "tap":
        for m in re.finditer(r"(?:^|\s)(not )?ok \d+ - (.*?)(?:\s+# (?:SKIP|TODO).*)?$", text, re.M):
            items.append((m.group(2).strip(), m.group(1) is None))
        return items, False
    if fmt == "cargo":
        for m in re.finditer(r"(?:^|\s)test (\S+) \.\.\. (ok|FAILED|ignored)$", text, re.M):
            if m.group(2) != "ignored":
                items.append((m.group(1), m.group(2) == "ok"))
        return items, False
    if fmt == "steps":
        # '· step' printed as each step starts; the run stops at the first failure
        steps = [m.group(1).strip() for m in re.finditer(r"(?:^|\s)· (.+)$", text, re.M)]
        for i, s in enumerate(steps):
            items.append((s, code == 0 or i < len(steps) - 1))
        return items, code != 0
    if fmt == "exit":
        return [("*", code == 0)], False
    sys.exit(f"unknown runner format {fmt!r}")


def matches(match, name):
    if match == "*":
        return True
    if match.startswith("="):
        return name == match[1:]
    return match.lower() in name.lower()


def record(m, runner, text, code, commit, tree, source, blocked=None):
    results = m.results()
    fmt = m.runners[runner].get("format", "exit")
    items, partial = parse(fmt, text, code) if not blocked else ([], False)
    now = datetime.now(timezone.utc).isoformat(timespec="seconds")
    counts = {}
    for cap in m.caps:
        for s in cap["scenarios"]:
            if s["runner"] != runner:
                continue
            base = {"at": now, "commit": commit, "source": source}
            if blocked:
                if s["id"] in results:
                    continue  # a run that could not happen does not erase what is known
                entry = {**base, "state": "blocked", "why": blocked}
            else:
                hits = [ok for name, ok in items if matches(s["match"], name)]
                if not hits and partial:
                    continue  # the run stopped before reaching it
                state = "missing" if not hits else "pass" if all(hits) else "fail"
                fp = {p: tree[p] for p in owns(s, tree)}
                entry = {**base, "state": state, "fingerprint": fp}
            results[s["id"]] = entry
            counts[entry["state"]] = counts.get(entry["state"], 0) + 1
    m.save(results)
    return counts


def runner_env(r):
    return {**os.environ, **{k: os.path.expandvars(v) for k, v in r.get("env", {}).items()}}


def run_runner(m, name):
    r = m.runners[name]
    env = runner_env(r)
    cwd = m.root / r.get("cwd", ".")
    sha, dirty = head(m.root)
    commit = sha + ("+dirty" if dirty else "")
    tree = blobs(m.root)
    logs = m.store.parent / "logs"
    logs.mkdir(parents=True, exist_ok=True)
    log = logs / f"{name}-{datetime.now():%Y%m%d-%H%M%S}.log"
    if r.get("requires"):
        pre = subprocess.run(r["requires"], shell=True, cwd=cwd, env=env, capture_output=True, text=True)
        if pre.returncode:
            why = (pre.stdout + pre.stderr).strip() or f"`{r['requires']}` failed"
            return record(m, name, "", 1, commit, tree, "local", blocked=why), log, why
    print(dim(f"  running {name}: {r['run']}"), flush=True)
    p = subprocess.run(r["run"], shell=True, cwd=cwd, env=env, capture_output=True, text=True,
                       timeout=r.get("timeout", 1800))
    log.write_text(p.stdout + p.stderr)
    return record(m, name, p.stdout + p.stderr, p.returncode, commit, tree, "local"), log, None


# ---------------------------------------------------------------- status

def assess(m, results, tree):
    """capability id -> (rollup, [(scenario, status, detail)])"""
    out = {}
    for cap in m.caps:
        rows = []
        for s in cap["scenarios"]:
            r = results.get(s["id"])
            if not r:
                rows.append((s, "unverified", r, None))
                continue
            if r["state"] in ("blocked", "missing"):
                rows.append((s, r["state"], r, None))
                continue
            now = {p: tree[p] for p in owns(s, tree)}
            then = r["fingerprint"]
            changed = sorted(p for p in set(now) | set(then) if now.get(p) != then.get(p))
            if changed:
                rows.append((s, "stale", r, changed))
            else:
                rows.append((s, "passing" if r["state"] == "pass" else "failing", r, None))
        rollup = min((st for _, st, _, _ in rows), key=SEVERITY.index, default="unverified")
        out[cap["id"]] = (rollup, rows)
    return out


def cmd_status(m, args):
    results, tree = m.results(), blobs(m.root)
    verdicts = assess(m, results, tree)
    caps = m.find(args.query)
    if args.json:
        print(json.dumps([{
            "id": c["id"], "group": c["group"], "name": c["name"], "inferred": c["inferred"],
            "status": verdicts[c["id"]][0],
            "scenarios": [{"name": s["name"], "status": st, "changed": ch,
                           "evidence": r and {k: r[k] for k in ("at", "commit", "source", "state")}}
                          for s, st, r, ch in verdicts[c["id"]][1]],
        } for c in caps], indent=1))
        return
    sha, dirty = head(m.root)
    tally = {}
    for c in caps:
        for _, st, _, _ in verdicts[c["id"]][1]:
            tally[st] = tally.get(st, 0) + 1
    summary = "  ".join(paint(f"{MARK[k]} {tally[k]}", COLOR[k]) for k in SEVERITY[::-1] if k in tally)
    inferred = sum(c["inferred"] for c in caps)
    print(f"{paint(m.name, '1')}  {summary}   {dim('at ' + sha + (' +uncommitted' if dirty else ''))}")
    print(dim(f"{len(caps)} capabilities, {sum(len(c['scenarios']) for c in caps)} scenarios"
              + (f", {inferred} inferred and not yet reviewed" if inferred else "")))
    width = max((len(c["name"]) for c in caps), default=0) + 2
    group = None
    for c in caps:
        if c["group"] != group:
            group = c["group"]
            print(f"\n{paint(group, '1')}")
        rollup, rows = verdicts[c["id"]]
        ok = sum(st == "passing" for _, st, _, _ in rows)
        note = ""
        stale = sorted({p for _, st, _, ch in rows if st == "stale" for p in ch})
        if rollup == "stale":
            note = "changed: " + ", ".join(Path(p).name for p in stale[:3]) + (" …" if len(stale) > 3 else "")
        elif rollup == "blocked":
            note = next(r["why"] for _, st, r, _ in rows if st == "blocked").splitlines()[0][:60]
        elif rollup == "missing":
            note = "evidence not found"
        tag = dim("  inferred") if c["inferred"] and args.verbose else ""
        print(f"  {paint(MARK[rollup], COLOR[rollup])} {c['name']:<{width}}{dim(f'{ok}/{len(rows)}')}  {dim(note)}{tag}")
        if args.verbose:
            for s, st, r, _ in rows:
                print(f"      {paint(MARK[st], COLOR[st])} {dim(s['name'])}")


def cmd_show(m, args):
    caps = m.find(args.query)
    if not caps:
        sys.exit(f"no capability matches {args.query!r}")
    verdicts = assess(m, m.results(), blobs(m.root))
    for c in caps:
        rollup, rows = verdicts[c["id"]]
        print(f"{paint(MARK[rollup], COLOR[rollup])} {paint(c['group'] + ' › ' + c['name'], '1')}"
              f"{dim('  (inferred, not reviewed)') if c['inferred'] else ''}")
        print(dim("  code: " + ", ".join(c["code"])))
        for s, st, r, changed in rows:
            print(f"  {paint(MARK[st], COLOR[st])} {s['name']}")
            print(dim(f"      {s['runner']}: “{s['match']}”"))
            if r:
                print(dim(f"      {r['state']} at {r['commit']} ({r['source']}, {r['at']})"))
            if r and r.get("why"):
                print(dim("      blocked: " + r["why"].splitlines()[0]))
            for p in changed or []:
                print(paint(f"      changed since: {p}", COLOR["stale"]))
        print()


def cmd_run(m, args):
    caps = m.find(args.query)
    needed = sorted({s["runner"] for c in caps for s in c["scenarios"]})
    if args.runner:
        needed = [args.runner]
    for name in needed:
        counts, log, blocked = run_runner(m, name)
        if blocked:
            print(f"  {paint('⊘', COLOR['blocked'])} {name} blocked: {blocked.splitlines()[0]}")
        else:
            print(f"  {name}: " + ", ".join(f"{v} {k}" for k, v in sorted(counts.items())) + dim(f"  log {log}"))
    print()
    cmd_status(m, argparse.Namespace(query=args.query, json=False, verbose=False))


def cmd_import(m, args):
    text = Path(args.log).read_text(errors="replace")
    tree = blobs(m.root, args.commit)
    sha = git(m.root, "rev-parse", "--short", args.commit).strip()
    counts = record(m, args.runner, text, args.exit, sha, tree, args.source)
    print(f"{args.runner} at {sha}: " + ", ".join(f"{v} {k}" for k, v in sorted(counts.items())))


def changed_files(m, since):
    out = set(p for p in git(m.root, "diff", "--name-only", "-z", since or "HEAD").split("\0") if p)
    out |= set(p for p in git(m.root, "ls-files", "-o", "--exclude-standard", "-z").split("\0") if p)
    return sorted(out)


def cmd_affected(m, args):
    files = changed_files(m, args.since)
    verdicts = assess(m, m.results(), blobs(m.root))
    hit, mapped = [], set()
    for c in m.caps:
        mine = sorted(owns(c, files))
        if mine:
            hit.append((c, mine, sorted(m.direct(c, mine))))
            mapped |= set(mine)
    unmapped = [f for f in files if f not in mapped and m.is_source(f)]
    owned = set().union(*(set(d) for _, _, d in hit))
    # run by some tests, but only along the way: likely a capability the map does not have yet
    incidental = [f for f in files if f in mapped and f not in owned and m.reach.get(f, 0) <= m.narrow * 2]
    direct = [h for h in hit if h[2]]
    indirect = [h for h in hit if not h[2]]
    if args.json:
        print(json.dumps({"files": files, "unmapped": unmapped, "incidental": incidental, "capabilities": [
            {"id": c["id"], "name": c["name"], "status": verdicts[c["id"]][0], "files": f, "direct": bool(d)}
            for c, f, d in hit]}, indent=1))
        return
    if args.md:
        print(f"**{len(direct)} of {len(m.caps)} capabilities hit directly**, {len(indirect)} more run the changed code;"
              f" {len(files)} changed files\n")
        print("| | Capability | Status | Changed |\n|---|---|---|---|")
        for c, f, d in direct:
            st = verdicts[c["id"]][0]
            print(f"| {MARK[st]} | {c['group']} › {c['name']} | {st} | {', '.join(Path(p).name for p in d[:4])} |")
        if indirect:
            print("\nAlso run the changed code: " + ", ".join(c["name"] for c, _, _ in indirect))
        if unmapped:
            print(f"\n{len(unmapped)} changed source files belong to no capability: " +
                  ", ".join(f"`{p}`" for p in unmapped[:10]))
        return
    print(f"{len(files)} changed files hit {paint(str(len(direct)), '1')} of {len(m.caps)} capabilities directly"
          + (f", {len(indirect)} more indirectly" if indirect else "") + dim(f"  (since {args.since or 'HEAD'})"))
    for c, f, d in direct:
        st = verdicts[c["id"]][0]
        print(f"  {paint(MARK[st], COLOR[st])} {c['group']} › {c['name']}  {dim(', '.join(Path(p).name for p in d[:4]))}")
    if indirect:
        print(dim("\n  also run the changed code (widely shared files):"))
        for c, f, _ in indirect:
            st = verdicts[c["id"]][0]
            print(f"  {paint(MARK[st], COLOR[st])} {dim(c['group'] + ' › ' + c['name'])}")
    if incidental:
        print(paint(f"\n  {len(incidental)} changed files only run along the way; no capability owns them:",
                    COLOR["missing"]))
        for p in incidental[:15]:
            print(dim("    " + p))
    if unmapped:
        print(paint(f"\n  {len(unmapped)} changed source files no capability claims:", COLOR["missing"]))
        for p in unmapped[:15]:
            print(dim("    " + p))


def cmd_gaps(m, args):
    tree = blobs(m.root)
    every = set()
    for c in m.caps:
        every |= owns(c, tree)
    orphans = sorted(p for p in tree if m.is_source(p) and p not in every)
    by_dir = {}
    for p in orphans:
        by_dir.setdefault(str(Path(p).parent), []).append(Path(p).name)
    source = sum(1 for p in tree if m.is_source(p))
    print(f"{source - len(orphans)} of {source} source files are claimed by a capability; {len(orphans)} are not:")
    for d, names in sorted(by_dir.items()):
        print(f"  {d}/  {dim(', '.join(names[:8]) + (' …' if len(names) > 8 else ''))}")


def executed(cov_dir, root):
    """Source files (relative to root) in which some function other than the module body ran."""
    out = set()
    for f in Path(cov_dir).glob("*.json"):
        for script in json.loads(f.read_text()).get("result", []):
            url = script["url"]
            if not url.startswith(f"file://{root}/"):
                continue
            body = lambda fn: fn["functionName"] == "" and fn["ranges"][0]["startOffset"] == 0  # noqa: E731
            if any(fn["ranges"][0]["count"] > 0 and not body(fn) for fn in script["functions"]):
                out.add(url[len(f"file://{root}/"):])
    return out


def trace_one(m, cwd, env, test_file, name):
    cov = tempfile.mkdtemp(prefix="cov-", dir=m.store.parent)
    try:
        p = subprocess.run(["node", "--test", "--test-reporter=tap", f"--test-name-pattern=^{re.escape(name)}$",
                            str(test_file)], cwd=cwd, env={**env, "NODE_V8_COVERAGE": cov},
                           capture_output=True, text=True, timeout=600)
        ran = bool(re.search(rf"^(not )?ok \d+ - {re.escape(name)}\s*$", p.stdout, re.M))
        return ran, {f for f in executed(cov, m.root) if m.is_source(f)}
    finally:
        shutil.rmtree(cov, ignore_errors=True)


def cmd_trace(m, args):
    """Each test alone under V8 coverage: the files it executed become its scenarios' claims."""
    m.store.parent.mkdir(parents=True, exist_ok=True)
    jobs = []
    for name, r in m.runners.items():
        if r.get("coverage") != "node":
            continue
        env, cwd = runner_env(r), m.root / r.get("cwd", ".")
        for f in sorted(cwd.glob(r["tests"])):
            out = subprocess.run(["node", "--test", "--test-reporter=tap", str(f)], cwd=cwd, env=env,
                                 capture_output=True, text=True).stdout
            for t in re.finditer(r"^(?:not )?ok \d+ - (.*?)(?:\s+# (?:SKIP|TODO).*)?$", out, re.M):
                jobs.append((name, cwd, env, f, t.group(1)))
    print(dim(f"  tracing {len(jobs)} tests one at a time ({args.jobs} at once)"), flush=True)
    with ThreadPoolExecutor(args.jobs) as pool:
        traced = list(pool.map(lambda j: trace_one(m, *j[1:]), jobs))
    runs = {}
    for (runner, _, _, _, test), (ran, files) in zip(jobs, traced):
        if not ran:
            print(paint(f"  could not run alone: {test}", COLOR["missing"]))
        runs.setdefault(runner, {})[test] = sorted(files)
    m.traces.write_text(json.dumps({"commit": head(m.root)[0], "runs": runs}, indent=1))
    m = Map(m.path)
    sizes = sorted(len(s["traced"]) for c in m.caps for s in c["scenarios"] if s["traced"])
    wide = sorted(f for f, r in m.reach.items() if r > m.narrow)
    print(f"  {len(sizes)} scenarios traced; files each, median {sizes[len(sizes) // 2]}, max {sizes[-1]}; "
          f"{len(wide)} files run by more than {m.narrow:.0%} of tests count as indirect")


def main():
    ap = argparse.ArgumentParser(prog="proof", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--map", default=os.environ.get("PROOF_MAP", "proof.toml"))
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("status")
    p.add_argument("query", nargs="?")
    p.add_argument("-v", "--verbose", action="store_true")
    p.add_argument("--json", action="store_true")
    p = sub.add_parser("show")
    p.add_argument("query")
    p = sub.add_parser("run")
    p.add_argument("query", nargs="?")
    p.add_argument("--runner")
    p = sub.add_parser("import")
    p.add_argument("runner")
    p.add_argument("log")
    p.add_argument("--commit", default="HEAD")
    p.add_argument("--exit", type=int, default=0)
    p.add_argument("--source", default="imported")
    p = sub.add_parser("affected")
    p.add_argument("--since")
    p.add_argument("--json", action="store_true")
    p.add_argument("--md", action="store_true")
    sub.add_parser("gaps")
    p = sub.add_parser("trace")
    p.add_argument("--jobs", type=int, default=4)
    args = ap.parse_args()
    m = Map(args.map)
    {"status": cmd_status, "show": cmd_show, "run": cmd_run, "import": cmd_import,
     "affected": cmd_affected, "gaps": cmd_gaps, "trace": cmd_trace}[args.cmd](m, args)


if __name__ == "__main__":
    main()
