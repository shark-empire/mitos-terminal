//! MITOS ecosystem IPC.
//!
//! * **Server** (`/tmp/mitos-term-<pid>.sock` via `mitos_utils::ipc::terminal_socket`):
//!   other MITOS daemons can read the terminal buffer, inject widgets and push
//!   theme changes. Hardening: socket mode 0600, every peer's uid is verified
//!   against ours (`SO_PEERCRED`), at most 8 concurrent clients, 60 s idle timeout,
//!   and buffer reads can be switched off in `terminal.toml`.
//! * **Clients**: notifications to mitos-gui, path autocompletion from the file
//!   manager, captive-portal polling of the network daemon.
//!
//! One small shared tokio runtime serves all of it (the old code created four
//! full multi-threaded runtimes).

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use mitos_utils::ipc::{self, IpcRequest, IpcResponse, RichWidget};
use tokio::net::{UnixListener, UnixStream};

use crate::term::Term;
use crate::theme::Rgb;

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// The shared async runtime (2 worker threads).
pub fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("mitos-ipc")
            .enable_all()
            .build()
            .expect("failed to start the IPC runtime")
    })
}

/// Cap on how much history a `GetTerminalBuffer` reply carries.
const MAX_SNAPSHOT_LINES: usize = 5000;
const MAX_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Registry: what the IPC server may touch
// ---------------------------------------------------------------------------

struct RegEntry {
    id: u64,
    term: Arc<Mutex<Term>>,
}

/// The set of live sessions, shared between the UI thread and the IPC tasks.
pub struct Registry {
    sessions: Mutex<Vec<RegEntry>>,
    active: AtomicU64,
    theme_override: Mutex<Option<(Rgb, Rgb)>>,
    theme_pending: AtomicBool,
    allow_buffer_read: AtomicBool,
}

impl Registry {
    pub fn new() -> Arc<Registry> {
        Arc::new(Registry {
            sessions: Mutex::new(Vec::new()),
            active: AtomicU64::new(0),
            theme_override: Mutex::new(None),
            theme_pending: AtomicBool::new(false),
            allow_buffer_read: AtomicBool::new(true),
        })
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, Vec<RegEntry>> {
        self.sessions.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn add(&self, id: u64, term: Arc<Mutex<Term>>) {
        self.entries().push(RegEntry { id, term });
    }

    pub fn remove(&self, id: u64) {
        self.entries().retain(|e| e.id != id);
    }

    pub fn set_active(&self, id: u64) {
        self.active.store(id, Ordering::Relaxed);
    }

    pub fn set_allow_buffer_read(&self, allow: bool) {
        self.allow_buffer_read.store(allow, Ordering::Relaxed);
    }

    fn active_term(&self) -> Option<Arc<Mutex<Term>>> {
        let active = self.active.load(Ordering::Relaxed);
        let entries = self.entries();
        entries
            .iter()
            .find(|e| e.id == active)
            .or_else(|| entries.first())
            .map(|e| Arc::clone(&e.term))
    }

    /// Buffer text for `GetTerminalBuffer`: the active session first, then the others.
    fn snapshot(&self) -> (String, String) {
        if !self.allow_buffer_read.load(Ordering::Relaxed) {
            return (String::new(), String::new());
        }
        let active = self.active.load(Ordering::Relaxed);
        let entries = self.entries();
        let mut prompt = String::new();
        let mut text = String::new();
        let mut ordered: Vec<&RegEntry> = entries.iter().collect();
        ordered.sort_by_key(|e| if e.id == active { 0 } else { 1 });
        for (i, e) in ordered.iter().enumerate() {
            let t = e.term.lock().unwrap_or_else(|p| p.into_inner());
            if i == 0 {
                prompt = t.block_prompt().to_string();
            }
            if ordered.len() > 1 {
                text.push_str(&format!("── session {} ({}) ──\n", e.id, t.title()));
            }
            text.push_str(&t.snapshot_text(MAX_SNAPSHOT_LINES));
            if text.len() > MAX_SNAPSHOT_BYTES {
                text.truncate(MAX_SNAPSHOT_BYTES);
                break;
            }
        }
        (prompt, text)
    }

    /// Inject a widget into the active session (IPC peers are same-user, hence trusted).
    pub fn inject_widget(&self, widget: RichWidget) {
        if let Some(t) = self.active_term() {
            t.lock().unwrap_or_else(|p| p.into_inner()).inject_widget(widget, true);
        }
    }

    fn set_theme_override(&self, fg: Rgb, bg: Rgb) {
        *self.theme_override.lock().unwrap_or_else(|p| p.into_inner()) = Some((fg, bg));
        self.theme_pending.store(true, Ordering::Release);
    }

    /// `Some((fg, bg))` once after a `ThemeChanged` request arrived.
    pub fn take_theme_override(&self) -> Option<(Rgb, Rgb)> {
        if self.theme_pending.swap(false, Ordering::AcqRel) {
            *self.theme_override.lock().unwrap_or_else(|p| p.into_inner())
        } else {
            None
        }
    }

    fn inject_captive_portal_widget(&self) {
        if let Some(t) = self.active_term() {
            let mut g = t.lock().unwrap_or_else(|p| p.into_inner());
            let already = g
                .widgets()
                .iter()
                .any(|w| matches!(&w.widget, RichWidget::Button { label, .. } if label.contains("Login Page")));
            if !already {
                g.inject_widget(
                    RichWidget::Button {
                        label: "🌐 Open Login Page".to_string(),
                        cmd: "xdg-open http://captive.apple.com".to_string(),
                    },
                    true,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

fn same_user(stream: &UnixStream) -> bool {
    match stream.peer_cred() {
        // SAFETY: getuid(2) is always safe.
        Ok(cred) => cred.uid() == unsafe { libc::getuid() },
        Err(_) => false,
    }
}

/// Start the IPC server. Returns the socket path (remove it on exit).
pub fn spawn_server(reg: Arc<Registry>, ctx: egui::Context) -> PathBuf {
    let socket_path = ipc::terminal_socket(std::process::id());
    let _ = std::fs::remove_file(&socket_path);
    let path_for_task = socket_path.clone();

    runtime().spawn(async move {
        let listener = match UnixListener::bind(&path_for_task) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[mitos-terminal] IPC bind failed: {e}");
                return;
            }
        };
        // Only the owner may connect (the connect(2) permission check needs write access).
        let _ = std::fs::set_permissions(&path_for_task, std::fs::Permissions::from_mode(0o600));
        eprintln!("[mitos-terminal] IPC listening on {:?}", path_for_task);

        let limiter = Arc::new(tokio::sync::Semaphore::new(8));
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            };
            if !same_user(&stream) {
                continue; // dropped: a different user has no business here
            }
            let permit = match Arc::clone(&limiter).try_acquire_owned() {
                Ok(p) => p,
                Err(_) => continue,
            };
            let reg = Arc::clone(&reg);
            let ctx = ctx.clone();
            tokio::spawn(async move {
                handle_client(stream, reg, ctx).await;
                drop(permit);
            });
        }
    });
    socket_path
}

async fn handle_client(mut stream: UnixStream, reg: Arc<Registry>, ctx: egui::Context) {
    loop {
        let req = match tokio::time::timeout(Duration::from_secs(60), ipc::ipc_recv::<IpcRequest>(&mut stream)).await {
            Ok(Ok(Some(r))) => r,
            _ => break,
        };
        let resp = match req {
            IpcRequest::GetTerminalBuffer => {
                let (prompt, text) = reg.snapshot();
                IpcResponse::BufferData { pid: std::process::id(), prompt, text }
            }
            IpcRequest::InjectWidget { widget } => {
                reg.inject_widget(widget);
                ctx.request_repaint();
                IpcResponse::Ack
            }
            IpcRequest::ThemeChanged { bg, fg } => {
                reg.set_theme_override(fg, bg);
                ctx.request_repaint();
                IpcResponse::Ack
            }
            IpcRequest::AutoCompletePath { .. } => IpcResponse::AutoCompleteResult { suggestions: vec![] },
            _ => IpcResponse::Ack,
        };
        if ipc::ipc_send(&mut stream, &resp).await.is_err() {
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Clients
// ---------------------------------------------------------------------------

/// Ask mitos-gui to show a notification; fall back to `notify-send`.
pub fn notify_user(title: String, body: String) {
    runtime().spawn(async move {
        let mut delivered = false;
        if let Ok(mut stream) = UnixStream::connect(ipc::gui_socket()).await {
            let req = IpcRequest::NotifyUser { title: title.clone(), body: body.clone() };
            delivered = ipc::ipc_send(&mut stream, &req).await.is_ok();
        }
        if !delivered {
            crate::platform::notify_send(&title, &body);
        }
    });
}

/// Ask the file manager for a completion of `partial` and show it as ghost text.
pub fn request_autocomplete(term: Arc<Mutex<Term>>, ctx: egui::Context, partial: String) {
    runtime().spawn(async move {
        let mut stream = match UnixStream::connect(ipc::file_manager_socket()).await {
            Ok(s) => s,
            Err(_) => return,
        };
        let req = IpcRequest::AutoCompletePath { partial_path: partial };
        if ipc::ipc_send(&mut stream, &req).await.is_err() {
            return;
        }
        let reply = tokio::time::timeout(Duration::from_secs(2), ipc::ipc_recv::<IpcResponse>(&mut stream)).await;
        if let Ok(Ok(Some(IpcResponse::AutoCompleteResult { suggestions }))) = reply {
            if let Some(first) = suggestions.into_iter().next() {
                if let Ok(mut t) = term.lock() {
                    t.set_ghost_text(Some(first));
                }
                ctx.request_repaint();
            }
        }
    });
}

/// Poll the network daemon every 3 s and offer a login button on captive portals.
pub fn spawn_network_poller(reg: Arc<Registry>, ctx: egui::Context) {
    runtime().spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3));
        loop {
            interval.tick().await;
            if let Ok(mut stream) = UnixStream::connect(ipc::network_socket()).await {
                let req = IpcRequest::GetNetworkStatus;
                if ipc::ipc_send(&mut stream, &req).await.is_ok() {
                    let reply = tokio::time::timeout(Duration::from_secs(2), ipc::ipc_recv::<IpcResponse>(&mut stream)).await;
                    if let Ok(Ok(Some(IpcResponse::NetworkStatus { is_captive_portal, .. }))) = reply {
                        if is_captive_portal {
                            reg.inject_captive_portal_widget();
                            ctx.request_repaint();
                        }
                    }
                }
            }
        }
    });
}
