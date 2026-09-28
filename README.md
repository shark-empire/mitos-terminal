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
  theme.rs       the Theme Engine: 9 presets, light/dark variants, WCAG helpers
  render.rs      Term -> egui painting (run-coalesced, damage-aware)
  fx.rs          theme effects: glow borders, glass surface, vignette, HUD ticks,
                  ambient backdrop, plus the cinematic layer (rain, scanlines, sweep)
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
name = "futuristic-scifi"     # futuristic-scifi | cyberpunk | glass-neon | minimal-dark
                               # | light | classic | retro-crt | high-contrast-dark
                               # | high-contrast-light | custom
follow_system = true          # let home.conf's theme_mode swap Sci-Fi <-> Light
# accent = "#46b8e8"          # any of accent/foreground/background/selection/
                               # border/surface/glow_color overrides the preset

[appearance]                  # live sliders; a preset only seeds these
blur = 0.35                   # simulated frosted-glass strength, 0..1
glow_intensity = 0.35
corner_radius = 10.0
tab_style = "rounded"         # rounded | underline | boxed
status_style = "breadcrumb"   # hidden | minimal | breadcrumb | segmented

[effects]
enabled = true
rain = false                  # matrix code rain (opt-in); omit to follow home.conf

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

## Themes

One terminal, any visual style. The **Theme Engine** (`theme.rs`, `fx.rs`,
and the theme half of `config.rs`) owns every colour and effect; the
**Terminal Engine** (`term/`, `pty.rs`, `session.rs`, `input.rs`) never
imports it. Switching themes only changes what gets painted — never the
shell, the PTY, or any terminal behaviour.

| Preset | Character |
|---|---|
| **Futuristic Sci-Fi** (default) | Dark blue-black, restrained cyan glow, soft glass, small HUD corner ticks |
| **Cyberpunk** | Charcoal, controlled magenta + cyan, faint technical grid |
| **Glass Neon** | Frosted glass, low-intensity multi-hue glow, most transparent |
| **Minimal Dark** | Flat and quiet; no glow, blur or grid |
| **Light** | Bright, soft colours for daytime |
| **Classic Terminal** | Plain black, standard 16 colours, zero effects |
| **Retro CRT** | Green phosphor, scanlines, soft screen-edge falloff |
| **Custom** | Blank slate — every colour and effect overridable |

Each preset carries *suggested* glow/blur/opacity/corner-radius/tab/status
settings that are copied into your live settings when you pick it, after
which the sliders in **Settings → Appearance** are independent of the
preset ("I love Cyberpunk but want less glow" is just a slider). Every
colour override under **Advanced colours** works on any preset, not just
Custom. Accessibility never forces the effects on you: reduced motion,
high contrast, low-glow, disable transparency, disable blur, and a forced
light/dark reading of any theme are all in **Settings → Accessibility**,
and win over the saved sliders without overwriting them.

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
