# mitos-terminal

MITOS's terminal emulator — an `egui`/`eframe` app that owns a PTY
(`pty.rs`), parses what comes out of it (`grid.rs`, via `vte`), and
renders it (`main.rs`). Not a shell itself; it hosts one, the same way
Alacritty hosts zsh or Windows Terminal hosts PowerShell.

See [`INTEGRATION.md`](INTEGRATION.md) for how this fits into the rest
of MITOS — that document leads with an honest status table of what's
actually implemented versus still planned, since a fair amount of it
started as a roadmap rather than a description of working code.

## What's real today

- **Runs `mitos-shell`** if it's on `$PATH`, falls back to `sh`
  otherwise (`pty.rs`) — so the terminal still opens even before
  `mitos-shell` exists on a given system.
- **Renders inline widgets** (buttons, progress bars) via a small
  custom escape-sequence protocol ("MROP") — `grid.rs`'s `osc_dispatch`.
- **Serves a Unix-socket IPC API** (`main.rs`'s `spawn_ipc_server`) so
  another process — `mitos-system-monitor`, say — can read the terminal's
  buffer or inject a widget into it.
- **Suggests an install when a command isn't found** (`pkg_bridge.rs`):
  watches PTY output for a "not found"-shaped line, and if
  `mitos-pkgd` has an exact match for the attempted command, renders an
  inline "📦 Install X" button using the same widget machinery above —
  works today regardless of which shell is actually running.
- **Applies a theme sent over IPC** (`TerminalGrid::apply_theme`) —
  reachable today, though nothing currently calls it, since that needs
  `mitos-settings` to actually send one.

## Building

```sh
cargo build --release
```

Needs a `mitos-pkgd` reachable at `mitos-pkg`'s default socket path for
the install-suggestion feature to do anything — without one, `mitos-pkg`
lookups just silently return nothing (see `pkg_bridge.rs`), the rest of
the terminal works the same either way.
