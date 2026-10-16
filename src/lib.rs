//! MITOS Terminal — library crate.
//!
//! Layering (bottom to top):
//!
//! * [`term`]     — pure VT/xterm screen model. No GUI, no I/O. Fully unit-tested.
//! * [`pty`]      — spawn / resize / signal a child on a pseudo-terminal.
//! * [`session`]  — one PTY + one [`term::Term`] + I/O threads + lifecycle + crash isolation.
//! * [`input`]    — keyboard / mouse / paste → bytes for the child.
//! * [`security`] — hyperlink, paste, clipboard and widget policy.
//! * [`config`], [`theme`] — `terminal.toml`, `home.conf`, palettes, contrast.
//! * [`render`], [`fx`], [`a11y`] — egui painting of a grid (+ cinematic effects).
//! * [`layout`], [`app`] — tabs, split panes, menus, dialogs, command palette.
//! * [`ipc`], [`pkg_bridge`], [`compat`], [`platform`] — MITOS ecosystem + desktop glue
//!   (the `mitos` feature; a standalone build swaps in local stand-ins).

pub mod a11y;
pub mod app;
pub mod compat;
pub mod config;
pub mod fx;
pub mod input;
#[cfg(feature = "mitos")]
pub mod ipc;
#[cfg(not(feature = "mitos"))]
#[path = "ipc_standalone.rs"]
pub mod ipc;
pub mod layout;
pub mod pkg_bridge;
pub mod platform;
pub mod pty;
pub mod render;
pub mod security;
pub mod session;
pub mod term;
pub mod theme;
