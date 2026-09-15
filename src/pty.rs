use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use anyhow::Result;

pub struct MitosPty {
    pub master: Box<dyn portable_pty::MasterPty + Send>,
}

impl MitosPty {
    pub fn new(cols: u16, rows: u16) -> Result<Self> {
        let pty_system = NativePtySystem::default();
        
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let cmd = shell_command();
        let _child = pair.slave.spawn_command(cmd)?;
        Ok(Self { master: pair.master })
    }
    
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(PtySize {
            rows, cols, pixel_width: 0, pixel_height: 0,
        })?;
        Ok(())
    }
}

/// Builds the command to launch inside the PTY: `mitos-shell` if it's
/// actually installed and on PATH, `sh` otherwise. Checked up front
/// (rather than trying `mitos-shell` and catching a spawn failure) so
/// there's only ever one `spawn_command` call — simpler than reasoning
/// about whether a `SlavePty` supports being spawned into twice.
fn shell_command() -> CommandBuilder {
    let mut cmd = if which_in_path("mitos-shell") {
        CommandBuilder::new("mitos-shell")
    } else {
        eprintln!("[mitos-terminal] mitos-shell not found in PATH — falling back to sh");
        CommandBuilder::new("sh")
    };

    cmd.env("TERM", "xterm-256color");
    // What INTEGRATION.md calls the shell-side half of the MROP
    // handshake: a real mitos-shell can check these to know it's
    // running inside a terminal that understands OSC_WIDGET /
    // OSC_NEW_BLOCK, rather than assuming or probing for it. Harmless
    // to set even when the fallback `sh` is what actually launches —
    // plain `sh` has no reason to read them, so they're just ignored.
    cmd.env("MITOS_TERMINAL_VERSION", env!("CARGO_PKG_VERSION"));
    cmd.env("MITOS_MROP_SUPPORTED", "1");

    cmd
}

/// Searches `$PATH` for an executable file named `binary` — the
/// standard, dependency-free way to answer "is this actually
/// installed" before trying to spawn it.
fn which_in_path(binary: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| is_executable(&dir.join(binary)))
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}
