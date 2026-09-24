//! Desktop-integration odds and ends that do not belong in any other module.

use std::process::{Command, Stdio};
use std::sync::Mutex;

use arboard::Clipboard;

/// `notify-send` fallback for OSC 9/777/9;4 notifications when mitos-gui's own
/// socket is not reachable (e.g. running the terminal standalone).
pub fn notify_send(title: &str, body: &str) {
    let _ = Command::new("notify-send")
        .arg("--app-name=MITOS Terminal")
        .arg(title)
        .arg(body)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
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
        SystemClipboard { main: Mutex::new(None), shadow_primary: Mutex::new(None) }
    }

    fn with_main<T>(&self, f: impl FnOnce(&mut Clipboard) -> Result<T, arboard::Error>) -> Option<T> {
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
        *self.shadow_primary.lock().unwrap_or_else(|p| p.into_inner()) = Some(text);
    }

    pub fn get_primary(&self) -> Option<String> {
        self.shadow_primary.lock().unwrap_or_else(|p| p.into_inner()).clone()
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
