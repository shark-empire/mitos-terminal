# Running MITOS Terminal on macOS

MITOS Terminal is developed for MITOS (Linux). This page covers running it as a **standalone**
terminal on macOS — for example on an Intel Mac with macOS 11 "Big Sur" — without the MITOS-only
libraries (`mitos-utils`, `mitos-pkg`).

> **Status:** the standalone configuration is compiled, linted and tested on Linux by CI (the
> `standalone` job), and the macOS binary is built by the `macOS build` workflow. It has had only
> light testing on real Mac hardware, so expect rough edges. The macOS-specific behaviour described
> on this page (config location, Command-key shortcuts, Option key, notifications, bell, speech,
> working directory and running-job detection, zsh integration) was written from a code review and
> still needs a real-Mac check; please report anything that does not behave as described.

## Option A — download a prebuilt binary (best for an old Mac)

1. On GitHub open **Actions → macOS build → Run workflow**. It takes roughly 10–15 minutes.
2. Open the finished run and download the artifact `mitos-terminal-macos-x86_64` (at the bottom of
   the page). Unzip it, then unpack `mitos-terminal-macos-x86_64.tar.gz` (double-click it, or
   `tar xzf mitos-terminal-macos-x86_64.tar.gz`).
3. Allow the unsigned binary and start it from Terminal:

   ```sh
   cd mitos-terminal
   xattr -dr com.apple.quarantine .
   ./mitos-terminal
   ```

   If macOS still refuses ("cannot be opened because the developer cannot be verified"), open
   **System Preferences → Security & Privacy → General** and choose **Open Anyway** / **Allow Anyway**.

## Option B — build it on the Mac

```sh
xcode-select --install        # Big Sur: if this fails, download "Command Line Tools for Xcode 13.x"
                              # from developer.apple.com/download/all (Big Sur's last supported)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
git clone <your repository URL> && cd mitos-terminal
cargo run --release --no-default-features
```

The first build compiles a few hundred crates and takes a long time on 2014 hardware.
`--no-default-features` is what leaves out the MITOS-only crates; without it Cargo also tries to
build `mitos-utils` and `mitos-pkg`, which are Linux userland libraries.

## Taking a screenshot

- Open Settings (the gear button, or `Cmd+,`) → **Effects** → tick **Matrix code rain**.
- Run something colourful, e.g. `ls -G`, `neofetch` (`brew install neofetch`) or a colour test.
- `Cmd+Shift+4`, then `Space`, then click the window for a clean window shot (`Option`-click to
  leave out the shadow).
- Replace the placeholder `screenshot.png` in the repository (it is currently a text file) with
  the image and uncomment the image tag at the top of the README.

## Shell integration (zsh)

zsh is the default shell on macOS, and the terminal now loads its shell integration into it
automatically (prompt marks, working-directory tracking, "command finished" notifications). It
starts zsh as a *login* shell, like Terminal.app does, so `~/.zprofile` (Homebrew's `PATH` setup)
is read even when the app is opened from the Dock. Your own `~/.zshenv`, `~/.zprofile`, `~/.zshrc`
and `~/.zlogin` run exactly as before, from wherever `ZDOTDIR` points. To switch this off set
`shell_integration = false` under `[general]` (or untick **Shell integration** in Settings); then
the old manual line still works in `~/.zshrc`:

```sh
[[ -n "$MITOS_SHELL_INTEGRATION_DIR" ]] && source "$MITOS_SHELL_INTEGRATION_DIR/mitos.zsh"
```

## What works and what does not

Works as on Linux: the terminal itself (VT/xterm emulation, 256/true colour, scrollback, search,
selection, tabs and split panes), the theme engine and effects, settings, `Cmd+C`/`Cmd+V` as well
as `Ctrl+Shift+C`/`Ctrl+Shift+V`, and configuration hot-reload.

Implemented for macOS with the tools the system ships (`/usr/bin/osascript`, `afplay`, `say`,
`defaults`, `/bin/ps`, `/usr/sbin/lsof`):

- desktop notifications (OSC 9/777 and "command finished"), the audible bell, bell → Dock-icon
  bounce, and speech output (notifications are posted through `osascript`, so macOS lists them
  under **Script Editor** in System Settings → Notifications; allow them there if nothing appears);
- finding a pane's working directory and running job, so new tabs and splits inherit the directory
  and closing a busy pane asks first;
- `[theme] follow_system` follows the macOS light/dark appearance.

Not available on macOS (these fail quietly rather than crash):

- the MITOS ecosystem: IPC with other MITOS components, the "command not found → install" button;
- real background blur: **Background blur** only tints the margin around the text, it does not blur
  what is behind the window (that would need macOS's vibrancy API);
- the software-renderer fallback (`LIBGL_ALWAYS_SOFTWARE` is Mesa-only).

## Configuration

The config directory is `~/.config/mitos/` (`terminal.toml`, `home.conf`) — the same as on Linux and
the path the README and the Settings window show. `$XDG_CONFIG_HOME/mitos` is used instead when that
variable is set. Earlier builds used `~/Library/Application Support/mitos/` on macOS; that folder is
still used when it holds a config file and `~/.config/mitos/` does not, so an existing setup keeps
working. The Settings window shows the exact file it saves to at the bottom. If `terminal.toml` has
a mistake the terminal falls back to the defaults and says so in a bar at the bottom of the window
and on stderr (run it from Terminal to see that), instead of silently ignoring every setting.

## macOS-specific behaviour

- **Command-key shortcuts** (iTerm2/Terminal.app habits): `Cmd+T` new tab, `Cmd+N` new window,
  `Cmd+W` close pane, `Cmd+D` / `Cmd+Shift+D` split, `Cmd+F` find, `Cmd+G` / `Cmd+Shift+G` next /
  previous match, `Cmd+=` / `Cmd+-` / `Cmd+0` zoom, `Cmd+K` clear scrollback, `Cmd+,` settings,
  `Cmd+Shift+P` command palette, `Cmd+1`–`9` go to tab, `Cmd+Up/Down` previous/next prompt,
  `Ctrl+Cmd+F` full screen (F11 is "Show Desktop" on a Mac). The Ctrl-based defaults still work,
  except `Alt+1`–`9` and the plain `Ctrl+F`, `Ctrl+G`, `Ctrl+=`, `Ctrl+-`, which are not bound on
  macOS (Option+digit types characters there, and those Ctrl keys are readline's forward-char /
  abort / undo). `cmd` is accepted in `[keybindings]`, e.g. `"cmd+shift+t" = "new_tab"`.
- **Option key:** by default Option types characters (`@`, `|`, `[`, `{`, `\`, accents on most
  non-US layouts). Set `option_as_meta = true` under `[general]` (or tick it in Settings) to make
  Option act as Meta instead (`Option+B` sends `ESC b`, so word movement and `Alt-` bindings work in
  zsh/bash).
- **Links:** `Cmd+click` opens a link (macOS turns `Ctrl+click` into a right-click). `[links] mode =
  "click"` makes a plain click open it.
- **Ctrl+C** always interrupts by default on macOS (`[selection] ctrl_c_copies` is off there); copy
  with `Cmd+C`.
- Bound shortcuts are no longer also sent to the program in the pane.

The window uses OpenGL, which macOS still provides; a 2014 Mac's integrated graphics are enough.
