# Running MITOS Terminal on macOS

MITOS Terminal is developed for MITOS (Linux). This page covers running it as a **standalone**
terminal on macOS — for example on an Intel Mac with macOS 11 "Big Sur" — without the MITOS-only
libraries (`mitos-utils`, `mitos-pkg`).

> **Status:** the standalone configuration is compiled, linted and tested on Linux by CI (the
> `standalone` job), and the macOS binary is built by the `macOS build` workflow. It has not been
> run on real Mac hardware yet, so expect rough edges.

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

- Open Settings (the gear button, or `Ctrl+,`) → **Effects** → tick **Matrix code rain**.
- Run something colourful, e.g. `ls -G`, `neofetch` (`brew install neofetch`) or a colour test.
- `Cmd+Shift+4`, then `Space`, then click the window for a clean window shot (`Option`-click to
  leave out the shadow).
- Replace the placeholder `screenshot.png` in the repository (it is currently a text file) with
  the image and uncomment the image tag at the top of the README.
- Optional prompt marks and working-directory tracking in zsh: add this line to `~/.zshrc`
  ```sh
  [[ -n "$MITOS_SHELL_INTEGRATION_DIR" ]] && source "$MITOS_SHELL_INTEGRATION_DIR/mitos.zsh"
  ```
  (the terminal writes the scripts to a private folder and exports that variable by itself).

## What works and what does not

Works as on Linux: the terminal itself (VT/xterm emulation, 256/true colour, scrollback, search,
selection, tabs and split panes), the theme engine and effects, settings, `Cmd+C`/`Cmd+V` as well
as `Ctrl+Shift+C`/`Ctrl+Shift+V`, and configuration hot-reload.

Not available on macOS (these fail quietly rather than crash):

- the MITOS ecosystem: IPC with other MITOS components, the "command not found → install" button;
- finding a pane's working directory and running job through `/proc` — new tabs only inherit the
  directory if the shell integration above is loaded, and closing a busy pane does not warn first;
- desktop notifications, bell sound and speech output (they call Linux tools);
- the software-renderer fallback (`LIBGL_ALWAYS_SOFTWARE` is Mesa-only).

Configuration lives in `~/Library/Application Support/mitos/` (`terminal.toml`, `home.conf`).
The window uses OpenGL, which macOS still provides; a 2014 Mac's integrated graphics are enough.
