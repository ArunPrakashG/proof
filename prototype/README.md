# proof: prototype

A capability map with staleness, built to test one question: does tying verification results to the
code they covered give a useful "what still works" signal, or is it noise? Dogfooded on a real project, using a local clone so the original repo was not touched.

## What is here

| File | What |
|---|---|
| `proof.py`, `proof` | The CLI (Python stdlib, ~530 lines): `status`, `show`, `run`, `import`, `affected`, `gaps`, `trace` |
| `maps/example.toml` | The map an agent wrote: 33 capabilities, 93 scenarios, 4 runners (game, replay, api, e2e) |
| `maps/.proof/` | Recorded results, per-test traces and run logs |
| `skill/SKILL.md` | How a coding agent should use it |
| `eval_history.py` | Replays git history through the map: globs only vs traced claims |
| `ci/` | The last green CI run's logs, imported as evidence |

```bash
export PROOF_MAP=$PWD/maps/example.toml
./proof status            # the map
./proof show "foil"       # one capability, its evidence and what changed since
./proof affected          # what uncommitted changes touch, directly and indirectly
./proof run hits          # run only what that capability needs
./proof trace             # each Node test alone under coverage (~2 min here)
python3 eval_history.py maps/example.toml
```

## How it works

- Each capability claims code with globs; each scenario points at an existing test by name.
- `proof trace` runs each test on its own under V8 coverage. The source files where it executed a
  function join its scenario's claims. Only runners with `coverage = "node"`; Rust and e2e keep globs.
- A result stores the git blob id of every claimed file. When any of them changes, the result is stale.
  This works for evidence from other commits too: CI logs imported at commit X are fingerprinted against X's tree.
- `affected` splits hits in two. **Direct**: a capability's globs claim the changed file, or its tests run it
  and at most 20% of all tests do (`narrow_below`). **Indirect**: its tests run a widely shared file (the sim, the engine).
- A changed file that tests run, but only along the way (no direct owner, run by at most 40% of tests),
  is reported as likely belonging to a capability the map doesn't have yet.
- States: ✓ passing, ✗ failing, ⚠ stale, ? evidence missing (test renamed or gone), ⊘ blocked (could not run here), ○ unverified.

## Findings

1. **Mapping by an agent is fast and decent.** One pass over routes, module names and test names gave
   33 capabilities that read like the product. Hypothesis 1 from the concept holds.
2. **Staleness is low-noise.** Across 173 source-changing commits, the median commit marks 3 of 33
   capabilities stale (90th percentile 8). 81 of 98 `Fixes #` commits touched a mapped capability.
3. **Hand-written code claims are the weak point.** 73% of commits change a source file no capability
   claims, mostly hub files (`game.js` ×42, `versus-play.js` ×33, `lib.rs` ×31). A planted off-by-one in
   `rules.js` was caught by "Hits land as loud as they are", but "Fights are deterministic" stayed green
   because its globs don't include `rules.js`.
4. **Per-test coverage fixes recall, and two tiers keep the noise down.** Replaying all 173 commits:

   | Claims | Direct hits per commit (p90) | Indirect | Commits with unclaimed files | Bug fixes seen |
   |---|---|---|---|---|
   | Globs only | 3 (8) | 0 | 73% | 81 / 98 |
   | Per-test coverage, two tiers | 3 (9) | 7 | 28% | 97 / 98 |

   Without tiers, coverage marks a median 10 capabilities per commit: the game is tightly coupled, and
   most tests run the sim. A "shared files" cutoff didn't help (still 10). Ranking by how widely a file is
   run did. The planted `rules.js` bug now reads: 1 capability hit directly, 10 more run the changed code.
5. **Coverage can hide a missing capability.** The extra-life ad feature (#402-#407) stopped looking
   unclaimed once traced, because boot code runs `ads.js` in 24% of tests. Hence the "only run along
   the way" check, which flags it. 54% of commits change such a file (`game.js` ×42, `versus-play.js` ×33,
   `dom.js` ×21, `waves.js` ×21): each is either a missing capability or one an existing one should claim.
   Coverage turns "no idea" into a short to-do list, but a human or agent still has to decide.
6. **What's still unclaimed is Rust and boot code** (`lib.rs` ×31, `config.rs` ×15, `boot.js` ×9). Rust
   coverage needs an instrumented `cargo llvm-cov` build, which didn't fit in free RAM here.
7. **"Blocked" must be a separate state from "failing".** 4 golden tests failed on Node 22; the hashes are
   made on Node 24. Pinning Node 24 in the runner made them pass. Without a `requires` check, a tool like
   this reports environment problems as regressions.
8. **Evidence reuse matters more than running.** The API and e2e suites need more RAM than was free, so
   they came from CI logs. That made the map useful on this machine at all.
9. **Traces go stale too.** Traces are taken at one commit; after a big refactor, re-run `proof trace`.
   Changing claims (re-tracing) also makes older results stale until they're re-run, which is correct
   but should be explained in the UI.

## Found along the way

- GitHub Actions is disabled on the repo. The last CI run was 2026-10-12 at `94349a4`. 188 commits
  have landed since without CI, though the README says every push is checked.
- The golden tests need Node 24.14 (`.node-version`); the default `node` on this machine is 22.20.
