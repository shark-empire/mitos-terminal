# Settings review (first macOS test)

Reported: "some settings are not working correctly or not at all on a Mac". This is what the code
review found. **Nothing here has been compiled or run** (the review environment has no Rust
toolchain and no Mac), so the fixes were written by reading the code and checked by hand; see
"Check on the Mac" at the end.

## 1. Settings that did nothing, and why

| Setting | What you saw | Cause | Now |
|---|---|---|---|
| `terminal.toml` edits (everything) | Nothing changed | On macOS `config_dir()` was `~/Library/Application Support/mitos`, while the README, the example config and the Settings footer all say `~/.config/mitos`; a file in the second place was silently ignored (`config.rs::config_dir`) | `~/.config/mitos` is used on macOS too (and `$XDG_CONFIG_HOME`); an existing Application Support config is still honoured when it is the only one. The Settings footer shows the real path |
| A mistake in `terminal.toml` | Every setting back to default, no message | `Config::load` returns defaults plus an error; `main.rs` dropped it and the app only listed it at the bottom of the Settings window. The README's "or stderr on startup" was not true | A dismissible bar at the bottom of the window, plus stderr, on load and on hot-reload |
| Settings window changes not saved | Gone after restart | `let _ = self.config.save()` ignored every error | The error is shown in the same bar |
| **Opacity** | Slider did nothing visible | `render::paint_pane` painted an *opaque* ambient backdrop (`fx::paint_ambient_backdrop`), then a 232/255 "legibility floor" over the text grid, and `app.rs` added a third translucent layer under both | The text grid has one background layer at the window opacity; the glass look is confined to the padding margin; the backdrop respects the opacity |
| **Background blur** | Slider did nothing visible | Same layers (only the 8 px margin ever showed it). It is also a simulated tint, not a real blur | Visible in the margin and, with opacity < 1, through the window. Still not a real blur (needs macOS vibrancy) |
| **Glow intensity** | Barely visible | The glow strokes were drawn centred on / outside the pane edge, which is outside the painter's clip rect: only a half-pixel line survived | Drawn just inside the edge, in the padding margin |
| **Vignette** | Nothing | Painted before the opaque layers, so hidden | Painted over the grid background |
| **Prompt / status style** | Never appeared | The strip is only drawn when padding ≥ 12 px; the default padding is 8 and the default style is `breadcrumb` | Padding is raised to 16 px whenever a style other than `hidden` is selected (`Config::effective_padding`), with a hint under the slider |
| Status strip text | `user` only, `~` always | Host name and the shell's working directory both came from Linux-only sources (see 2) | `gethostname`, working directory via OSC 7 / `lsof` |
| `[cursor] unfocused` | "hollow" hid the cursor, "same" drew it hollow | Inverted test in `paint_pane`, and it used pane focus instead of window focus | "hollow" outlines, "same" stays filled, window focus counts |
| Follow system light/dark | Never followed macOS | Only read MITOS's `home.conf` | Reads the macOS appearance (`defaults read -g AppleInterfaceStyle`) and re-checks every 3 s |
| `[selection] word_separators` | No effect | Never copied into the `Term` | Applied to new panes and on reload |
| `[scrollback] lines` | Only new tabs | Read once when a pane is created | Applied to open panes on reload |

## 2. macOS-specific problems

| Area | Cause | Now |
|---|---|---|
| Links (`[links] mode = "ctrl_click"`, the default) | macOS turns Ctrl+click into a right-click, so it could never fire | `Cmd+click` on macOS |
| Typing with Option | Any text typed with Alt/Option was ESC-prefixed, so `@ \| [ ] { } \\ ~` on many non-US layouts and `å ∫ ç` on US came out as `ESC` + character | Option composes text by default; new `[general] option_as_meta` (Settings checkbox) makes it Meta |
| Command key | `Chord` had no `cmd`; every default was Ctrl-based | `cmd` accepted in `[keybindings]`; Cmd+T/N/W/D/F/G/K/,/1–9 etc. by default on macOS. `Alt+1`–`9` and plain `Ctrl+F/G/=/-` are not bound by default on macOS (Option+digit types characters; the Ctrl keys are readline's forward-char / abort / undo) |
| Ctrl+Shift+V | Pasted on Linux only by accident (egui treats Ctrl as Command there) | Handled explicitly |
| `Ctrl+C copies when text is selected` | Default on; on macOS that turns Ctrl+C into a copy | Default off on macOS |
| Notifications, audible bell, bell urgency, speech | `notify-send`, freedesktop sound files, `wmctrl`, `spd-say` | `osascript` (arguments passed as `argv`, not spliced into the script), `afplay`, Dock bounce, `say` |
| Working directory of new tabs, "close running program?" | `/proc` | `lsof` / `ps` |
| OSC 7 working directory from the shell | The host name in `file://host/path` was compared with `/proc/sys/kernel/hostname`, so every report on macOS looked remote and was dropped | `gethostname` |
| `shell_integration` | Only bash was ever set up automatically; zsh is the macOS default | zsh gets it through a private `ZDOTDIR` whose startup files run your own first (macOS only) |
| PATH / locale | Shell not started as a login shell; fallback `LANG=C.UTF-8` is not a macOS locale | `zsh -l` on macOS; `en_US.UTF-8` fallback |
| F11 | "Show Desktop" on a Mac | `Ctrl+Cmd+F` bound by default |

## 3. Found on the way (all platforms)

- **Bound shortcuts were also sent to the shell** (Shift+PageUp scrolled *and* typed `;2~`; Ctrl+Tab
  also sent a Tab; Alt+1 also sent `ESC 1`). They are no longer forwarded.
- **Typing in the find bar, the command palette or a Settings colour box also reached the shell**
  (egui leaves typed text in the input queue for everyone). Pane input is now skipped while one of
  those has focus.
- **Wheel scrolling** divided by the row height instead of by egui's 50-point notch, so a notch moved
  about 3× `wheel_lines` lines and a trackpad ran about 3× faster than the content moved under the
  finger. `wheel_lines` now means lines per notch, as documented.

## 3b. Behaviour changes to know about

- The default look now shows the `breadcrumb` status strip (it was configured but never appeared)
  and therefore has a 16 px margin instead of 8 px. `status_style = "hidden"` brings the old look
  back (the shipped `.config/mitos/terminal.toml` example already says so).
- The vignette and the HUD corner ticks are now visible over the text area instead of only in the
  margin; the default presets use small values, so this is subtle.
- Light-mode Macs start in the Light preset while **Follow system light/dark** is ticked (it is by
  default); untick it to keep the Sci-Fi preset.
- On macOS, `Ctrl+C` always interrupts by default (`ctrl_c_copies` is off there) and zsh is started
  as a login shell with the integration loaded.

## 4. Still not working (not changed)

- `[font] family`, `fallbacks`, `ligatures`: only the bundled monospace font renders (already in
  `AUDIT.md`). This is the largest remaining gap; it needs system-font loading.
- Real background blur (macOS `NSVisualEffectView`).
- `appearance.wallpaper_path`, `effects.glitch`, a profile's `theme`, `title` and `font_size`.
- `[window] width/height/decorations` apply at start-up only.
- The Settings window has controls for roughly a third of the keys. Added this round: Option as
  Meta, copy on select, Ctrl+C copies, confirm close, shell integration, bells. Everything else
  (profiles, key bindings, link policy, notification thresholds, scrollback size, ...) is file-only.
- On Linux, egui may turn Ctrl+C / Ctrl+X / Ctrl+V into copy/cut/paste events before they become
  key events, in which case `Ctrl+C` would never send SIGINT. This could not be confirmed without
  running it; worth a 10-second check on the Linux build.

## 5. Check on the Mac

1. `cargo test --no-default-features` and `cargo clippy --no-default-features -- -D warnings`
   (nothing here was compiled).
2. Put a one-line change in `~/.config/mitos/terminal.toml` (e.g. `[window] opacity = 0.6`): it
   should apply live. Break the file on purpose: the orange bar should appear.
3. Settings → Opacity slider: the window should become see-through; Blur/Glow/Vignette should be
   visible; choose a Prompt/status style: the strip should appear with `user@host` and the cwd.
4. `Cmd+T`, `Cmd+D`, `Cmd+,`, `Cmd+click` on a URL; type `@` and `|` on your layout.
5. In zsh: `echo $ZDOTDIR` (should be empty / your own), `echo $HISTFILE` (your usual file),
   `brew --version`; open a new tab after `cd /tmp`: it should start in `/tmp`.
6. `sleep 1; printf '\a'` with Audible bell on; a notification (`printf '\e]9;hello\a'`).
7. Option-as-Meta needs a physical-key fallback in egui-winit 0.28; if `Option+B` prints
   `ESC ∫` instead of moving back a word, tell me and I will switch it to a different mechanism.
