//! Bridges "command not found" moments to `mitos-pkgd`, so the terminal
//! can offer an inline install button instead of just leaving the raw
//! shell error on screen — the mitos-pkg integration point
//! `INTEGRATION.md` describes, built directly against mitos-pkg's own
//! `DaemonClient` rather than a guessed protocol.
//!
//! `suggest_install` is deliberately synchronous — `DaemonClient` is
//! blocking I/O, matching mitos-pkg's own no-async design (see that
//! project's `Cargo.toml` for why). Call it via
//! `tokio::task::spawn_blocking` from async code, never `.await` a
//! wrapper around it — see `main.rs`'s grid-update loop for how it's
//! actually used.

use mitos_pkg::config::Config;
use mitos_pkg::daemon::client::DaemonClient;
use mitos_utils::ipc::RichWidget;
use std::path::PathBuf;

/// Best-effort extraction of an attempted command name from a shell's
/// "not found" error line. Shells word this differently — bash says
/// "bash: foo: command not found", dash/sh says "sh: 1: foo: not
/// found", zsh says "zsh: command not found: foo" — so this tries a
/// few common shapes rather than committing to exactly one shell's
/// wording. Pure string matching, no I/O — see `suggest_install` for
/// the part that actually talks to anything.
///
/// Both ways this can be wrong are low-stakes: a miss just means no
/// install suggestion appears (the shell's own error text is still
/// right there), and a spurious match — some unrelated line that
/// happens to contain one of these phrases, e.g. `grep`ping a log file
/// with "not found" in it — just means an unhelpful button shows up.
/// Nothing here ever executes automatically, so a wrong guess costs a
/// glance, never a mistake.
pub fn detect_missing_command(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let extract_before = |text: &str| -> Option<String> {
        let cmd = text.rsplit(": ").next()?.trim();
        (!cmd.is_empty()).then(|| cmd.to_string())
    };

    if let Some(rest) = line.strip_suffix(": command not found") {
        return extract_before(rest);
    }
    if let Some(idx) = line.find("command not found: ") {
        let cmd = line[idx + "command not found: ".len()..].trim();
        if !cmd.is_empty() {
            return Some(cmd.to_string());
        }
    }
    if let Some(rest) = line.strip_suffix(": not found") {
        return extract_before(rest);
    }

    None
}

/// Looks up `missing_command` against the running `mitos-pkgd` and, if
/// there's an exact package-name or `provides` match, returns a button
/// widget offering to install it.
///
/// Returns `None` for anything short of an exact match — including
/// "the daemon isn't reachable" and "nothing exact matched." Both
/// collapse to the same `None` on purpose: neither is worth surfacing
/// as an error for what's ultimately a nice-to-have suggestion on top
/// of a mistyped command, not a diagnostic tool. A caller that wants to
/// know *why* nothing came back should use `DaemonClient` directly.
pub fn suggest_install(missing_command: &str) -> Option<RichWidget> {
    let socket_path = PathBuf::from(Config::DEFAULT_SOCKET);
    let mut client = DaemonClient::connect(&socket_path).ok()?;
    let matches = client.search(missing_command).ok()?;

    let hit = matches.into_iter().find(|meta| {
        meta.name == missing_command || meta.provides.iter().any(|p| p == missing_command)
    })?;

    Some(RichWidget::Button {
        label: format!("📦 Install {}", hit.name),
        cmd: format!("mitos-pkg install {}", hit.name),
    })
}
