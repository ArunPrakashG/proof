#!/usr/bin/env python3
"""How noisy would `proof affected` have been? Replays every first-parent commit on main through
the map, with glob claims only and with traced claims split into direct and indirect hits.
Traces are from HEAD, so for old commits they are an approximation (files since renamed are missed)."""
import re
import statistics
import sys
from collections import Counter

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from proof import Map, git, owns  # noqa: E402

traced = Map(sys.argv[1])
globs = Map(sys.argv[1])
for c in globs.caps:
    c["traced"] = set()

commits = []
for line in git(traced.root, "log", "--first-parent", "--format=%h%x09%s", "main").splitlines():
    sha, subject = line.split("\t", 1)
    files = git(traced.root, "diff-tree", "--no-commit-id", "-r", "--name-only", "-m", "--first-parent", sha).split("\n")
    source = [f for f in files if f and traced.is_source(f)]
    if source:
        commits.append((source, bool(re.search(r"\bFixes #\d+", subject))))
n, fixes = len(commits), sum(f for _, f in commits)
print(f"{n} source-changing commits on main, {len(traced.caps)} capabilities, {fixes} of them bug fixes\n")
print(f"{'claims':<24}{'direct/commit':>14}{'p90':>5}{'+indirect':>10}{'unclaimed':>11}{'fixes seen':>12}")
for label, m in [("globs only", globs), ("traced, two tiers", traced)]:
    direct, indirect, loose_commits, seen, loose = [], [], 0, 0, Counter()
    for source, fix in commits:
        hit = [c for c in m.caps if owns(c, source)]
        d = [c for c in hit if m.direct(c, source)] if m is traced else hit
        direct.append(len(d))
        indirect.append(len(hit) - len(d))
        stray = [f for f in source if not any(owns(c, [f]) for c in m.caps)]
        loose.update(stray)
        loose_commits += bool(stray)
        seen += fix and bool(hit)
    print(f"{label:<24}{statistics.median(direct):>14}{sorted(direct)[int(n * .9)]:>5}"
          f"{statistics.median(indirect):>10}{loose_commits / n:>11.0%}{seen:>8}/{fixes}")
ownerless = Counter()
for source, _ in commits:
    stray = [f for f in source if any(owns(c, [f]) for c in traced.caps)
             and not any(traced.direct(c, [f]) for c in traced.caps) and traced.reach.get(f, 0) <= traced.narrow * 2]
    ownerless.update(stray)
    ownerless["__commits__"] += bool(stray)
print(f"\ncommits changing a file only run along the way (no capability owns it): "
      f"{ownerless.pop('__commits__') / n:.0%}; most often: "
      + ", ".join(f"{f.rsplit('/', 1)[1]} ×{k}" for f, k in ownerless.most_common(5)))
print("still unclaimed most often:", ", ".join(f"{f.rsplit('/', 1)[1]} ×{k}" for f, k in loose.most_common(6)))
