//! Desktop-integration odds and ends that do not belong in any other module.

use std::process::{Command, Stdio};
use std::sync::Mutex;

use arboard::Clipboard;

/// Desktop notification for OSC 9/777/9;4 notifications when mitos-gui's own socket is not
/// reachable (e.g. running the terminal standalone): `notify-send` on Linux, `osascript` on macOS.
pub fn notify_send(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        // The text comes from whatever program is running in the terminal, so it is handed to a
        // fixed script as `argv` instead of being spliced into AppleScript source (which would let
        // an escape sequence run arbitrary AppleScript). `--` keeps a title or body that starts
        // with `-` from being read as an option.
        let _ = Command::new("/usr/bin/osascript")
            .args([
                "-e",
                "on run argv",
                "-e",
                "display notification (item 2 of argv) with title (item 1 of argv)",
                "-e",
                "end run",
                "--",
            ])
            .arg(title)
            .arg(body)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = Command::new("notify-send")
            .arg("--app-name=MITOS Terminal")
            .arg(title)
            .arg(body)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

/// This machine's host name (`gethostname(3)`), for the status strip and for deciding whether an
/// OSC 7 `file://host/...` URL is local. This replaces reading `/proc/sys/kernel/hostname`, which
/// does not exist on macOS (so the host was missing from the strip and every OSC 7 working-directory
/// report from the shell was rejected as "remote").
pub fn hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: `gethostname` writes at most `buf.len()` bytes into the buffer.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if rc != 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let name = String::from_utf8_lossy(&buf[..end]).trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Ask the window manager to flash/highlight the taskbar entry (bell, in an
/// unfocused window). Best-effort: works under X11 with `wmctrl`; a Wayland
/// compositor without an equivalent simply ignores it.
pub fn request_urgent(title_hint: &str) {
    let _ = Command::new("wmctrl")
        .arg("-r")
        .arg(title_hint)
        .arg("-b")
        .arg("add,demands_attention")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// System clipboard, plus (on X11) the PRIMARY selection.
///
/// `arboard::Clipboard` is `!Send` on some backends (it can hold an X11
/// connection), so the instance is created lazily on whichever thread first
/// touches it and kept there; call these accessors from the UI thread only.
pub struct SystemClipboard {
    main: Mutex<Option<Clipboard>>,
    /// Best-effort shadow copy of the last selection, returned by
    /// `get_primary` when the platform clipboard crate has no portable
    /// PRIMARY-selection API. Real X11 PRIMARY-selection support (so a
    /// *different* app's middle-click paste sees it) needs a platform crate
    /// feature this project does not currently pin a verified version of;
    /// tracked in docs/AUDIT.md rather than guessed at here.
    shadow_primary: Mutex<Option<String>>,
}

impl SystemClipboard {
    pub fn new() -> SystemClipboard {
        SystemClipboard {
            main: Mutex::new(None),
            shadow_primary: Mutex::new(None),
        }
    }

    fn with_main<T>(
        &self,
        f: impl FnOnce(&mut Clipboard) -> Result<T, arboard::Error>,
    ) -> Option<T> {
        let mut slot = self.main.lock().unwrap_or_else(|p| p.into_inner());
        if slot.is_none() {
            *slot = Clipboard::new().ok();
        }
        f(slot.as_mut()?).ok()
    }

    pub fn set_text(&self, text: String) -> bool {
        self.with_main(|c| c.set_text(text)).is_some()
    }

    pub fn get_text(&self) -> Option<String> {
        self.with_main(|c| c.get_text())
    }

    /// Middle-click / PRIMARY-selection paste target. See the field doc above
    /// for why this is in-process only rather than a real X11 PRIMARY.
    pub fn set_primary(&self, text: String) {
        *self
            .shadow_primary
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(text);
    }

    pub fn get_primary(&self) -> Option<String> {
        self.shadow_primary
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

impl Default for SystemClipboard {
    fn default() -> Self {
        Self::new()
    }
}

/// Open a URL/path with the desktop's default handler.
pub fn open_external(target: &str) -> bool {
    open::that_detached(target).is_ok()
}
