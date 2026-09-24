# mitos-terminal

MITOS's native terminal emulator — an `egui`/`eframe` application that owns
real PTYs, parses ANSI/VT100 output through a from-scratch screen model, and
integrates with the rest of the MITOS ecosystem (`mitos-pkg`, `mitos-shell`,
IPC-connected daemons) while staying fast, secure, and accessible.

This is a from-scratch rewrite audited line-by-line against a full terminal-emulator
spec — see **[`docs/AUDIT.md`](docs/AUDIT.md)** for the complete, honest
feature-by-feature status (what's done, and the handful of things that are a
documented best-effort rather than silently stubbed).

## Layout

```
src/
  term/          the VT/xterm screen model — no GUI, no I/O, fully unit-tested
    mod.rs         Term, events, policy, damage tracking, widgets
    cell.rs        Cell/Color/Pen/Row, attribute & mark bitflags
    modes.rs       terminal modes, cursor style, charsets
    width.rs       Unicode cell-width table
    ops.rs         screen-editing primitives (print, scroll, erase, ...)
    perform.rs     vte::Perform — the actual escape-sequence interpreter
    reflow.rs      resize / logical-line reflow
    select.rs      selection, text extraction, word/line bounds, URL detection
    search.rs      scrollback search
    tests.rs       VT-compatibility, Unicode, resize, selection, fuzz tests
  pty.rs         spawn/resize/signal a child on a real PTY, shell discovery
  session.rs     one PTY + one Term + I/O threads + lifecycle + crash isolation
  input.rs       keyboard/mouse -> xterm-compatible byte encoding
  security.rs    link policy, paste sanitising, MROP command gating
  config.rs      terminal.toml schema, keybindings, action table
  theme.rs       palettes, WCAG contrast helpers
  render.rs      Term -> egui painting (run-coalesced, damage-aware)
  fx.rs          cinematic effects (matrix rain, scanlines, phosphor glow, sweep)
  layout.rs      tabs & split panes (pure logic, no egui types)
  a11y.rs        announcements, speech, audible bell, contrast enforcement
  ipc.rs         Unix-socket server + clients for the MITOS ecosystem
  platform.rs    clipboard, desktop notifications, urgency hint
  pkg_bridge.rs  "command not found" -> mitos-pkgd install-button bridge
  app.rs         the eframe::App: ties everything above into a running UI
  main.rs        binary entry point (NativeOptions, fallback renderer)
tests/
  integration.rs crate-boundary tests: VT session, config persistence,
                  layout+security workflow, a real long-running session
shell-integration/
  mitos.bash, mitos.zsh, mitos.fish   OSC 7 / OSC 133 hooks, embedded into
                                       the binary and written out at runtime
docs/
  AUDIT.md       full spec-compliance table + known limitations
```

## Building

```sh
cargo build --release
cargo test
```

`mitos-utils` and `mitos-pkg` are pulled from their MITOS git repos (see
`Cargo.toml`); if you're developing against local checkouts, switch those to
`path = "../mitos-utils"` / `path = "../mitos-pkg"` before building.

## Configuration

`~/.config/mitos/terminal.toml` — every table is optional; a partial or
missing file just falls back to defaults, and a broken one is reported
without crashing (see the in-app Settings window, or stderr on startup) and
falls back to the last-known-good config. Every setting in the file is also
live-editable from the Settings window (`Ctrl+,`), which writes changes back
to this file.

```toml
[general]
default_profile = "default"
shell_integration = true      # inject OSC 7/133 hooks into bash automatically

[font]
size = 14.0

[theme]
name = "mitos-dark"           # mitos-dark | mitos-light | high-contrast-dark
                               # | high-contrast-light | solarized-dark
follow_system = true          # follow ~/.config/mitos/home.conf's theme_mode

[effects]
enabled = true
rain = true                   # matrix code rain; omit to follow home.conf

[security]
osc52_write = true            # let programs write the clipboard (never reads)
mrop_trusted_prefixes = []    # command prefixes that skip the click-to-run confirm

[[profile]]
name = "work"
shell = "/usr/bin/fish"
cwd = "~/work"
```

`~/.config/mitos/home.conf` (shared system-wide look & feel) is still read
for `theme_mode`, `accent_color`, `matrix_rain` and `reduced_motion` —
`terminal.toml` wins wherever both speak. The `[theme] follow_system`
convenience above is how the Matrix-rain toggle in Settings persists back
to `home.conf` rather than `terminal.toml`, so it stays in sync with the
rest of MITOS.

Both files are watched for changes and hot-reloaded.

## Keyboard shortcuts

Every binding below is user-remappable via `[keybindings]` in
`terminal.toml` (`"ctrl+shift+t" = "new_tab"`, or `"none"` to unbind); this
is just the shipped default. The in-app command palette (`Ctrl+Shift+P`)
lists every action with its current shortcut and lets you run any of them
without memorising a chord.

| Shortcut | Action | | Shortcut | Action |
|---|---|---|---|---|
| `Ctrl+Shift+T` | New tab | | `Ctrl+Shift+F` / `Ctrl+F` | Find in scrollback |
| `Ctrl+Shift+N` | New window | | `Ctrl+G` / `Ctrl+Shift+G` | Find next/previous |
| `Ctrl+Shift+W` | Close pane | | `Ctrl+=` / `Ctrl+-` / `Ctrl+0` | Zoom in/out/reset |
| `Ctrl+Tab` / `+Shift` | Next/previous tab | | `Shift+PageUp/Down` | Scroll a page |
| `Alt+1`–`9` | Go to tab N / last | | `Shift+Home/End` | Scroll to top/bottom |
| `Ctrl+Shift+D` / `E` | Split right/down | | `Ctrl+Shift+Up/Down` | Jump to prev/next prompt |
| `Alt+Shift+arrows` | Focus pane in direction | | `Ctrl+Shift+K` | Clear scrollback |
| `Ctrl+Shift+Z` | Zoom/maximize pane | | `Ctrl+Shift+R` | Reset terminal |
| `Ctrl+C` (with selection) | Copy | | `Ctrl+Shift+P` | Command palette |
| `Ctrl+V` / OS paste | Paste | | `Ctrl+Shift+H` | Command history |
| `Ctrl+Shift+A` | Select all | | `Ctrl+,` | Settings |
| `Ctrl+Click` | Open link (configurable) | | `F11` | Fullscreen |

## Ecosystem integration

- **MROP (MITOS Rich Object Protocol)** — `OSC MITOS_WIDGET;<json>` lets any
  program inject a real, interactive `egui` widget (button, progress bar,
  sparkline) into the terminal stream; `OSC MITOS_NEW_BLOCK;<prompt>` starts
  a new execution block.
- **Unix-socket IPC** (`terminal_socket(pid)`) — same-uid-only, rate- and
  client-count-limited — lets `mitos-system-monitor`-style daemons read the
  buffer, inject widgets, or push a theme change.
- **`mitos-pkg`** — a "command not found" line gets an inline
  `📦 Install <name>` button by querying `mitos-pkgd` directly.
- **`mitos-shell`** — used automatically when installed (`$PATH` lookup),
  falling back to `$SHELL` and then `/bin/sh`.
- **`mitos-network`** — polled for captive-portal state; injects a
  `🌐 Open Login Page` button when one is detected.
- **`mitos-file-manager`** — ghost-text path autocomplete over IPC.

See `docs/AUDIT.md` for exactly which side of each integration this repo
implements today.
