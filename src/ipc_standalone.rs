//! Stand-in for `ipc.rs` in a standalone build (`--no-default-features`).
//!
//! Without the MITOS ecosystem there is no other component to talk to, so this exposes the same
//! functions the app calls but nothing listens, nothing is sent and no widgets are injected.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::compat::RichWidget;
use crate::term::Term;
use crate::theme::Rgb;

/// Pane registry of the IPC server in a MITOS build; here it only keeps the app's calls valid.
pub struct Registry;

impl Registry {
    pub fn new() -> Arc<Registry> {
        Arc::new(Registry)
    }

    pub fn add(&self, _id: u64, _term: Arc<Mutex<Term>>) {}

    pub fn remove(&self, _id: u64) {}

    pub fn set_active(&self, _id: u64) {}

    pub fn set_allow_buffer_read(&self, _allow: bool) {}

    pub fn inject_widget(&self, _widget: RichWidget) {}

    pub fn take_theme_override(&self) -> Option<(Rgb, Rgb)> {
        None
    }
}

/// There is no IPC server; returns the path one would have used (nothing exists there).
pub fn spawn_server(_reg: Arc<Registry>, _ctx: egui::Context) -> PathBuf {
    std::env::temp_dir().join(format!("mitos-term-{}.sock", std::process::id()))
}

/// MITOS' notification service is absent, so fall back to the desktop notifier.
pub fn notify_user(title: String, body: String) {
    crate::platform::notify_send(&title, &body);
}

/// There is no completion service to ask.
pub fn request_autocomplete(_term: Arc<Mutex<Term>>, _ctx: egui::Context, _partial: String) {}

/// There is no network service to poll.
pub fn spawn_network_poller(_reg: Arc<Registry>, _ctx: egui::Context) {}
