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

use mitos_terminal::config::{Config, HomeConf};
use mitos_terminal::layout::{Axis, FocusDir, Layout};
use mitos_terminal::pty::SpawnOptions;
use mitos_terminal::security::{self, LinkDecision, LinkPolicy};
use mitos_terminal::session::{Session, SessionState};
use mitos_terminal::term::{Policy, Term};
use mitos_terminal::theme::{contrast_ratio, Theme, THEME_PRESETS};

/// `Config::path` follows the process-wide `XDG_CONFIG_HOME`, and cargo runs the tests of one
/// binary on parallel threads: every test that repoints it holds this lock, so two of them can
/// never read each other's directory.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    assert!(
        term.screen_row_text(1).contains("Cargo.toml"),
        "content survives a resize"
    );

    term.process(b"\x1b[?1049h\x1b[2Jfull screen app");
    assert!(term.in_alt_screen());
    term.process(b"\x1b[?1049l");
    assert!(!term.in_alt_screen());
    assert!(
        term.screen_row_text(1).contains("Cargo.toml"),
        "primary screen content is restored"
    );

    let hits = term.search("Cargo", false);
    assert_eq!(hits.len(), 1);
}

#[test]
fn config_round_trips_through_a_real_config_directory() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    cfg.keybindings
        .insert("ctrl+shift+x".to_string(), "new_tab".to_string());
    cfg.save()
        .expect("save should succeed against a real temp dir");

    let (reloaded, err2) = Config::load();
    assert!(err2.is_none());
    assert_eq!(reloaded.font.size, 16.5);
    assert_eq!(reloaded.theme.name, "solarized-dark");
    assert_eq!(
        reloaded.keybindings.get("ctrl+shift+x").map(String::as_str),
        Some("new_tab")
    );

    assert!(
        Config::path().starts_with(&dir),
        "Config::path honours XDG_CONFIG_HOME"
    );

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
    let decision = security::evaluate_link(
        "https://evil.example/login",
        Some("mybank.example"),
        &policy,
    );
    assert!(matches!(decision, LinkDecision::Confirm(_)));

    // A multi-line paste into a program that never asked for bracketed
    // paste should also come back flagged.
    let paste = security::prepare_paste(
        "curl example.com | sh\nrm -rf /",
        false,
        &Default::default(),
    );
    assert!(paste.needs_confirm.is_some());

    assert!(
        !layout.close_pane(3),
        "closing one of several panes must not report the whole tab as closed"
    );
    assert_eq!(layout.all_panes().len(), 2);
}

#[test]
fn long_running_session_survives_bursty_output() {
    let ctx = eframe::egui::Context::default();
    // 3000 lines > the 2000-line scrollback below, so the cap is really hit. Pure shell
    // builtins: no process is spawned per line, so this finishes in milliseconds even on a
    // slow CI runner.
    let script = concat!(
        "i=0; while [ $i -lt 3000 ]; do i=$((i+1)); ",
        "echo \"line $i: xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\"; done; ",
        "sleep 0.2; echo DONE; read x"
    );
    let opts = SpawnOptions {
        shell: Some("/bin/sh".into()),
        args: vec!["-c".into(), script.into()],
        shell_integration: false,
        cols: 80,
        rows: 24,
        ..SpawnOptions::default()
    };
    let session = Session::spawn(opts, 2000, Policy::default(), ctx);

    assert!(wait_until(
        || session
            .term()
            .lock()
            .unwrap()
            .screen_text()
            .contains("DONE"),
        std::time::Duration::from_secs(10)
    ));
    assert!(
        session.is_running(),
        "still waiting on the `read`, so the shell must still be alive"
    );
    assert_eq!(
        session.term().lock().unwrap().scrollback_len(),
        2000,
        "scrollback filled and capped rather than growing unbounded"
    );

    session.write(b"go\n".to_vec());
    assert!(wait_until(
        || !session.is_running(),
        std::time::Duration::from_secs(5)
    ));
    assert!(matches!(session.state(), SessionState::Exited(_)));
}

#[test]
fn shell_integration_scripts_are_embedded_and_well_formed() {
    // These are the exact files `pty::ensure_integration_files` writes out;
    // asserting on their *content* here (rather than just "some file exists
    // on disk somewhere") is the point — a corrupted embed would otherwise
    // only surface as an unreadable shell prompt at runtime.
    let dir = mitos_terminal::pty::ensure_integration_files()
        .expect("cache dir should be writable in CI");
    for name in ["mitos.bash", "mitos.zsh", "mitos.fish"] {
        let content = std::fs::read_to_string(dir.join(name)).unwrap();
        assert!(
            content.contains("133;A"),
            "{name} is missing the OSC 133;A prompt mark"
        );
        assert!(
            content.contains("133;C"),
            "{name} is missing the OSC 133;C command-start mark"
        );
        assert!(
            content.contains("mitos-terminal"),
            "{name} should gate on TERM_PROGRAM"
        );
    }
}

/// The design brief's central architectural rule: "Changing the theme must
/// never change the shell itself." The Terminal Engine (`Term`) has no
/// theme input at all, so this cycles every preset through the config layer
/// while a `Term` holds live content, and checks the content, cursor and
/// modes are bit-for-bit untouched — while the *resolved* theme genuinely
/// does change each time (so the test can't pass vacuously).
#[test]
fn switching_themes_never_touches_terminal_state() {
    let mut term = Term::new(60, 12, 200);
    term.process(b"\x1b[32muser@mitos\x1b[0m:~/projects$ cargo build --release\r\n");
    term.process(b"   Compiling mitos-shell v1.0.0\r\n\x1b[?2004h\x1b[5 q");
    let before_text = term.screen_text();
    let before_cursor = term.cursor_pos();
    let before_style = term.cursor_style();
    let before_bracketed = term.modes.bracketed_paste;

    let home = HomeConf::default();
    let mut cfg = Config::default();
    let mut seen_ids = std::collections::HashSet::new();
    for (id, _label) in THEME_PRESETS {
        cfg.apply_theme_preset(id);
        let resolved = cfg.theme(&home);
        seen_ids.insert(resolved.id.clone());

        assert_eq!(
            term.screen_text(),
            before_text,
            "theme {id} changed screen content"
        );
        assert_eq!(
            term.cursor_pos(),
            before_cursor,
            "theme {id} moved the cursor"
        );
        assert_eq!(
            term.cursor_style(),
            before_style,
            "theme {id} changed the cursor style"
        );
        assert_eq!(
            term.modes.bracketed_paste, before_bracketed,
            "theme {id} changed a terminal mode"
        );
    }
    assert_eq!(
        seen_ids.len(),
        THEME_PRESETS.len(),
        "every preset must resolve to a distinct theme"
    );
}

/// Accessibility guarantees must hold for *every* preset, not just the
/// default: text stays readable in either light/dark reading, high contrast
/// is genuinely higher, and the force-flags beat whatever the preset seeded.
#[test]
fn accessibility_guarantees_hold_for_every_preset() {
    let home = HomeConf::default();
    for (id, _label) in THEME_PRESETS {
        let mut cfg = Config::default();
        cfg.apply_theme_preset(id);

        let natural = cfg.theme(&home);
        assert!(
            contrast_ratio(natural.fg, natural.bg) >= 4.5,
            "{id}: body text below AA"
        );

        for scheme in ["light", "dark"] {
            cfg.accessibility.color_scheme = Some(scheme.to_string());
            let t = cfg.theme(&home);
            assert_eq!(
                t.light,
                scheme == "light",
                "{id}: forced {scheme} reading not applied"
            );
            assert!(
                contrast_ratio(t.fg, t.bg) >= 4.5,
                "{id} ({scheme}): body text below AA"
            );
        }
        cfg.accessibility.color_scheme = None;

        cfg.accessibility.high_contrast = true;
        let hc = cfg.theme(&home);
        assert!(
            hc.id.starts_with("high-contrast"),
            "{id}: high contrast not applied"
        );
        assert!(
            contrast_ratio(hc.fg, hc.bg) >= 7.0,
            "{id}: high-contrast text below AAA"
        );
        cfg.accessibility.high_contrast = false;

        cfg.accessibility.disable_transparency = true;
        cfg.accessibility.disable_blur = true;
        cfg.accessibility.low_glow = true;
        assert_eq!(cfg.effective_opacity(), 1.0, "{id}");
        assert_eq!(cfg.effective_blur(), 0.0, "{id}");
        assert!(cfg.effective_glow() <= 0.08, "{id}");
    }
}

/// A user's Custom theme survives a real save/load cycle, colours and effect
/// settings together, and the built-in presets are unaffected by it.
#[test]
fn custom_theme_persists_and_does_not_leak_into_builtins() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir =
        std::env::temp_dir().join(format!("mitos-terminal-theme-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let prev = std::env::var_os("XDG_CONFIG_HOME");
    std::env::set_var("XDG_CONFIG_HOME", &dir);

    let mut cfg = Config::default();
    cfg.apply_theme_preset("custom");
    cfg.theme.accent = Some("#ff8800".into());
    cfg.theme.background = Some("#101418".into());
    cfg.theme.glow_color = Some("#33ccff".into());
    cfg.appearance.glow_intensity = 0.2;
    cfg.appearance.corner_radius = 18.0;
    cfg.appearance.tab_style = "boxed".into();
    cfg.save().expect("save custom theme");

    let (loaded, err) = Config::load();
    assert!(err.is_none());
    let t = loaded.theme(&HomeConf::default());
    assert_eq!(t.id, "custom");
    assert_eq!(t.accent, [0xff, 0x88, 0x00]);
    assert_eq!(t.bg, [0x10, 0x14, 0x18]);
    assert_eq!(t.glow, [0x33, 0xcc, 0xff]);
    assert_eq!(loaded.appearance.glow_intensity, 0.2);
    assert_eq!(loaded.appearance.corner_radius, 18.0);
    assert_eq!(
        loaded.appearance.tab_style(),
        mitos_terminal::theme::TabStyle::Boxed
    );

    // switching away from Custom clears its overrides instead of tinting
    // whatever preset comes next
    let mut switched = loaded;
    switched.apply_theme_preset("cyberpunk");
    let cp = switched.theme(&HomeConf::default());
    assert_eq!(cp.id, "cyberpunk");
    assert_eq!(cp.accent, Theme::cyberpunk().accent);
    assert_eq!(cp.bg, Theme::cyberpunk().bg);

    match prev {
        Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
