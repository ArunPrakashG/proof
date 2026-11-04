---
name: proof
description: Keep the project's capability map (proof.toml) current and use it to say whether the application still works. Use after implementing or changing a feature, when asked "does X still work?", "verify payments", "what did this change touch?", or to map a project for the first time.
---

# proof: verify capabilities, not endpoints

`proof.toml` lists what the application does, in the user's words, grouped (Accounts, Shop, ...).
Each capability claims the code it depends on (`code` globs) and has scenarios. Every scenario
points at evidence the project already has: a test name (`match`) in one of the `runners`.
Results are pinned to the git blob ids of the claimed code, so a result goes stale when that code changes.

## After changing code

1. `proof affected --json` lists the capabilities your change touches (`direct` true or false), changed source
   no capability claims (`unmapped`), and changed files tests run only along the way (`incidental`).
2. For each unmapped or incidental file: if it belongs to an existing capability, add it to that capability's
   `code`. If it is a new thing the app does, add a capability (see Mapping) with `inferred = true`.
3. `proof run "<capability or group>"` runs only the runners those scenarios need. Run the directly hit
   capabilities first; run the indirect ones too before calling the change verified.
4. If a scenario fails, fix the code and run again. A blocked runner (⊘) is not a failure: say why
   it could not run, and do not report the capability as verified.
5. Report with `proof status <group>`. Quote failing scenario names verbatim, not test output.

## Mapping a project for the first time

- Read routes, modules and the existing tests. Name capabilities as a user would describe them
  ("Buy Foil packs"), not as endpoints ("POST /foil/checkout"). Aim for 5-40 capabilities, not one per route.
- Each scenario must point at a test that exists today. Do not write new tests while mapping;
  a capability with no evidence is a finding to report, not something to paper over.
- Claim narrowly: the files that implement this capability, not shared plumbing (router, config, error types).
- Mark every capability you write `inferred = true`. Only a human removes the flag.
- If a runner can trace (`coverage = "node"`), run `proof trace` so claims come from what tests execute,
  then `proof run` once, since results recorded before the trace go stale when claims grow.
- Finish with `proof gaps` and report the unclaimed source directories.

## Never

- Never edit a `match` string to make a missing scenario pass. Missing (?) means the test it pointed at is gone or renamed.
- Never mark a capability verified from a run of a different commit without saying which commit.
