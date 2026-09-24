//! Integration tests: exercise the crate the way an external caller would,
//! through `mitos_terminal`'s public API only (no `pub(crate)` access), each
//! covering one spec section end-to-end rather than one function in
//! isolation:
//! * [`vt_session_end_to_end`] — Core (PTY-shaped byte stream in, correct
//!   screen state out) using every layer from `vte` parsing through resize.
//! * [`config_round_trips_through_a_real_config_directory`] — persistence
//!   against an actual (temp) `$XDG_CONFIG_HOME`, not just in-memory parsing.
//! * [`layout_and_security_multi_pane_workflow`] — UX (tabs/splits) and
//!   Security (link/paste policy) composed together the way `app.rs` uses them.
//! * [`long_running_session_survives_bursty_output`] — Testing's
//!   "long-running session" and "stress" bullets against a real child process.

use mitos_terminal::config::Config;
use mitos_terminal::layout::{Axis, FocusDir, Layout};
use mitos_terminal::pty::SpawnOptions;
use mitos_terminal::security::{self, LinkDecision, LinkPolicy};
use mitos_terminal::session::{Session, SessionState};
use mitos_terminal::term::{Policy, Term};

fn wait_until(mut pred: impl FnMut() -> bool, timeout: std::time::Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if pred() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    pred()
}

#[test]
fn vt_session_end_to_end() {
    let mut term = Term::new(40, 10, 500);

    // A shell prompt, coloured `ls` output, a resize mid-stream, and a trip
    // through the alternate screen (as `vim`/`less` would use) — all through
    // the one public entry point, `process`.
    term.process(b"\x1b]0;my-shell\x07");
    assert_eq!(term.title(), "my-shell");

    term.process(b"\x1b[32muser@host\x1b[0m:\x1b[34m~/proj\x1b[0m$ ls\r\n");
    term.process(b"\x1b[1mCargo.toml\x1b[0m  src\r\n");
    assert!(term.screen_row_text(1).contains("Cargo.toml"));

    term.resize(80, 24);
    assert_eq!((term.cols(), term.rows()), (80, 24));
    assert!(term.screen_row_text(1).contains("Cargo.toml"), "content survives a resize");

    term.process(b"\x1b[?1049h\x1b[2Jfull screen app");
    assert!(term.in_alt_screen());
    term.process(b"\x1b[?1049l");
    assert!(!term.in_alt_screen());
    assert!(term.screen_row_text(1).contains("Cargo.toml"), "primary screen content is restored");

    let hits = term.search("Cargo", false);
    assert_eq!(hits.len(), 1);
}

#[test]
fn config_round_trips_through_a_real_config_directory() {
    let dir = std::env::temp_dir().join(format!("mitos-terminal-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: integration test binaries run single-threaded per test file by
    // default for tests that touch process-global env state like this one
    // would need `--test-threads=1`; scoping the var to this process and
    // restoring it after is the best we can do without a crate dependency
    // just for env-var mocking.
    let prev = std::env::var_os("XDG_CONFIG_HOME");
    std::env::set_var("XDG_CONFIG_HOME", &dir);

    let (loaded, err) = Config::load();
    assert!(err.is_none());
    assert_eq!(loaded, Config::default(), "no file yet: defaults");

    let mut cfg = Config::default();
    cfg.font.size = 16.5;
    cfg.theme.name = "solarized-dark".to_string();
    cfg.keybindings.insert("ctrl+shift+x".to_string(), "new_tab".to_string());
    cfg.save().expect("save should succeed against a real temp dir");

    let (reloaded, err2) = Config::load();
    assert!(err2.is_none());
    assert_eq!(reloaded.font.size, 16.5);
    assert_eq!(reloaded.theme.name, "solarized-dark");
    assert_eq!(reloaded.keybindings.get("ctrl+shift+x").map(String::as_str), Some("new_tab"));

    assert!(Config::path().starts_with(&dir), "Config::path honours XDG_CONFIG_HOME");

    match prev {
        Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn layout_and_security_multi_pane_workflow() {
    let mut layout = Layout::new(1);
    layout.split(Axis::X, 2);
    layout.split(Axis::Y, 3);
    assert_eq!(layout.all_panes().len(), 3);

    layout.focus(1);
    layout.focus_direction(FocusDir::Right);
    assert_ne!(layout.focused_pane(), 1);

    // A pane's terminal emits an OSC 8 link with mismatched visible text —
    // security policy should ask for confirmation, not open it outright.
    let policy = LinkPolicy::default();
    let decision = security::evaluate_link("https://evil.example/login", Some("mybank.example"), &policy);
    assert!(matches!(decision, LinkDecision::Confirm(_)));

    // A multi-line paste into a program that never asked for bracketed
    // paste should also come back flagged.
    let paste = security::prepare_paste("curl example.com | sh\nrm -rf /", false, &Default::default());
    assert!(paste.needs_confirm.is_some());

    assert!(layout.close_pane(3));
    assert_eq!(layout.all_panes().len(), 2);
}

#[test]
fn long_running_session_survives_bursty_output() {
    let ctx = eframe::egui::Context::default();
    let opts = SpawnOptions {
        shell: Some("/bin/sh".into()),
        args: vec!["-c".into(), "for i in $(seq 1 500); do echo \"line $i: $(head -c 40 /dev/zero | tr '\\0' 'x')\"; done; sleep 0.2; echo DONE; read x".into()],
        shell_integration: false,
        cols: 80,
        rows: 24,
        ..SpawnOptions::default()
    };
    let session = Session::spawn(opts, 2000, Policy::default(), ctx);

    assert!(wait_until(|| session.term().lock().unwrap().screen_text().contains("DONE"), std::time::Duration::from_secs(10)));
    assert!(session.is_running(), "still waiting on the `read`, so the shell must still be alive");
    assert_eq!(session.term().lock().unwrap().scrollback_len(), 2000, "scrollback filled and capped rather than growing unbounded");

    session.write(b"go\n".to_vec());
    assert!(wait_until(|| !session.is_running(), std::time::Duration::from_secs(5)));
    assert!(matches!(session.state(), SessionState::Exited(_)));
}

#[test]
fn shell_integration_scripts_are_embedded_and_well_formed() {
    // These are the exact files `pty::ensure_integration_files` writes out;
    // asserting on their *content* here (rather than just "some file exists
    // on disk somewhere") is the point — a corrupted embed would otherwise
    // only surface as an unreadable shell prompt at runtime.
    let dir = mitos_terminal::pty::ensure_integration_files().expect("cache dir should be writable in CI");
    for name in ["mitos.bash", "mitos.zsh", "mitos.fish"] {
        let content = std::fs::read_to_string(dir.join(name)).unwrap();
        assert!(content.contains("133;A"), "{name} is missing the OSC 133;A prompt mark");
        assert!(content.contains("133;C"), "{name} is missing the OSC 133;C command-start mark");
        assert!(content.contains("mitos-terminal"), "{name} should gate on TERM_PROGRAM");
    }
}
