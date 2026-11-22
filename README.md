# proof

What your app can do, and whether it still does, at this commit.

`proof.toml` lists the capabilities of an application in the words a user would use
("Buy Foil packs", "Cheating is refused"), grouped, each with scenarios that point at tests the
project already has. Results are pinned to the git blob ids of the code each capability claims,
so a result goes stale the moment that code changes, and you can see which.

A coding agent writes and maintains the map. People read it in the desktop app.

## Layout

| Path | What |
|---|---|
| `crates/proof-core` | The model: map, evidence, verdicts, affected, runs, traces, edits |
| `crates/proof-cli` | `proof`, the command line. Every report takes `--json` |
| `crates/proof-app` | The desktop app (GPUI) |
| `skill/SKILL.md` | How a coding agent maps a project and keeps it current (`proof guide`) |
| `design/` | The approved design: foundations, tokens and the screens the app is built to |
| `prototype/` | The Python prototype the model was proven with |

## Build

```bash
cargo build --release
```

The app needs the GPUI system libraries on Linux (Vulkan, Wayland or X11, xkbcommon, fontconfig).

## Use

```bash
proof status
proof affected --since main
proof run "story runs"
proof-app --map path/to/proof.toml
```

To open a map in the app, point it at any project's `proof.toml`:

```bash
proof-app --map path/to/proof.toml
```

In the app, Ctrl+K (⌘K on macOS) finds a capability or runs any command, and shows the CLI command
that does the same. Ctrl+1, 2, 3 switch between Map, Changes and Runners. J and K move, Enter opens,
Ctrl+R runs what is on screen, Ctrl+Shift+C copies its CLI command.

## States

| | |
|---|---|
| ✓ passing | passed on the code as it is |
| ✗ failing | failed |
| ⚠ stale | passed on code that has changed since |
| ? missing | the test it points at did not run |
| ⊘ blocked | its runner can't run on this machine; the last verdict stands, marked with its commit |
| ○ unverified | nothing has run yet |
