//! The few types the terminal shares with the rest of MITOS (`mitos-utils`).
//!
//! With the default `mitos` feature these are the real ones. A standalone build
//! (`--no-default-features`, e.g. on macOS) has no `mitos-utils`, so small local
//! stand-ins take their place: widgets and the private escape sequences still work
//! inside the terminal, there is just nothing outside it to talk to.

#[cfg(feature = "mitos")]
pub use mitos_utils::ipc::{RichWidget, OSC_NEW_BLOCK, OSC_WIDGET};

#[cfg(not(feature = "mitos"))]
pub use self::standalone::{RichWidget, OSC_NEW_BLOCK, OSC_WIDGET};

#[cfg(not(feature = "mitos"))]
mod standalone {
    use serde::{Deserialize, Serialize};

    /// OSC command that carries a JSON-encoded `RichWidget`.
    pub const OSC_WIDGET: &str = "MITOS_WIDGET";
    /// OSC command that starts a new execution block (its payload is the prompt text).
    pub const OSC_NEW_BLOCK: &str = "MITOS_NEW_BLOCK";

    /// An inline widget a program can draw under a line of output.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub enum RichWidget {
        Button { label: String, cmd: String },
        Progress { percent: f32, label: String },
        Sparkline { values: Vec<f32>, label: String },
    }
}
