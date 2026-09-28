# Feature audit vs. the original spec

Status of every bullet in the spec this rewrite was built against. `✅` means
implemented and unit- or integration-tested; `⚠️` means implemented as an
honest best-effort with a documented limitation; nothing is silently stubbed.

## Core
| Feature | Status | Where |
|---|---|---|
| PTY create/manage/resize | ✅ | `pty.rs` (`spawn`, `resize`), `session.rs` |
| stdin/stdout/stderr streaming | ✅ | `session.rs` reader/writer threads (stderr is merged into the PTY, as it is for every real terminal) |
| Pseudo-terminal resize | ✅ | `pty::resize`, `Session::resize`, `Term::resize` (full reflow) |
| Process lifecycle tracking | ✅ | `session::SessionState`, `pty::ExitInfo` |
| Foreground/background process handling | ✅ | `pty::foreground_pgid/has_foreground_job/foreground_name` via `/proc` |
| SIGINT/SIGTERM/SIGHUP/SIGKILL | ✅ | `pty::signal_foreground/signal_group/signal_pid`, `Session::sigint/sigterm/sigkill/shutdown` |
| UTF-8 | ✅ | `vte` + `term/tests.rs` (split-across-chunks, invalid-byte tests) |
| ANSI/VT escape sequences | ✅ | `term/perform.rs` (C0/ESC/CSI/OSC), `term/tests.rs` |
| Colors, 256/true-color | ✅ | `term/perform.rs::sgr`, `theme.rs` |
| Cursor styles | ✅ | DECSCUSR, `config::CursorCfg`, `render::paint_cursor` |
| Alternate screen | ✅ | `term/ops.rs::enter_alt/leave_alt`, own scrollback rules |
| Scrollback | ✅ | capped `VecDeque<Row>`, `config::ScrollCfg` |
| Hyperlinks | ✅ | OSC 8 + plain-URL detection (`term/select.rs`), policy-gated open (`security.rs`) |
| Bracketed paste | ✅ | DECSET 2004, `security::prepare_paste` |
| Mouse reporting | ✅ | X10/Normal/Button/Any × default/UTF-8/SGR/urxvt (`input.rs`) |
| OSC sequences | ✅ | 0/1/2/4/7/8/9/10/11/12/52/104/110/111/112/133/777 + MITOS-private ones |
| Bell handling | ✅ | visual/audible/urgent, rate-independent of terminal flood (`term/mod.rs` events cap) |
| Title/icon updates | ✅ | OSC 0/1/2, title stack (`CSI 22/23 t`) |
| Terminal capability reporting | ✅ | DA1/DA2/DSR/DECRQM — deliberately **not** implemented: title report (`CSI 21 t`), ENQ answerback, DECRQSS, resize/move requests (classic injection vectors) |

## UX
| Feature | Status | Where |
|---|---|---|
| Tabs | ✅ | `layout.rs::Layout` (tabs) |
| Split panes | ✅ | `layout.rs::Node` (binary tree, X/Y axis) |
| Windows | ✅ | `Action::NewWindow` re-execs the binary; each OS window is a separate process |
| Searchable scrollback | ✅ | `Term::search`, `Ctrl+F` |
| Copy/paste, selection | ✅ | simple/word/line/block selection (`term/select.rs`), paste sanitising (`security.rs`) |
| Context menu | ⚠️ | not implemented — right-click currently does nothing. Everything it would expose (copy/paste/clear/reset) is reachable via the command palette (`Ctrl+Shift+P`) or keybindings today |
| Profiles | ✅ | `config::Profile`, `[[profile]]` in `terminal.toml` |
| Default shell selection | ✅ | `pty::resolve_shell` (explicit → `mitos-shell` → `$SHELL` → `/bin/sh`) |
| Working-directory inheritance | ✅ | OSC 7 (`Term::cwd`) — new tabs/panes don't yet inherit it automatically (see Limitations) |
| Command history integration | ✅ | OSC 133 command records (`Term::commands`), prev/next-prompt jump |
| Zoom | ✅ | font size (`Ctrl+`/`Ctrl-`), pane zoom/maximize (`Layout::toggle_zoom`) |
| Font selection | ⚠️ | config accepted and round-trips; only the bundled monospace font actually renders — see Limitations |
| Ligatures | ⚠️ | config flag exists; egui's text layer has no OpenType shaper, so ligature glyphs never substitute — see Limitations |
| Themes | ✅ | 7 aesthetic presets + 2 high-contrast variants + Custom, each with its own glow/glass/corner/effects personality — see **Theme Engine** below |
| Transparency, glass, opacity | ✅ | `window.opacity`, transparent viewport, per-pane layered glass surface (`fx::paint_glass_surface`) over an ambient backdrop, with a near-opaque legibility floor behind the text grid |
| Background blur | ⚠️ | *simulated* frosted-glass strength (`appearance.blur`) is fully implemented and themed; a real compositor blur of the desktop behind the window is not — see Limitations |
| Notifications | ✅ | OSC 9/9;4/777, rate-limited (`NotifyCfg`), desktop `notify-send` fallback |
| Command completion integration | ✅ | ghost-text autocomplete via `mitos-file-manager` IPC (`ipc::request_autocomplete`) |

## Rendering
| Feature | Status | Where |
|---|---|---|
| Efficient text grid | ✅ | run-coalesced painting, one draw call per same-attribute span (`render::coalesce_row`, unit-tested) |
| GPU acceleration where practical | ✅ | via egui/eframe's own GPU-backed tessellator; see Limitations for what that means concretely |
| Glyph atlas, font cache | ✅ | egui's own `Fonts` (one atlas texture, cached across frames) |
| Damage tracking | ✅ | `Term::take_damage` (per-row dirty), idle panes stop requesting repaints (`ctx.request_repaint_after`) |
| Smooth scrolling | ✅ | `general.smooth_scroll` + wheel-based `Term::scroll_display` |
| Cursor rendering | ✅ | block/underline/bar, blink, hollow-when-unfocused |
| Selection rendering | ✅ | `render::paint_pane` |
| Fallback renderer | ✅ | `main.rs` retries once with `LIBGL_ALWAYS_SOFTWARE=1` if hardware init fails |
| HiDPi support | ✅ | inherited from egui (logical points throughout; no raw-pixel assumptions in `render.rs`) |

## Security
| Feature | Status | Where |
|---|---|---|
| No arbitrary privilege escalation | ✅ | nothing in this codebase elevates privileges; IPC peers are verified same-uid |
| Safe OSC handling | ✅ | length/count caps on links/clipboard/title/widgets, runaway-OSC cancellation (`term/mod.rs`'s `OscGuard`) |
| Configurable hyperlink behavior | ✅ | `security::LinkPolicy` (off/click/ctrl-click, scheme allowlist, mismatch/always confirm) |
| Paste protection | ✅ | `security::prepare_paste` (control-char stripping, size caps, multiline-into-non-bracketed confirm) |
| Application permission boundaries | ✅ | `term::Policy` (title/clipboard/notifications/widgets/cwd/hyperlinks/colors, all per-terminal.toml) |
| Secure clipboard behavior | ✅ | OSC 52 **write** only — a read request (`?`) is never answered (`term/tests.rs::osc52_write_allowed_read_denied`) |
| Resource limits | ✅ | scrollback cap, link/cluster/widget/command-history table caps, IPC client cap (8) + 60s idle timeout, paste size cap |
| Crash isolation | ✅ | `session::reader_loop` runs `Term::process` inside `catch_unwind` without poisoning the mutex (`session.rs` test proves the technique) |

## Accessibility
| Feature | Status | Where |
|---|---|---|
| Scalable text | ✅ | font-size zoom, no hardcoded pixel sizes |
| High contrast | ✅ | two built-in high-contrast themes + `min_contrast` enforcement (`a11y::enforce_min_contrast`) |
| Screen-reader compatibility | ✅ | a real, transparent `egui::Label` carrying the visible screen text sits over the painted grid, so it gets a normal accessibility node for free from whatever AT backend egui/eframe is built with — see Limitations for the precise scope of "for free" |
| Keyboard navigation | ✅ | every dialog/palette/tab uses real focusable egui widgets |
| Reduced motion | ✅ | `accessibility.reduced_motion` (+ `$MITOS_REDUCED_MOTION`) disables cursor blink and all fx animation |
| Configurable cursor | ✅ | shape/blink/thickness/unfocused style |
| Audible/visual bell | ✅ | `a11y::ring_bell` (desktop bell sound, falls back to `/dev/tty` BEL), visual flash |

## Testing
| Feature | Status | Where |
|---|---|---|
| VT compatibility tests | ✅ | `term/tests.rs` |
| PTY tests | ✅ | `pty.rs` (spawn, env, resize, signals, exit status) |
| Resize tests | ✅ | `term/tests.rs` (reflow, wrap, alt-screen, extremes), `session.rs` |
| Unicode tests | ✅ | `term/tests.rs` (widths, wide chars, combining marks, invalid UTF-8, chunk-split UTF-8) |
| Rendering tests | ✅ | `render.rs` (run-coalescing — the part of rendering that's pure logic and can be unit-tested without a GPU) |
| Shell integration tests | ✅ | `tests/integration.rs` (embedded bash/zsh/fish scripts asserted well-formed); the bash script was also behaviourally verified against a real PTY during development (OSC 133 A/B/C/D sequencing) |
| Stress tests | ✅ | `term/tests.rs::random_bytes_and_escape_soup_never_break_invariants` (seeded fuzzer, structural invariants checked every round), `::flood_of_output_stays_bounded` |
| Long-running session tests | ✅ | `session.rs`, `tests/integration.rs::long_running_session_survives_bursty_output` |

## Known limitations (honest best-effort, not silently stubbed)
- **Font family selection** (`[font] family`) is accepted and saved, but every
  family currently renders with egui's bundled monospace font. Real family
  loading needs a fontconfig lookup + `egui::Context::set_fonts`, which isn't
  wired up. Ligatures are blocked on the same gap plus an OpenType shaper
  egui doesn't have.
- **Background blur** is *simulated*, not a real blur of what's behind the
  window: `appearance.blur` drives a translucent glass surface painted
  over a soft ambient backdrop (`fx::paint_ambient_backdrop`), which is what
  gives Glass Neon and Futuristic Sci-Fi their frosted look. Truly blurring
  the *desktop* behind the window needs a platform-specific compositor
  request (`_KDE_NET_WM_BLUR_BEHIND_REGION` on X11, an equivalent Wayland
  protocol elsewhere) that this rewrite didn't implement rather than guess at.
- **Wallpaper image** (`appearance.wallpaper_path`) is accepted and saved
  but not yet decoded/drawn; the procedural ambient backdrop is always used.
  Loading an arbitrary image would add an image-decoding dependency this
  project doesn't currently take on.
- **PRIMARY selection** (`[selection] primary_selection`,
  `middle_click_paste`) works *within* this app (select in one pane,
  middle-click in another) but doesn't publish to X11's real PRIMARY
  selection, so a *different* application's middle-click paste won't see it.
- **Split-pane dividers** aren't draggable yet; new splits are always 50/50.
- **New panes/tabs** don't inherit the previous pane's OSC-7 working
  directory yet — they start in the profile's configured directory (or
  `$HOME`).
- **`--tty` recovery mode**: this is an `eframe`/OpenGL GUI application: it
  cannot run on a display-less virtual console. A real recovery path would
  be a separate, text-mode-only binary, not a flag on this one.
- **Screen-reader support** depends on `eframe` having been built with
  AccessKit support compiled in; this rewrite adds a normal, standard
  `egui::Label` (which AccessKit picks up automatically when present) rather
  than hand-building a platform accessibility tree, but does not itself
  control whether that backend is compiled into this build of `eframe`.

## Theme Engine

Implements the "one terminal, any visual style" design brief. The
architecture matches its diagram: the **Terminal Engine** (`term/`,
`pty.rs`, `session.rs`, `input.rs`) never imports `theme`, `fx` or
`render`; the **Theme Engine** (`theme.rs`, `fx.rs`, and the theme-related
half of `config.rs`) owns every colour and effect; **Terminal UI**
(`render.rs`, `layout.rs`, `app.rs`) is the only layer that reads both.
Switching themes changes what gets *painted* — never the shell, the PTY,
or any terminal behaviour (`config::Config::apply_theme_preset` touches
only appearance settings).

| Brief item | Status | Where |
|---|---|---|
| Futuristic Sci-Fi (default) | ✅ | `Theme::futuristic_scifi` — dark blue-black, restrained cyan glow, soft glass, HUD corner ticks |
| Cyberpunk | ✅ | `Theme::cyberpunk` — charcoal, controlled magenta + cyan, faint grid, underline tabs, segmented status |
| Glass Neon | ✅ | `Theme::glass_neon` — heaviest simulated blur, lowest opacity, low-intensity multi-hue glow |
| Minimal Dark | ✅ | `Theme::minimal_dark` — flat, no glow/blur/grid, quiet slate accent |
| Light | ✅ | `Theme::light` |
| Classic | ✅ | `Theme::classic` — plain black, textbook 16-colour palette, zero effects |
| Retro CRT | ✅ | `Theme::retro_crt` — green phosphor, scanlines, vignette, soft glow |
| Custom | ✅ | `Theme::custom_base` + generic per-colour overrides (`[theme]` accent/foreground/background/selection/border/surface/glow_color) that work on *any* preset, not just Custom |
| Theme / colour scheme / accent / font / size | ✅ | Settings → Theme, Advanced colours, Font & cursor |
| Opacity, background blur, corner radius, glow, cursor style/blink, animations | ✅ | Settings → Appearance / Font & cursor; live values in `[window]`/`[appearance]`/`[cursor]`/`[effects]` |
| Wallpaper | ⚠️ | procedural ambient backdrop (toggle in Appearance); image path accepted but not rendered — see Limitations |
| Tab appearance | ✅ | `theme::TabStyle` (rounded/underline/boxed), rendered by `app.rs::paint_tab_decoration` |
| Prompt style | ✅ | `theme::StatusStyle` — a terminal-drawn `user@host`/cwd strip (hidden/minimal/breadcrumb/segmented), independent of the shell's `$PS1`; drawn in the padding margin so it never costs grid space, and skipped automatically when padding is under 12px |
| Presets | ✅ | `theme::THEME_PRESETS`; picking one seeds the live sliders (`Config::apply_theme_preset`) rather than owning them, so per-slider tweaks stay independent |
| Reduced motion / disable animations | ✅ | `accessibility.reduced_motion`, `effects.enabled` |
| High contrast | ✅ | `accessibility.high_contrast` → hand-tuned palettes |
| Low-glow mode | ✅ | `accessibility.low_glow` → `Config::effective_glow` (hard cap, non-destructive) |
| Disable transparency / blur | ✅ | `accessibility.disable_transparency`/`disable_blur` → `Config::effective_opacity`/`effective_blur` (win over the saved sliders without overwriting them) |
| Light and dark variants of *every* theme | ✅ | `accessibility.color_scheme` → `Theme::as_light_variant`/`as_dark_variant`, a contrast-verified algorithmic derivation rather than 14 hand-authored palettes |
| Adjustable font size / opacity | ✅ | Settings sliders + `Ctrl+=`/`Ctrl+-` |
| OSC 10/11/12 colour queries reflect the active theme | ✅ | `Term::set_theme_colors`, synced on creation, config reload, settings change and IPC `ThemeChanged` (this was never actually wired before) |

Design restraint is enforced by tests, not just intent
(`theme::tests::brief_mandated_restraint_is_actually_restrained`,
`every_preset_meets_aa_contrast_for_body_text`): no shipped preset exceeds a
glow of 0.6, a border of 2px or a vignette of 0.4, and every preset's
default text clears WCAG AA. The ambient backdrop is deliberately
time-independent — no animation — for the same reason ("avoid constant
animations") and because it means painting it never forces a repaint.

Behaviour change worth knowing about: Matrix code rain (from the original
cinematic layer) is now **opt-in** rather than on by default, since none of
the presets in the brief call for it and the brief asks for something
comfortable over hours of use.
