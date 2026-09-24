//! Pseudo-terminal plumbing: spawn a shell, resize it, signal it, look at what
//! it is doing. No GUI and no terminal emulation in here.
//!
//! Lifecycle notes
//! * The slave end is dropped in the parent right after spawning, otherwise the
//!   master never sees EOF/EIO when the child exits.
//! * Closing the master delivers SIGHUP to the session's foreground process
//!   group (kernel tty semantics); `ChildKiller::kill` also sends SIGHUP on unix.
//! * Foreground-job tracking reads `/proc/<pid>/stat` (`tpgid` = foreground
//!   process group of the controlling terminal) — no extra dependencies.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShellKind {
    MitosShell,
    Bash,
    Zsh,
    Fish,
    Other,
}

impl ShellKind {
    pub fn of(path: &str) -> ShellKind {
        let base = Path::new(path).file_name().and_then(|s| s.to_str()).unwrap_or("").trim_start_matches('-');
        match base {
            "mitos-shell" => ShellKind::MitosShell,
            "bash" => ShellKind::Bash,
            "zsh" => ShellKind::Zsh,
            "fish" => ShellKind::Fish,
            _ => ShellKind::Other,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SpawnOptions {
    /// Explicit shell/program; otherwise mitos-shell → `$SHELL` → `/bin/sh`.
    pub shell: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub cols: u16,
    pub rows: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
    /// Inject OSC 133/7 integration into bash.
    pub shell_integration: bool,
}

impl Default for SpawnOptions {
    fn default() -> Self {
        SpawnOptions {
            shell: None,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            cols: 80,
            rows: 24,
            pixel_width: 0,
            pixel_height: 0,
            shell_integration: true,
        }
    }
}

pub struct SpawnedPty {
    pub master: Box<dyn MasterPty + Send>,
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn Child + Send + Sync>,
    pub killer: Box<dyn ChildKiller + Send + Sync>,
    pub pid: Option<u32>,
    pub shell: String,
    pub kind: ShellKind,
}

// ---------------------------------------------------------------------------
// Shell discovery
// ---------------------------------------------------------------------------

fn is_executable(p: &Path) -> bool {
    match std::fs::metadata(p) {
        Ok(m) => m.is_file() && m.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

/// Look `name` up on `$PATH` (plus the usual system dirs).
pub fn which(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return if is_executable(&p) { Some(p) } else { None };
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    for extra in ["/usr/local/bin", "/usr/bin", "/bin"] {
        dirs.push(PathBuf::from(extra));
    }
    for d in dirs {
        let cand = d.join(name);
        if is_executable(&cand) {
            return Some(cand);
        }
    }
    None
}

/// Shell precedence: explicit → `mitos-shell` → `$SHELL` → `/bin/sh`.
pub fn resolve_shell(explicit: Option<&str>) -> (String, ShellKind) {
    let pick = |p: PathBuf| -> (String, ShellKind) {
        let s = p.to_string_lossy().into_owned();
        let k = ShellKind::of(&s);
        (s, k)
    };
    if let Some(e) = explicit.filter(|e| !e.trim().is_empty()) {
        if let Some(p) = which(e.trim()) {
            return pick(p);
        }
    }
    if let Some(p) = which("mitos-shell") {
        return pick(p);
    }
    if let Ok(sh) = std::env::var("SHELL") {
        if let Some(p) = which(&sh) {
            return pick(p);
        }
    }
    ("/bin/sh".to_string(), ShellKind::Other)
}

fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// An existing directory, else `$HOME`.
pub fn usable_cwd(cwd: Option<&Path>) -> PathBuf {
    match cwd {
        Some(p) if p.is_dir() => p.to_path_buf(),
        _ => home_dir(),
    }
}

// ---------------------------------------------------------------------------
// Shell-integration files
// ---------------------------------------------------------------------------

const BASH_INTEGRATION: &str = include_str!("../shell-integration/mitos.bash");
const ZSH_INTEGRATION: &str = include_str!("../shell-integration/mitos.zsh");
const FISH_INTEGRATION: &str = include_str!("../shell-integration/mitos.fish");

fn write_if_changed(path: &Path, content: &str) {
    if std::fs::read_to_string(path).map(|c| c == content).unwrap_or(false) {
        return;
    }
    let _ = std::fs::write(path, content);
}

/// Write the embedded integration scripts to a private per-user directory and
/// return it. Sourcing `mitos.bash|zsh|fish` from there enables OSC 7 / OSC 133.
pub fn ensure_integration_files() -> Option<PathBuf> {
    let dir = dirs::cache_dir()?.join("mitos").join("terminal").join("shell-integration");
    std::fs::create_dir_all(&dir).ok()?;
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    write_if_changed(&dir.join("mitos.bash"), BASH_INTEGRATION);
    write_if_changed(&dir.join("mitos.zsh"), ZSH_INTEGRATION);
    write_if_changed(&dir.join("mitos.fish"), FISH_INTEGRATION);
    let rc = format!(
        "# Generated by mitos-terminal: your normal bashrc, then shell integration.\n\
         if [ -f /etc/bash.bashrc ]; then . /etc/bash.bashrc; fi\n\
         if [ -f \"$HOME/.bashrc\" ]; then . \"$HOME/.bashrc\"; fi\n\
         . \"{}/mitos.bash\"\n",
        dir.display()
    );
    write_if_changed(&dir.join("bashrc"), &rc);
    Some(dir)
}

/// Environment variables set by *other* terminals that must not leak into ours.
const FOREIGN_TERMINAL_VARS: &[&str] = &[
    "KITTY_WINDOW_ID",
    "KITTY_PID",
    "WEZTERM_PANE",
    "WEZTERM_EXECUTABLE",
    "ALACRITTY_LOG",
    "ALACRITTY_SOCKET",
    "ALACRITTY_WINDOW_ID",
    "VTE_VERSION",
    "GNOME_TERMINAL_SCREEN",
    "GNOME_TERMINAL_SERVICE",
    "KONSOLE_VERSION",
    "KONSOLE_DBUS_SERVICE",
    "ITERM_SESSION_ID",
    "TERM_SESSION_ID",
    "WT_SESSION",
    "TERMINATOR_UUID",
    "TILIX_ID",
];

// ---------------------------------------------------------------------------
// Spawn / resize
// ---------------------------------------------------------------------------

pub fn spawn(opts: &SpawnOptions) -> Result<SpawnedPty> {
    let (shell, kind) = resolve_shell(opts.shell.as_deref());
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: opts.rows.max(1),
            cols: opts.cols.max(1),
            pixel_width: opts.pixel_width,
            pixel_height: opts.pixel_height,
        })
        .map_err(|e| anyhow!("openpty failed: {e}"))?;

    let mut cmd = CommandBuilder::new(&shell);
    let integration = if opts.shell_integration { ensure_integration_files() } else { None };
    if opts.args.is_empty() {
        if let (ShellKind::Bash, Some(dir)) = (kind, integration.as_ref()) {
            cmd.arg("--rcfile");
            cmd.arg(dir.join("bashrc"));
        }
    } else {
        for a in &opts.args {
            cmd.arg(a);
        }
    }
    cmd.cwd(usable_cwd(opts.cwd.as_deref()));

    for v in FOREIGN_TERMINAL_VARS {
        cmd.env_remove(v);
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TERM_PROGRAM", "mitos-terminal");
    cmd.env("TERM_PROGRAM_VERSION", VERSION);
    cmd.env("MITOS_TERMINAL_VERSION", VERSION);
    cmd.env("MITOS_TERM_VERSION", VERSION);
    cmd.env("MITOS_TERMINAL_PID", std::process::id().to_string());
    if let Some(dir) = integration.as_ref() {
        cmd.env("MITOS_SHELL_INTEGRATION_DIR", dir);
    }
    for (k, v) in &opts.env {
        cmd.env(k, v);
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| anyhow!("failed to spawn `{shell}`: {e}"))?;
    // Critical: without this the master never reports EOF when the child exits.
    drop(pair.slave);

    let reader = pair.master.try_clone_reader().map_err(|e| anyhow!("cannot read from pty: {e}"))?;
    let writer = pair.master.take_writer().map_err(|e| anyhow!("cannot write to pty: {e}"))?;
    let killer = child.clone_killer();
    let pid = child.process_id();

    Ok(SpawnedPty { master: pair.master, reader, writer, child, killer, pid, shell, kind })
}

/// Tell the kernel (and, through SIGWINCH, the foreground program) the new size.
pub fn resize(master: &dyn MasterPty, cols: u16, rows: u16, pixel_width: u16, pixel_height: u16) -> Result<()> {
    master
        .resize(PtySize { rows: rows.max(1), cols: cols.max(1), pixel_width, pixel_height })
        .map_err(|e| anyhow!("resize failed: {e}"))
        .context("pty resize")
}

// ---------------------------------------------------------------------------
// /proc introspection (Linux)
// ---------------------------------------------------------------------------

/// `(pgrp, tpgid)` from the contents of `/proc/<pid>/stat`. The command name is
/// parenthesised and may itself contain spaces and parentheses, so parse from the *last* `)`.
pub fn parse_stat(stat: &str) -> Option<(i32, i32)> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    // f: state ppid pgrp session tty_nr tpgid …
    let pgrp = f.get(2)?.parse::<i32>().ok()?;
    let tpgid = f.get(5)?.parse::<i32>().ok()?;
    Some((pgrp, tpgid))
}

fn stat_of(pid: u32) -> Option<(i32, i32)> {
    parse_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// Foreground process group of the child's controlling terminal.
pub fn foreground_pgid(child_pid: u32) -> Option<i32> {
    let (_, tpgid) = stat_of(child_pid)?;
    if tpgid > 0 {
        Some(tpgid)
    } else {
        None
    }
}

/// `true` when something other than the shell itself owns the terminal
/// (a running command, an editor, …).
pub fn has_foreground_job(child_pid: u32) -> bool {
    match stat_of(child_pid) {
        Some((pgrp, tpgid)) => tpgid > 0 && tpgid != pgrp,
        None => false,
    }
}

/// Name of the foreground job (`vim`, `cargo`, …) if one is running.
pub fn foreground_name(child_pid: u32) -> Option<String> {
    let (pgrp, tpgid) = stat_of(child_pid)?;
    if tpgid <= 0 || tpgid == pgrp {
        return None;
    }
    let comm = std::fs::read_to_string(format!("/proc/{tpgid}/comm")).ok()?;
    let name = comm.trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Current working directory of a process.
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

// ---------------------------------------------------------------------------
// Signals
// ---------------------------------------------------------------------------

pub const SIGHUP: i32 = libc::SIGHUP;
pub const SIGINT: i32 = libc::SIGINT;
pub const SIGTERM: i32 = libc::SIGTERM;
pub const SIGKILL: i32 = libc::SIGKILL;

/// Signal a whole process group. Never signals pgid ≤ 1.
pub fn signal_group(pgid: i32, sig: i32) -> bool {
    if pgid <= 1 {
        return false;
    }
    // SAFETY: kill(2) has no memory-safety preconditions.
    unsafe { libc::kill(-pgid, sig) == 0 }
}

/// Signal a single process. Never signals pid ≤ 1.
pub fn signal_pid(pid: u32, sig: i32) -> bool {
    if pid <= 1 {
        return false;
    }
    // SAFETY: kill(2) has no memory-safety preconditions.
    unsafe { libc::kill(pid as libc::pid_t, sig) == 0 }
}

/// Signal the foreground job (or the shell itself when nothing else runs).
pub fn signal_foreground(child_pid: u32, sig: i32) -> bool {
    match foreground_pgid(child_pid) {
        Some(g) => signal_group(g, sig),
        None => signal_pid(child_pid, sig),
    }
}

/// How a child ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitInfo {
    pub code: i32,
}

impl ExitInfo {
    /// Human-readable, e.g. `exited with code 3` or `terminated by signal 9`.
    pub fn describe(&self) -> String {
        if self.code > 128 && self.code < 160 {
            format!("terminated by signal {} (exit code {})", self.code - 128, self.code)
        } else if self.code == 0 {
            "exited normally".to_string()
        } else {
            format!("exited with code {}", self.code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn opts(sh_c: &str) -> SpawnOptions {
        SpawnOptions {
            shell: Some("/bin/sh".into()),
            args: vec!["-c".into(), sh_c.into()],
            shell_integration: false,
            ..SpawnOptions::default()
        }
    }

    /// Read everything until EOF/EIO, then reap the child.
    fn run(sh_c: &str) -> (String, i32) {
        let mut p = spawn(&opts(sh_c)).expect("spawn");
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match p.reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        let status = p.child.wait().expect("wait");
        (String::from_utf8_lossy(&out).into_owned(), status.exit_code() as i32)
    }

    #[test]
    fn stat_parsing_survives_hostile_command_names() {
        let stat = "123 (a) b) c (d) S 1 123 123 34816 456 4194304 100 0 0 0";
        assert_eq!(parse_stat(stat), Some((123, 456)));
        assert_eq!(parse_stat("garbage"), None);
    }

    #[test]
    fn shell_kind_detection() {
        assert_eq!(ShellKind::of("/usr/bin/bash"), ShellKind::Bash);
        assert_eq!(ShellKind::of("-zsh"), ShellKind::Zsh);
        assert_eq!(ShellKind::of("/opt/mitos/bin/mitos-shell"), ShellKind::MitosShell);
        assert_eq!(ShellKind::of("/bin/dash"), ShellKind::Other);
    }

    #[test]
    fn explicit_bogus_shell_falls_back() {
        let (s, _) = resolve_shell(Some("/definitely/not/a/shell"));
        assert!(std::path::Path::new(&s).exists());
    }

    #[test]
    fn output_and_exit_status_are_captured() {
        let (out, code) = run("printf hello; exit 3");
        assert!(out.contains("hello"), "{out:?}");
        assert_eq!(code, 3);
    }

    #[test]
    fn environment_advertises_the_terminal() {
        let (out, _) = run("echo $TERM $COLORTERM $TERM_PROGRAM");
        assert!(out.contains("xterm-256color truecolor mitos-terminal"), "{out:?}");
    }

    #[test]
    fn utf8_passes_through_the_pty() {
        let (out, _) = run("printf '世界 é'");
        assert!(out.contains("世界 é"), "{out:?}");
    }

    #[test]
    fn resize_reaches_the_child() {
        let mut o = opts("sleep 0.4; stty size");
        o.cols = 80;
        o.rows = 24;
        let mut p = spawn(&o).expect("spawn");
        resize(p.master.as_ref(), 100, 30, 0, 0).expect("resize");
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = p.reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        let s = String::from_utf8_lossy(&out).into_owned();
        assert!(s.contains("30 100"), "{s:?}");
    }

    #[test]
    fn ctrl_c_interrupts_the_foreground_process() {
        let mut p = spawn(&opts("sleep 30")).expect("spawn");
        std::thread::sleep(Duration::from_millis(300));
        p.writer.write_all(&[0x03]).expect("write ^C");
        p.writer.flush().ok();
        let start = Instant::now();
        let status = p.child.wait().expect("wait");
        assert!(start.elapsed() < Duration::from_secs(10), "child did not die on ^C");
        assert_ne!(status.exit_code(), 0);
    }

    #[test]
    fn closing_the_master_hangs_up_the_child() {
        let p = spawn(&opts("sleep 30")).expect("spawn");
        let SpawnedPty { master, reader, writer, mut child, .. } = p;
        drop(writer);
        drop(reader);
        drop(master);
        let start = Instant::now();
        let _ = child.wait();
        assert!(start.elapsed() < Duration::from_secs(10), "SIGHUP did not reach the child");
    }

    #[test]
    fn signal_helpers_refuse_dangerous_targets() {
        assert!(!signal_group(0, SIGTERM));
        assert!(!signal_group(1, SIGTERM));
        assert!(!signal_pid(0, SIGTERM));
        assert!(!signal_pid(1, SIGTERM));
    }

    #[test]
    fn foreground_introspection_of_a_live_child() {
        let p = spawn(&opts("sleep 1")).expect("spawn");
        let pid = p.pid.expect("pid");
        assert!(foreground_pgid(pid).is_some());
        assert!(process_cwd(pid).is_some());
        let mut killer = p.killer;
        let _ = killer.kill();
    }

    #[test]
    fn exit_descriptions() {
        assert_eq!(ExitInfo { code: 0 }.describe(), "exited normally");
        assert_eq!(ExitInfo { code: 3 }.describe(), "exited with code 3");
        assert!(ExitInfo { code: 137 }.describe().contains("signal 9"));
    }
}
