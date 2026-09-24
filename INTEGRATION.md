> ## Implementation status (read this first)
>
> Updated after the September 2026 rewrite (tabs/splits, hardened IPC, real
> shell-integration scripts, security policy layer, accessibility). Current state:
>
> | Integration | Status |
> |---|---|
> | `mitos-shell` handoff, `MITOS_TERMINAL_VERSION`/`MITOS_TERM_VERSION` env vars | **Done** — `pty.rs::resolve_shell` checks `$PATH` for `mitos-shell` first, then `$SHELL`, then `/bin/sh`. `MITOS_MROP_SUPPORTED` is not currently set (nothing reads it yet; MROP support is unconditional on the terminal's side regardless). |
> | Real shell integration (OSC 7 working directory + OSC 133 prompt/command marks) for **bash, zsh and fish** | **Done, and new.** `shell-integration/mitos.{bash,zsh,fish}` are embedded in the binary and written to a per-user cache dir at startup (`pty::ensure_integration_files`); bash gets it automatically via `--rcfile`, zsh/fish need one `source` line (see `README.md`). This is what powers prev/next-prompt jump, the command-history list, and "command finished" notifications — none of which existed in the pre-rewrite build. |
> | MROP widget rendering (`OSC_WIDGET`) + block finalization (`OSC_NEW_BLOCK`) | **Done** — now in `term/perform.rs::osc_mitos`, policy-gated (`[security] widgets`), with a click-to-run confirmation for anything not in `mrop_trusted_prefixes` (`security::sanitize_command`/`is_trusted_command`). |
> | `mitos-system-monitor`-style buffer scraping + widget injection over the IPC socket | **Done, and hardened.** `ipc.rs`: same-uid peer check (`SO_PEERCRED`), socket mode `0600`, max 8 concurrent clients with a 60s idle timeout, and buffer reads can be switched off entirely (`[security] ipc_buffer_read`). |
> | `mitos-pkg` "command not found" install button | **Done, from the terminal's side, not `mitos-shell`'s** — unchanged design from before: the terminal watches PTY output for a "not found"-shaped line and queries `mitos-pkgd` directly (`pkg_bridge.rs`), off the UI thread. Works with any shell. |
> | `mitos-settings` theme sync | **Done, from the terminal's side.** `ThemeChanged` over IPC now takes effect the same frame it arrives (`ipc::Registry::take_theme_override`, applied in `app.rs::resolve_theme_overrides`) — still nothing to send it yet, since `mitos-settings` connecting to this socket is outside this repo. |
> | `mitos-file-manager` ghost prompts / semantic clipboard | **Client-side plumbing exists; protocol is still unverified.** `ipc::request_autocomplete` connects to `file_manager_socket()` and sets ghost text from the first suggestion if a matching daemon answers — but that daemon's actual IPC shape isn't available to build or test against, so this is best-effort until it exists. Semantic (URI-list) clipboard is not implemented. |
> | `mitos-gui` global hotkeys / notifications | **Notifications: done** (`ipc::notify_user`, falling back to `notify-send`). **Global/system-wide hotkeys: not done** — needs `mitos-gui`'s compositor-level API, not available yet. In-app shortcuts (new tab, etc.) work today; only OS-wide hotkeys while unfocused are missing. |
> | `mitos-network` captive portal / bandwidth widgets | **Captive portal: done** — polls `network_socket()` every 3s and injects an "Open Login Page" button (`ipc::spawn_network_poller`). Bandwidth widgets: not implemented (no protocol to build against). |
> | `mitos-kernel` TTY fallback | **Not buildable as described**, unchanged from before: this is an `eframe`/OpenGL GUI application and cannot run on a display-less virtual console. See `docs/AUDIT.md`. |
>
> Also new since the last note: tabs, split panes (with directional focus
> movement), a command palette, a settings window, WCAG contrast enforcement,
> reduced-motion support, crash isolation around the screen model, and a
> from-scratch VT/xterm interpreter (`term/`) replacing the old `grid.rs` —
> see `docs/AUDIT.md` for the full list against the original spec.

---

🔌 3. Integration Points with MITOS
To fully integrate this into your  mitos/  ecosystem, you should implement the following bridges:
	1.	 mitos-settings  Integration:
	•	Read a config file ( ~/.config/mitos/settings.toml ) on startup to load the user’s preferred  current_fg ,  current_bg , and Font Family into  TerminalGrid  and  egui ’s  FontDefinitions .
	2.	 mitos-shell  Handoff:
	•	In  pty.rs , replace  CommandBuilder::new("sh")  with  CommandBuilder::new("mitos-shell") . Pass an environment variable  MITOS_TERMINAL_VERSION=0.1.0  so your shell knows what escape sequences are supported.
	3.	 mitos-system-monitor  Hooks:
	•	Because your  TerminalGrid  is isolated and memory-safe, you can expose a public API (via IPC or shared memory) that allows  mitos-system-monitor  to read the terminal’s buffer for features like “search in terminal” or accessibility screen readers.

(⬆ the three bullets above are the *original design note*, kept verbatim for
history — as the table above shows, all three now have a real implementation:
`config.rs`+`theme.rs` / `pty.rs::resolve_shell` / `ipc.rs`, respectively. The
`TerminalGrid` type they refer to was superseded by `term::Term`.)

---

---

🔌 3. Integration Points with MITOS
To fully integrate this into your  mitos/  ecosystem, you should implement the following bridges:
	1.	 mitos-settings  Integration:
	•	Read a config file ( ~/.config/mitos/settings.toml ) on startup to load the user’s preferred  current_fg ,  current_bg , and Font Family into  TerminalGrid  and  egui ’s  FontDefinitions .
	2.	 mitos-shell  Handoff:
	•	In  pty.rs , replace  CommandBuilder::new("sh")  with  CommandBuilder::new("mitos-shell") . Pass an environment variable  MITOS_TERMINAL_VERSION=0.1.0  so your shell knows what escape sequences are supported.
	3.	 mitos-system-monitor  Hooks:
	•	Because your  TerminalGrid  is isolated and memory-safe, you can expose a public API (via IPC or shared memory) that allows  mitos-system-monitor  to read the terminal’s buffer for features like “search in terminal” or accessibility screen readers.



# MITOS Terminal: Ecosystem Integrations

This document outlines the architectural connections, IPC (Inter-Process Communication) protocols, and data flows between `mitos-terminal` and the rest of the MITOS operating system ecosystem.

Unlike traditional Linux terminals that act as passive text renderers, `mitos-terminal` acts as an **OS-Integrated Workspace**, bridging the gap between CLI tools and the MITOS GUI environment.

---

## 🗺️ High-Level Connection Map

| MITOS Project | Connection Type | Primary Function in Terminal |
| :--- | :--- | :--- |
| **`mitos-shell`** | PTY / OSC Sequences | Sub-process execution, Execution Block triggers, MROP UI injection. |
| **`mitos-system-monitor`** | Unix Domain Sockets | Buffer scraping for global search, MROP Sparkline/Graph rendering. |
| **`mitos-file-manager`** | IPC / Semantic Clipboard | Context-aware "Ghost Prompts", file URI clipboard syncing. |
| **`mitos-settings`** | File Watchers / DBus | Theme syncing, font configuration, keybinding management. |
| **`mitos-pkg`** | OSC / MROP | Interactive "Command Not Found" installation buttons. |
| **`mitos-network`** | MROP / IPC | Captive portal triggers, connection status widgets. |
| **`mitos-gui`** | Wayland/X11 / Global Hotkeys | Window compositing, Quake-style dropdown shortcuts. |
| **`mitos-kernel`** | `/dev/ptmx` / Signals | PTY allocation, `SIGWINCH` (resize) handling, TTY fallback. |

---

## 🔌 1. Core Sub-System Connections

### 🐚 `mitos-shell`
The terminal acts as the host environment for `mitos-shell`. They communicate via standard PTY I/O and custom ANSI/OSC escape sequences.
*   **Environment Variables:** On spawn, the terminal injects:
    *   `MITOS_TERM_VERSION=0.1.0`
    *   `MITOS_MROP_SUPPORTED=1` (Tells the shell it can output GUI widgets)
*   **Execution Blocks:** The shell emits `\x1b]MITOS_NEW_BLOCK;[Prompt]\x07` every time a command finishes, allowing the terminal to wrap the output in a visual "Card".
*   **MROP (MITOS Rich Output Protocol):** The shell outputs JSON payloads via `\x1b]MITOS_WIDGET;{...}\x07` to render native `egui` buttons, progress bars, and toggles directly inline with text.

### 🐧 `mitos-kernel` (Linux PTY Subsystem)
While MITOS user-space is written in Rust, it leverages the underlying Linux kernel's PTY subsystem.
*   **PTY Allocation:** Uses `portable-pty` to interact with `/dev/ptmx` and `/dev/pts/*`.
*   **Signal Handling:** Listens for `SIGWINCH` from the kernel to detect when the window is resized, dynamically recalculating the `TerminalGrid` dimensions and sending the new size to `mitos-shell`.
*   **TTY Fallback:** If `mitos-gui` crashes, `mitos-login` can spawn `mitos-terminal` directly on a raw TTY as a recovery environment.

---

## 🖥️ 2. Desktop Environment (GUI) Layer

### ⚙️ `mitos-settings`
The terminal does not store its own isolated preferences; it acts as a client to the global MITOS settings daemon.
*   **Theme Syncing:** Listens to global MITOS theme changes (Light/Dark mode, Accent Colors). When the user changes their desktop wallpaper or theme, the terminal dynamically updates its `egui::Frame` colors and ANSI palette without restarting.
*   **Configuration:** Watches `~/.config/mitos/terminal.toml` using the `notify` crate for hot-reloading of fonts, opacity, and custom keybindings.

### 📁 `mitos-file-manager`
*   **Context-Aware Ghost Prompts:** When a user types `cd ` or `rm `, `mitos-shell` queries the File Manager daemon via IPC. The terminal renders the results as faint "Ghost Text" or inline icon grids before the user even presses Tab.
*   **Semantic Clipboard:** When a user highlights a list of files in the terminal and presses `Ctrl+C`, the terminal doesn't just copy raw text. It queries the File Manager to resolve the paths and copies them to the system clipboard as `text/uri-list`, allowing direct drag-and-drop into other MITOS GUI apps.

### 🪟 `mitos-gui` (Window Manager / Compositor)
*   **Global Shortcuts:** Registers with the MITOS Window Manager to listen for global hotkeys (e.g., `Super + T` to open a new tab, or `Super + ~` for a Quake-style dropdown terminal overlay).
*   **Notifications:** Uses the MITOS native notification API to alert the user if a long-running background terminal tab finishes a task (e.g., "Build Complete").

---

## 📊 3. System Services & Daemons

### 📈 `mitos-system-monitor`
*   **MROP Visualizations:** CLI tools like `mitos-top` or `mitos-btop` do not draw ASCII art graphs. Instead, they output `RichWidget::Sparkline` and `RichWidget::Progress` sequences, which the terminal renders as high-performance, GPU-accelerated `egui` charts.
*   **Buffer Scraping (Accessibility):** The terminal exposes a read-only Unix Domain Socket (`/tmp/mitos-term-{pid}.sock`). The System Monitor GUI connects to this socket to scrape the terminal buffer, enabling features like "Search across all open terminal tabs" and screen-reader accessibility.

### 📦 `mitos-pkg` (Package Manager)
*   **Interactive "Command Not Found":** If a user types a command that isn't installed, `mitos-shell` queries `mitos-pkg`. Instead of printing `command not found`, it outputs an MROP Button: `[ 📦 Install mitos-code ]`. Clicking it natively triggers the package manager installation flow inline.

### 🌐 `mitos-network`
*   **Captive Portal Integration:** If the network daemon detects a captive portal (e.g., hotel Wi-Fi login), it sends an IPC message to the terminal. The terminal intercepts the next network request and renders an MROP `[ 🌐 Open Login Page ]` button directly in the active Execution Block.
*   **Status Widgets:** Long-running scripts can query the network daemon to render a live MROP bandwidth usage widget next to the prompt.

---

## 🛠️ 4. Shared Protocols & APIs

To maintain strict memory safety and performance across the MITOS ecosystem, `mitos-terminal` relies on the following shared Rust primitives defined in `mitos-utils`:

### MROP (MITOS Rich Output Protocol)
A standardized JSON-over-OSC protocol for injecting GUI elements into the terminal stream.
```json
// Example Payload sent from mitos-shell to mitos-terminal
\x1b]MITOS_WIDGET;{
  "type": "button", 
  "label": "Deploy to Prod", 
  "cmd": "mitos-deploy --env=prod"
}\x07
