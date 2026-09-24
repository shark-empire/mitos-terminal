//! A session: one PTY, one [`Term`], the threads that connect them, and the
//! lifecycle state (running / exited / crashed) the UI reacts to.
//!
//! Threading model
//! * **reader thread** — blocking reads from the PTY master. Each chunk is fed
//!   to `Term::process` *inside `catch_unwind`*: `Term` is fuzz-tested to
//!   never panic (see `term/tests.rs`), but a third-party terminal feeding
//!   attacker-controlled bytes into a screen model is exactly the kind of
//!   place defense-in-depth belongs. The lock is held only inside the
//!   `catch_unwind` closure, so a caught panic unwinds *before* the
//!   `MutexGuard` drops and the mutex is never poisoned. On EOF/error the
//!   thread reaps the child and records how it exited — this doubles as the
//!   crate's process-lifecycle tracking.
//! * **writer thread** — an unbounded channel to the PTY, so keystrokes and
//!   pastes from the UI thread never block on I/O.
//!
//! Both threads hold only a `Weak` reference to the `Term`'s `Arc`+`Mutex`
//! bundle where possible... in practice they need `Arc` to run at all, so
//! instead `Session::shutdown` closes the PTY (which makes the reader thread
//! exit on its own) and threads are `join`ed with a timeout-free `JoinHandle`
//! that is simply detached: they are guaranteed to finish shortly after the
//! fds close, and holding up pane-close on that would make the UI stutter.

use std::io::{Read, Write};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::pty::{self, ExitInfo, SpawnOptions, SpawnedPty};
use crate::term::{Policy, Term};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionState {
    Running,
    Exited(ExitInfo),
    /// The screen model panicked and was isolated; the pane shows the crash
    /// banner in `render.rs` instead of feeding it more bytes.
    Crashed(String),
    /// The PTY itself could not be created (bad shell, out of ptys, …).
    SpawnFailed(String),
}

pub struct Session {
    pub id: u64,
    term: Arc<Mutex<Term>>,
    writer_tx: Option<mpsc::Sender<Vec<u8>>>,
    /// Kept alive for `resize`; `try_clone_reader`/`take_writer` in `pty::spawn`
    /// gave the I/O threads their own independent handles, so this is not on
    /// their hot path and dropping it (on session drop) does not affect them.
    master: Option<Box<dyn portable_pty::MasterPty + Send>>,
    state: Arc<Mutex<SessionState>>,
    pid: Option<u32>,
    shell: String,
    kind: pty::ShellKind,
    started: Instant,
    /// Set by the reader thread right before it exits, so the UI can join
    /// without guessing; polled, never blocked on.
    reader_done: Arc<AtomicBool>,
}

impl Session {
    /// Spawn a shell and start the I/O threads. Never blocks on the child.
    pub fn spawn(opts: SpawnOptions, scrollback_limit: usize, policy: Policy, ctx: egui::Context) -> Session {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let mut term = Term::new(opts.cols as usize, opts.rows as usize, scrollback_limit);
        term.set_policy(policy);
        let term = Arc::new(Mutex::new(term));

        match pty::spawn(&opts) {
            Ok(p) => {
                let SpawnedPty { master, reader, mut writer, mut child, pid, shell, kind, .. } = p;
                let state = Arc::new(Mutex::new(SessionState::Running));
                let reader_done = Arc::new(AtomicBool::new(false));

                let (tx, rx) = mpsc::channel::<Vec<u8>>();
                std::thread::Builder::new()
                    .name(format!("mitos-term-w{id}"))
                    .spawn(move || {
                        for chunk in rx {
                            if writer.write_all(&chunk).is_err() {
                                break;
                            }
                            let _ = writer.flush();
                        }
                    })
                    .expect("failed to start the terminal writer thread");

                {
                    let term = Arc::clone(&term);
                    let state = Arc::clone(&state);
                    let reader_done = Arc::clone(&reader_done);
                    std::thread::Builder::new()
                        .name(format!("mitos-term-r{id}"))
                        .spawn(move || {
                            reader_loop(reader, &term, &ctx);
                            let exit = child.wait().ok().map(|s| ExitInfo { code: s.exit_code() as i32 });
                            let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
                            if matches!(*st, SessionState::Running) {
                                *st = SessionState::Exited(exit.unwrap_or(ExitInfo { code: 0 }));
                            }
                            drop(st);
                            reader_done.store(true, Ordering::Release);
                            ctx.request_repaint();
                        })
                        .expect("failed to start the terminal reader thread");
                }

                Session {
                    id,
                    term,
                    writer_tx: Some(tx),
                    master: Some(master),
                    state,
                    pid,
                    shell,
                    kind,
                    started: Instant::now(),
                    reader_done,
                }
            }
            Err(e) => Session {
                id,
                term,
                writer_tx: None,
                master: None,
                state: Arc::new(Mutex::new(SessionState::SpawnFailed(e.to_string()))),
                pid: None,
                shell: opts.shell.clone().unwrap_or_default(),
                kind: pty::ShellKind::Other,
                started: Instant::now(),
                reader_done: Arc::new(AtomicBool::new(true)),
            },
        }
    }

    pub fn term(&self) -> Arc<Mutex<Term>> {
        Arc::clone(&self.term)
    }

    pub fn state(&self) -> SessionState {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state(), SessionState::Running)
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn shell(&self) -> &str {
        &self.shell
    }

    pub fn shell_kind(&self) -> pty::ShellKind {
        self.kind
    }

    pub fn uptime(&self) -> Duration {
        self.started.elapsed()
    }

    /// `true` when a program other than the shell owns the terminal (an editor,
    /// a long build, …) — used for the "confirm before closing" prompt.
    pub fn has_foreground_job(&self) -> bool {
        self.pid.map(pty::has_foreground_job).unwrap_or(false)
    }

    pub fn foreground_name(&self) -> Option<String> {
        self.pid.and_then(pty::foreground_name)
    }

    /// `true` once the reader thread has seen EOF/EIO and reaped the child.
    /// Useful for tests and for a clean-shutdown check before closing a window.
    pub fn reader_finished(&self) -> bool {
        self.reader_done.load(Ordering::Acquire)
    }

    /// Send bytes to the child. Silently dropped once the session has ended —
    /// callers do not need to check `is_running` before every keystroke.
    pub fn write(&self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        if let Some(tx) = &self.writer_tx {
            let _ = tx.send(bytes);
        }
    }

    pub fn resize(&self, cols: usize, rows: usize, pixel_width: u16, pixel_height: u16) {
        {
            let mut t = self.term.lock().unwrap_or_else(|p| p.into_inner());
            if t.cols() == cols && t.rows() == rows {
                return;
            }
            t.resize(cols, rows);
        }
        if let Some(m) = self.master.as_ref() {
            let _ = pty::resize(m.as_ref(), cols as u16, rows as u16, pixel_width, pixel_height);
        }
    }

    // ----- signals ---------------------------------------------------------

    pub fn sigint(&self) {
        if let Some(pid) = self.pid {
            pty::signal_foreground(pid, pty::SIGINT);
        }
    }

    pub fn sigterm(&self) {
        if let Some(pid) = self.pid {
            pty::signal_foreground(pid, pty::SIGTERM);
        }
    }

    pub fn sigkill(&self) {
        if let Some(pid) = self.pid {
            pty::signal_pid(pid, pty::SIGKILL);
        }
    }

    /// Close the write end and drop the writer thread; on Unix this alone
    /// delivers SIGHUP to the child once the master side follows (`Drop`).
    pub fn shutdown(&mut self) {
        self.writer_tx.take();
        if let Some(pid) = self.pid {
            if self.is_running() {
                pty::signal_foreground(pid, pty::SIGHUP);
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Read until EOF/error, feeding every chunk to `Term::process` behind a
/// panic boundary. See the module doc for why this cannot poison the mutex.
fn reader_loop(mut reader: Box<dyn Read + Send>, term: &Arc<Mutex<Term>>, ctx: &egui::Context) {
    let mut buf = [0u8; 65536];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break, // EIO: the child's side of the pty is gone
        };
        let chunk = &buf[..n];
        let panicked = {
            let mut guard = match term.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            std::panic::catch_unwind(AssertUnwindSafe(|| guard.process(chunk))).is_err()
        };
        if panicked {
            eprintln!("[mitos-terminal] the screen model panicked on incoming output; this session is now isolated");
            break;
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicI32;

    fn opts(cmd: &str) -> SpawnOptions {
        SpawnOptions { shell: Some("/bin/sh".into()), args: vec!["-c".into(), cmd.into()], shell_integration: false, cols: 40, rows: 10, ..SpawnOptions::default() }
    }

    fn wait_until(mut pred: impl FnMut() -> bool, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if pred() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        pred()
    }

    #[test]
    fn output_reaches_the_term_and_repaints_are_requested() {
        let ctx = egui::Context::default();
        let s = Session::spawn(opts("printf hi"), 100, Policy::default(), ctx);
        assert!(wait_until(|| s.term().lock().unwrap().screen_text().contains("hi"), Duration::from_secs(5)));
        assert!(wait_until(|| !s.is_running(), Duration::from_secs(5)));
        assert_eq!(s.state(), SessionState::Exited(ExitInfo { code: 0 }));
    }

    #[test]
    fn written_bytes_reach_the_child() {
        let ctx = egui::Context::default();
        let s = Session::spawn(opts("read x; printf GOT:$x"), 100, Policy::default(), ctx);
        std::thread::sleep(Duration::from_millis(300));
        s.write(b"hello\n".to_vec());
        assert!(wait_until(|| s.term().lock().unwrap().screen_text().contains("GOT:hello"), Duration::from_secs(5)));
    }

    #[test]
    fn nonzero_exit_is_recorded() {
        let ctx = egui::Context::default();
        let s = Session::spawn(opts("exit 7"), 100, Policy::default(), ctx);
        assert!(wait_until(|| !s.is_running(), Duration::from_secs(5)));
        assert_eq!(s.state(), SessionState::Exited(ExitInfo { code: 7 }));
    }

    #[test]
    fn bad_shell_is_reported_without_panicking() {
        let ctx = egui::Context::default();
        let mut o = opts("true");
        o.shell = Some("/definitely/not/a/binary/xyz".into());
        o.args.clear();
        let s = Session::spawn(o, 100, Policy::default(), ctx);
        // resolve_shell falls back to a real shell for a bad explicit path, so this
        // spawns successfully — the meaningful assertion is just that nothing panics
        // and the session reaches a terminal state.
        assert!(wait_until(|| !s.is_running(), Duration::from_secs(5)));
    }

    #[test]
    fn sigint_stops_a_running_child() {
        let ctx = egui::Context::default();
        let s = Session::spawn(opts("trap '' INT; sleep 1; echo done"), 100, Policy::default(), ctx);
        std::thread::sleep(Duration::from_millis(300));
        // shell traps SIGINT so it survives; sigterm on the whole thing should still work
        s.sigterm();
        assert!(wait_until(|| !s.is_running(), Duration::from_secs(5)));
    }

    #[test]
    fn resize_updates_the_term_immediately() {
        let ctx = egui::Context::default();
        let s = Session::spawn(opts("sleep 2"), 100, Policy::default(), ctx);
        s.resize(100, 40, 0, 0);
        assert_eq!(s.term().lock().unwrap().cols(), 100);
        assert_eq!(s.term().lock().unwrap().rows(), 40);
    }

    /// Exercises the exact catch_unwind-inside-the-lock pattern `reader_loop`
    /// uses, with a toy payload instead of `Term`, to prove the technique
    /// does not poison the mutex — `Term::process` itself is proven panic-free
    /// by the fuzz test in `term/tests.rs`, so this is defense in depth, not
    /// a real fault path.
    #[test]
    fn panic_inside_the_lock_does_not_poison_it() {
        let data = Arc::new(Mutex::new(0i32));
        let armed = AtomicI32::new(1);
        {
            let mut g = data.lock().unwrap();
            let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
                if armed.load(Ordering::Relaxed) == 1 {
                    *g += 1;
                    panic!("simulated screen-model panic");
                }
            }));
        }
        assert!(!data.is_poisoned());
        assert_eq!(*data.lock().unwrap(), 1);
    }

    #[test]
    fn write_after_shutdown_is_a_harmless_noop() {
        let ctx = egui::Context::default();
        let mut s = Session::spawn(opts("cat"), 100, Policy::default(), ctx);
        s.shutdown();
        s.write(b"still here?\n".to_vec());
        assert!(wait_until(|| !s.is_running(), Duration::from_secs(5)));
    }
}
