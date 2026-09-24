//! Accessibility support that isn't just "use a real egui widget so AccessKit
//! sees it for free" (that part lives in `render.rs`, which places a standard
//! `egui::Label` holding the visible screen text over the custom-painted grid,
//! so a screen reader gets normal AT-SPI/UIA nodes with no bespoke tree-building
//! here).
//!
//! This module covers what still needs code:
//! * an announcement queue (bell, notifications, command-finished) — rate
//!   limited so a busy build log cannot turn into a wall of chatter;
//! * optional speech via `spd-say` (speech-dispatcher) for output/announcements;
//! * a best-effort audible bell (the *sound*, not the escape sequence — that
//!   part is `TermEvent::Bell`, handled by the caller);
//! * contrast checking, delegating the actual math to `theme::ensure_contrast`.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::theme::{contrast_ratio, ensure_contrast, Rgb, Theme};

// ---------------------------------------------------------------------------
// Announcements
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announcement {
    pub text: String,
    /// Interrupts anything currently being spoken (bell, errors) vs. queues behind it.
    pub urgent: bool,
}

/// Batches announcements for the screen reader / speech backend, dropping
/// duplicates that arrive faster than a person could take them in.
pub struct AnnounceQueue {
    pending: Vec<Announcement>,
    last_text: Option<String>,
    last_at: Option<Instant>,
    min_gap: Duration,
}

impl AnnounceQueue {
    pub fn new() -> AnnounceQueue {
        AnnounceQueue { pending: Vec::new(), last_text: None, last_at: None, min_gap: Duration::from_millis(300) }
    }

    pub fn push(&mut self, text: impl Into<String>, urgent: bool) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }
        let now = Instant::now();
        if !urgent {
            if self.last_text.as_deref() == Some(text.as_str()) {
                if let Some(t) = self.last_at {
                    if now.duration_since(t) < self.min_gap {
                        return;
                    }
                }
            }
        }
        self.last_text = Some(text.clone());
        self.last_at = Some(now);
        if self.pending.len() > 32 {
            self.pending.remove(0);
        }
        self.pending.push(Announcement { text, urgent });
    }

    /// Everything queued since the last call, oldest first.
    pub fn drain(&mut self) -> Vec<Announcement> {
        std::mem::take(&mut self.pending)
    }
}

impl Default for AnnounceQueue {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Speech (optional; requires speech-dispatcher's `spd-say` on $PATH)
// ---------------------------------------------------------------------------

pub fn speech_available() -> bool {
    crate::pty::which("spd-say").is_some()
}

/// Speak `text` if `enabled`. `spd-say -C` cancels anything currently being
/// said, so an urgent announcement (bell) can pre-empt a long one (output).
pub fn speak(text: &str, enabled: bool, interrupt: bool) {
    if !enabled || text.trim().is_empty() {
        return;
    }
    let mut cmd = Command::new("spd-say");
    if interrupt {
        cmd.arg("-C");
    }
    let _ = cmd.arg("--").arg(text).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

// ---------------------------------------------------------------------------
// Audible bell
// ---------------------------------------------------------------------------

const FREEDESKTOP_BELL_PATHS: &[&str] = &[
    "/usr/share/sounds/freedesktop/stereo/bell.oga",
    "/usr/share/sounds/freedesktop/stereo/complete.oga",
    "/usr/share/sounds/gnome/default/alerts/glass.ogg",
];

/// Best-effort audible bell: the desktop's own bell sound via `paplay`/`ffplay`/`aplay`,
/// falling back to writing BEL to `/dev/tty` (which the *controlling* terminal, if any —
/// not this window — may turn into a beep). Silent no-op if nothing is available.
pub fn ring_bell() {
    if let Some(path) = FREEDESKTOP_BELL_PATHS.iter().find(|p| std::path::Path::new(p).exists()) {
        for player in ["paplay", "ffplay", "aplay"] {
            if crate::pty::which(player).is_some() {
                let mut cmd = Command::new(player);
                if player == "ffplay" {
                    cmd.args(["-nodisp", "-autoexit", "-loglevel", "quiet"]);
                }
                let ok = cmd.arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok();
                if ok {
                    return;
                }
            }
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
        use std::io::Write;
        let _ = f.write_all(b"\x07");
    }
}

// ---------------------------------------------------------------------------
// Contrast
// ---------------------------------------------------------------------------

/// Foreground/background adjusted so every combination in the theme meets
/// `min_ratio` (0 = leave the theme untouched). Cheap enough to call once per
/// theme change; callers should not call it per cell.
pub fn enforce_min_contrast(mut theme: Theme, min_ratio: f32) -> Theme {
    if min_ratio <= 1.0 {
        return theme;
    }
    theme.fg = ensure_contrast(theme.fg, theme.bg, min_ratio);
    for c in theme.ansi.iter_mut() {
        *c = ensure_contrast(*c, theme.bg, min_ratio);
    }
    theme
}

/// `true` when `fg` on `bg` already clears WCAG AA (4.5:1) for normal text.
pub fn meets_aa(fg: Rgb, bg: Rgb) -> bool {
    contrast_ratio(fg, bg) >= 4.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_drops_rapid_duplicates_but_keeps_urgent() {
        let mut q = AnnounceQueue::new();
        q.push("Build finished", false);
        q.push("Build finished", false);
        q.push("Error!", true);
        q.push("Error!", true);
        let out = q.drain();
        assert_eq!(out.len(), 3, "{out:?}");
        assert!(out[0].text == "Build finished" && !out[0].urgent);
        assert!(out[2].urgent);
        assert!(q.drain().is_empty());
    }

    #[test]
    fn blank_announcements_are_ignored() {
        let mut q = AnnounceQueue::new();
        q.push("   ", false);
        assert!(q.drain().is_empty());
    }

    #[test]
    fn contrast_enforcement_improves_a_bad_theme() {
        let mut t = Theme::dark();
        t.bg = [10, 10, 10];
        t.fg = [20, 20, 20];
        assert!(!meets_aa(t.fg, t.bg));
        let fixed = enforce_min_contrast(t, 4.5);
        assert!(meets_aa(fixed.fg, fixed.bg));
    }

    #[test]
    fn zero_ratio_leaves_theme_untouched() {
        let t = Theme::dark();
        let same = enforce_min_contrast(t.clone(), 0.0);
        assert_eq!(t.fg, same.fg);
        assert_eq!(t.ansi, same.ansi);
    }
}
