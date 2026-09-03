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
