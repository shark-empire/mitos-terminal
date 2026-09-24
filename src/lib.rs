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
//! * [`ipc`], [`pkg_bridge`], [`platform`] — MITOS ecosystem + desktop glue.

pub mod a11y;
pub mod app;
pub mod config;
pub mod fx;
pub mod input;
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
