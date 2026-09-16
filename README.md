# mitos-terminal

**MITOS's native terminal emulator** — a cinematic, system-aware `egui`/`eframe` application that owns a PTY, parses ANSI/VT100 streams, and deeply integrates with the rest of the MITOS ecosystem. 

It is not just a shell wrapper; it is a rich GUI that hosts your shell (like Alacritty or Windows Terminal), but with built-in OS-level awareness, visual effects, and package management integration.

<!-- Tip: Add a screenshot or GIF of the terminal here! -->
<!-- ![MITOS Terminal](docs/screenshot.png) -->

## ✨ Key Features

### 🎬 Cinematic Visuals & Phosphor Decay
- **Matrix Code Rain**: A configurable, performant background effect with drifting holographic grids and radar sweeps.
- **Phosphor Glow**: Newly printed characters ignite with a "white-hot" core and phosphor halo that smoothly cools down over time.
- **Glitch Jitter**: Shell errors (like "command not found") trigger a cinematic red glitch effect strictly on the affected text.
- **CRT Scanlines**: Subtle overlay for that authentic retro-futuristic feel.

### 🧠 System-Aware Intelligence
- **Package Bridge (`mitos-pkg`)**: Intercepts shell "command not found" errors and dynamically injects a GUI `📦 Install` button right into the terminal stream.
- **Semantic Clipboard**: Press `Ctrl+Shift+C` on a word. If it's a valid file path, the terminal copies it to your clipboard as a native `file://` URI for the file manager.
- **Captive Portal Detection**: Polls `mitos-network` via IPC and injects a "🌐 Open Login Page" button if a captive portal is detected on the network.

### 🛠️ Modern Terminal Essentials
- **OSC 8 Hyperlinks**: Full support for clickable URLs rendered by CLI tools (opens in default browser).
- **Standard Selection**: Click-and-drag text selection with standard `Ctrl+C` copying.
- **Search (`Ctrl+F`)**: Instantly search and highlight matches across your entire scrollback history.
- **Smart Reflow**: Resizing the window dynamically reflows text without breaking lines or leaving ghost characters.
- **Memory Safe Scrollback**: Uses a capped `VecDeque` ring buffer to store history without consuming all your RAM during long sessions.

## 🏗️ Ecosystem Integration

`mitos-terminal` is designed to be the visual anchor of the MITOS operating system:

- **MROP (MITOS Rich Object Protocol)**: A custom OSC sequence protocol allowing CLI tools to inject rich `egui` widgets (buttons, progress bars, sparklines) directly into the terminal output.
- **Unix-Socket IPC**: Exposes an API so other MITOS daemons (like `mitos-system-monitor` or `mitos-settings`) can read the terminal buffer, inject widgets, or push theme updates in real-time.
- **Theme Syncing**: Watches `~/.config/mitos/home.conf` and retroactively repaints the entire terminal history when the system theme (Light/Dark) or accent color changes.
- **Shell Agnostic**: Automatically launches `mitos-shell` if installed, gracefully falling back to `sh` or `bash` during early OS bootstrapping.

## ⌨️ Keyboard Shortcuts

| Shortcut | Action |
| :--- | :--- |
| `Ctrl` + `C` | Copy selected text (Standard) |
| `Ctrl` + `Shift` + `C` | **Semantic Copy** (Copies valid file paths as `file://` URIs) |
| `Ctrl` + `F` | Toggle Search Bar (Highlights matches in history) |
| `Ctrl` + `,` | Toggle Settings Menu (Toggle Matrix Rain, etc.) |
| `Ctrl` + `Shift` + `V` | Paste |

## ⚙️ Configuration

Settings are automatically saved to and hot-reloaded from `~/.config/mitos/home.conf`.

```ini
theme_mode=dark
accent_color=#55FF55
matrix_rain=true
