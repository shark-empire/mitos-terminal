//! Security policy helpers.
//!
//! The terminal treats everything an application prints as *untrusted*.
//! This module holds the pure decision logic (no GUI, no I/O) so it can be
//! unit-tested:
//!
//! * [`evaluate_link`] — may a hyperlink be opened, must the user confirm, or is it refused?
//! * [`prepare_paste`] — sanitise pasted text, apply bracketed-paste framing, decide if a confirmation is needed.
//! * [`sanitize_command`] / [`is_trusted_command`] — gate for MROP buttons that inject a shell command.
//! * [`file_uri`] — RFC 3986 percent-encoding for `file://` URIs.

use std::path::Path;

// ---------------------------------------------------------------------------
// Hyperlinks
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkMode {
    /// Never open links from the terminal.
    Off,
    /// A plain click opens (selection by dragging still works).
    Click,
    /// Ctrl+click opens (default — hard to trigger by accident).
    CtrlClick,
}

impl LinkMode {
    pub fn parse(s: &str) -> LinkMode {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "disabled" | "never" => LinkMode::Off,
            "click" => LinkMode::Click,
            _ => LinkMode::CtrlClick,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LinkPolicy {
    pub mode: LinkMode,
    pub allowed_schemes: Vec<String>,
    /// Ask when the visible text looks like a *different* URL than the target.
    pub confirm_mismatch: bool,
    /// Always ask before opening anything.
    pub confirm_all: bool,
}

impl Default for LinkPolicy {
    fn default() -> Self {
        LinkPolicy {
            mode: LinkMode::CtrlClick,
            allowed_schemes: ["http", "https", "mailto", "file", "ftp", "ssh"].iter().map(|s| s.to_string()).collect(),
            confirm_mismatch: true,
            confirm_all: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkDecision {
    Open,
    /// Show the full URL and the reason, let the user decide.
    Confirm(String),
    Deny(String),
}

/// Schemes that are never opened, whatever the configuration says.
const ALWAYS_DENIED: &[&str] = &["javascript", "data", "vbscript", "blob", "about"];

pub const MAX_URI_LEN: usize = 2048;

pub fn scheme_of(uri: &str) -> Option<String> {
    let idx = uri.find(':')?;
    let s = &uri[..idx];
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') {
        return None;
    }
    Some(s.to_ascii_lowercase())
}

/// Host part of `scheme://[user@]host[:port]/…`, lowercased, plus whether
/// userinfo was present (`http://google.com@evil.example` is a classic spoof).
pub fn host_of(uri: &str) -> Option<(String, bool)> {
    let rest = &uri[uri.find("://")? + 3..];
    let end = rest.find(|c: char| c == '/' || c == '?' || c == '#').unwrap_or(rest.len());
    let authority = &rest[..end];
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(i) => (true, &authority[i + 1..]),
        None => (false, authority),
    };
    let host = if hostport.starts_with('[') {
        hostport.split(']').next().unwrap_or(hostport).trim_start_matches('[')
    } else {
        hostport.split(':').next().unwrap_or(hostport)
    };
    if host.is_empty() {
        return None;
    }
    Some((host.to_ascii_lowercase(), userinfo))
}

/// Decide what to do with a link. `visible_text` is what the user actually sees
/// (for OSC 8 links; `None` for plain-text URLs, which cannot lie).
pub fn evaluate_link(uri: &str, visible_text: Option<&str>, policy: &LinkPolicy) -> LinkDecision {
    if policy.mode == LinkMode::Off {
        return LinkDecision::Deny("links are disabled in terminal.toml ([links] mode = \"off\")".into());
    }
    if uri.len() > MAX_URI_LEN {
        return LinkDecision::Deny("link is too long".into());
    }
    if uri.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return LinkDecision::Deny("link contains control characters or whitespace".into());
    }
    let scheme = match scheme_of(uri) {
        Some(s) => s,
        None => return LinkDecision::Deny("link has no valid scheme".into()),
    };
    if ALWAYS_DENIED.contains(&scheme.as_str()) {
        return LinkDecision::Deny(format!("`{scheme}:` links are never opened"));
    }
    if !policy.allowed_schemes.iter().any(|s| s.eq_ignore_ascii_case(&scheme)) {
        return LinkDecision::Deny(format!("`{scheme}:` is not in the allowed link schemes"));
    }

    let mut reasons: Vec<String> = Vec::new();
    if policy.confirm_all {
        reasons.push("confirmation is required for every link".into());
    }
    match scheme.as_str() {
        "file" => {
            match host_of(uri) {
                Some((h, _)) if h != "localhost" => reasons.push("file link points at a remote host".into()),
                _ => {}
            }
            reasons.push("this opens a local file".into());
        }
        "ssh" => reasons.push("this starts an SSH client".into()),
        "http" | "https" | "ftp" => {
            if let Some((host, userinfo)) = host_of(uri) {
                if userinfo {
                    reasons.push("the address contains a user@ part, a common phishing trick".into());
                }
                if host.contains("xn--") {
                    reasons.push("the host is an internationalised domain (possible look-alike)".into());
                }
                if policy.confirm_mismatch {
                    if let Some(vis) = visible_text {
                        if let Some(vis_host) = visible_host(vis) {
                            if vis_host != host {
                                reasons.push(format!("the link text says `{vis_host}` but it opens `{host}`"));
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
    if reasons.is_empty() {
        LinkDecision::Open
    } else {
        LinkDecision::Confirm(reasons.join("; "))
    }
}

/// If `text` looks like a URL or a bare domain, its lowercased host.
fn visible_host(text: &str) -> Option<String> {
    let t = text.trim();
    if t.contains("://") {
        return host_of(t).map(|(h, _)| h);
    }
    // bare "example.com/path"
    let first = t.split(|c: char| c == '/' || c == '?' || c == '#').next()?;
    let looks_like_domain = first.contains('.')
        && !first.contains(' ')
        && first.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && first.rsplit('.').next().map(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())).unwrap_or(false);
    if looks_like_domain {
        Some(first.to_ascii_lowercase())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Paste protection
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct PasteConfig {
    /// Ask before pasting multiple lines into a program that did not enable bracketed paste.
    pub confirm_multiline: bool,
    /// Ask before pasting more than this many bytes.
    pub confirm_bytes: usize,
    /// Hard cap; larger pastes are refused.
    pub max_bytes: usize,
    /// Remove control characters (ESC, NUL, DEL, C1…) from pasted text.
    pub strip_controls: bool,
}

impl Default for PasteConfig {
    fn default() -> Self {
        PasteConfig { confirm_multiline: true, confirm_bytes: 64 * 1024, max_bytes: 4 * 1024 * 1024, strip_controls: true }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paste {
    /// Bytes to send to the child (already framed for bracketed paste if requested).
    pub bytes: Vec<u8>,
    /// `Some(reason)` when the user must confirm before `bytes` is sent.
    pub needs_confirm: Option<String>,
    /// Control characters were removed.
    pub stripped: bool,
    /// The paste exceeded `max_bytes` and was refused; `bytes` is empty.
    pub refused: bool,
}

/// Turn clipboard text into the bytes to send.
///
/// * newlines are normalised to `\n` first;
/// * with `strip_controls`, everything except `\t` and `\n` below U+0020, DEL and C1 controls is dropped
///   — this alone defeats "paste-jacking" (`ESC[201~` to break out of bracketed paste, embedded escape sequences);
/// * without bracketed paste, `\n` becomes `\r` (what Enter sends);
/// * with bracketed paste, the text is wrapped in `ESC[200~ … ESC[201~`.
pub fn prepare_paste(text: &str, bracketed: bool, cfg: &PasteConfig) -> Paste {
    if text.len() > cfg.max_bytes {
        return Paste { bytes: Vec::new(), needs_confirm: None, stripped: false, refused: true };
    }
    let mut clean = String::with_capacity(text.len());
    let mut stripped = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                clean.push('\n');
            }
            '\n' | '\t' => clean.push(c),
            c if cfg.strip_controls && (c.is_control()) => stripped = true,
            c => clean.push(c),
        }
    }
    let multiline = clean.contains('\n');
    let mut reasons: Vec<String> = Vec::new();
    if cfg.confirm_multiline && multiline && !bracketed {
        reasons.push("it has several lines and this program did not ask for bracketed paste — each line would run as a separate command".to_string());
    }
    if clean.len() > cfg.confirm_bytes {
        reasons.push(format!("it is large ({} KiB)", clean.len() / 1024));
    }
    if stripped {
        reasons.push("control characters were removed from it".to_string());
    }

    let payload = if bracketed { clean } else { clean.replace('\n', "\r") };
    let mut bytes = Vec::with_capacity(payload.len() + 12);
    if bracketed {
        bytes.extend_from_slice(b"\x1b[200~");
    }
    bytes.extend_from_slice(payload.as_bytes());
    if bracketed {
        bytes.extend_from_slice(b"\x1b[201~");
    }
    Paste {
        bytes,
        needs_confirm: if reasons.is_empty() { None } else { Some(reasons.join("; ")) },
        stripped,
        refused: false,
    }
}

/// A short, safe-to-display preview of pasted text (control characters made visible).
pub fn paste_preview(text: &str, max_lines: usize, max_cols: usize) -> String {
    let mut out = String::new();
    let mut total = 0usize;
    for (i, line) in text.replace("\r\n", "\n").replace('\r', "\n").split('\n').enumerate() {
        total = i + 1;
        if i >= max_lines {
            continue;
        }
        let mut shown = String::new();
        for c in line.chars().take(max_cols) {
            match c {
                '\t' => shown.push('→'),
                c if c.is_control() => {
                    let code = c as u32;
                    if code < 0x20 {
                        shown.push('^');
                        shown.push((b'@' + code as u8) as char);
                    } else {
                        shown.push('�');
                    }
                }
                c => shown.push(c),
            }
        }
        if line.chars().count() > max_cols {
            shown.push('…');
        }
        out.push_str(&shown);
        out.push('\n');
    }
    if total > max_lines {
        out.push_str(&format!("… {} more line(s)\n", total - max_lines));
    }
    out
}

// ---------------------------------------------------------------------------
// MROP command gating
// ---------------------------------------------------------------------------

/// A single-line command safe to type into a shell, or `None`.
/// Newlines are refused (they would run several commands) as are all other control characters.
pub fn sanitize_command(cmd: &str) -> Option<String> {
    let c = cmd.trim();
    if c.is_empty() || c.len() > 1024 || c.chars().any(|ch| ch.is_control()) {
        return None;
    }
    Some(c.to_string())
}

/// Commands starting with one of the configured prefixes run without a confirmation prompt.
pub fn is_trusted_command(cmd: &str, prefixes: &[String]) -> bool {
    prefixes.iter().any(|p| !p.is_empty() && cmd.starts_with(p.as_str()))
}

// ---------------------------------------------------------------------------
// URIs
// ---------------------------------------------------------------------------

/// `file://` URI for a local path, percent-encoding everything except unreserved characters and `/`.
pub fn file_uri(path: &Path) -> String {
    let s = path.to_string_lossy();
    let mut out = String::from("file://");
    for b in s.as_bytes() {
        let c = *b;
        if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{:02X}", c));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pol() -> LinkPolicy {
        LinkPolicy::default()
    }

    #[test]
    fn plain_https_link_opens() {
        assert_eq!(evaluate_link("https://example.com/a?b=c", None, &pol()), LinkDecision::Open);
    }

    #[test]
    fn dangerous_schemes_are_denied_even_if_allowed() {
        let mut p = pol();
        p.allowed_schemes.push("javascript".into());
        p.allowed_schemes.push("data".into());
        assert!(matches!(evaluate_link("javascript:alert(1)", None, &p), LinkDecision::Deny(_)));
        assert!(matches!(evaluate_link("data:text/html,hi", None, &p), LinkDecision::Deny(_)));
        assert!(matches!(evaluate_link("gopher://x", None, &pol()), LinkDecision::Deny(_)));
    }

    #[test]
    fn disabled_mode_denies_everything() {
        let mut p = pol();
        p.mode = LinkMode::Off;
        assert!(matches!(evaluate_link("https://example.com", None, &p), LinkDecision::Deny(_)));
    }

    #[test]
    fn spoofed_link_text_needs_confirmation() {
        let d = evaluate_link("https://evil.example/login", Some("https://bank.example"), &pol());
        assert!(matches!(d, LinkDecision::Confirm(ref r) if r.contains("bank.example")), "{d:?}");
        assert_eq!(evaluate_link("https://bank.example/x", Some("bank.example"), &pol()), LinkDecision::Open);
        // arbitrary prose is not treated as a URL
        assert_eq!(evaluate_link("https://bank.example", Some("click here"), &pol()), LinkDecision::Open);
    }

    #[test]
    fn userinfo_and_idn_are_flagged() {
        assert!(matches!(evaluate_link("http://google.com@evil.example/", None, &pol()), LinkDecision::Confirm(_)));
        assert!(matches!(evaluate_link("https://xn--pple-43d.com/", None, &pol()), LinkDecision::Confirm(_)));
    }

    #[test]
    fn file_and_ssh_links_confirm() {
        assert!(matches!(evaluate_link("file:///etc/passwd", None, &pol()), LinkDecision::Confirm(_)));
        assert!(matches!(evaluate_link("ssh://host", None, &pol()), LinkDecision::Confirm(_)));
    }

    #[test]
    fn control_characters_and_length_are_refused() {
        assert!(matches!(evaluate_link("https://a.b/\u{1b}[31m", None, &pol()), LinkDecision::Deny(_)));
        assert!(matches!(evaluate_link("https://a.b/ x", None, &pol()), LinkDecision::Deny(_)));
        let long = format!("https://a.b/{}", "x".repeat(3000));
        assert!(matches!(evaluate_link(&long, None, &pol()), LinkDecision::Deny(_)));
    }

    #[test]
    fn host_parsing() {
        assert_eq!(host_of("https://User@Example.COM:8080/x"), Some(("example.com".to_string(), true)));
        assert_eq!(host_of("http://[::1]:80/"), Some(("::1".to_string(), false)));
        assert_eq!(host_of("mailto:a@b"), None);
    }

    #[test]
    fn bracketed_paste_is_framed_and_cannot_be_escaped() {
        let cfg = PasteConfig::default();
        let p = prepare_paste("ls\n\x1b[201~rm -rf ~", true, &cfg);
        let s = String::from_utf8(p.bytes.clone()).unwrap();
        assert!(s.starts_with("\x1b[200~") && s.ends_with("\x1b[201~"));
        // the only ESC[201~ is the closing one
        assert_eq!(s.matches("\x1b[201~").count(), 1);
        assert!(p.stripped);
        assert!(p.needs_confirm.is_some());
    }

    #[test]
    fn plain_paste_converts_newlines_and_asks_for_multiline() {
        let cfg = PasteConfig::default();
        let p = prepare_paste("a\r\nb\nc", false, &cfg);
        assert_eq!(p.bytes, b"a\rb\rc".to_vec());
        assert!(p.needs_confirm.is_some());
        let single = prepare_paste("echo hi", false, &cfg);
        assert_eq!(single.bytes, b"echo hi".to_vec());
        assert!(single.needs_confirm.is_none());
    }

    #[test]
    fn multiline_in_bracketed_mode_needs_no_confirmation() {
        let p = prepare_paste("a\nb", true, &PasteConfig::default());
        assert!(p.needs_confirm.is_none());
        assert_eq!(p.bytes, b"\x1b[200~a\nb\x1b[201~".to_vec());
    }

    #[test]
    fn size_limits() {
        let cfg = PasteConfig { max_bytes: 10, ..PasteConfig::default() };
        assert!(prepare_paste(&"x".repeat(11), false, &cfg).refused);
        let cfg = PasteConfig { confirm_bytes: 4, ..PasteConfig::default() };
        assert!(prepare_paste("hello", false, &cfg).needs_confirm.is_some());
    }

    #[test]
    fn preview_shows_control_characters() {
        let p = paste_preview("a\tb\x1bc\nline2\nline3", 2, 80);
        assert!(p.contains("a→b^[c"));
        assert!(p.contains("1 more line"));
    }

    #[test]
    fn command_sanitising() {
        assert_eq!(sanitize_command("  mitos-pkg install foo "), Some("mitos-pkg install foo".to_string()));
        assert_eq!(sanitize_command("ls\nrm -rf ~"), None);
        assert_eq!(sanitize_command("ls\x1b[31m"), None);
        assert_eq!(sanitize_command(""), None);
        let prefixes = vec!["mitos-pkg install ".to_string()];
        assert!(is_trusted_command("mitos-pkg install htop", &prefixes));
        assert!(!is_trusted_command("rm -rf /", &prefixes));
        assert!(!is_trusted_command("anything", &[String::new()]));
    }

    #[test]
    fn file_uri_encoding() {
        assert_eq!(file_uri(Path::new("/home/a b/ü.txt")), "file:///home/a%20b/%C3%BC.txt");
    }
}
