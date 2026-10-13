//! Binary entry point. Everything else lives in the library crate
//! (`mitos_terminal`, see `lib.rs`) so it can be unit- and integration-tested
//! without a display; this file only builds the window and starts eframe.

use mitos_terminal::app::TerminalApp;
use mitos_terminal::config::Config;

fn make_options() -> eframe::NativeOptions {
    // A lightweight, throwaway load just to size the window before eframe
    // creates it — `TerminalApp::new` loads the config again (cheap, and it
    // needs the full result anyway) once the real window exists.
    let (cfg, _err) = Config::load();
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([cfg.window.width, cfg.window.height])
            .with_min_inner_size([320.0, 160.0])
            .with_title("MITOS Terminal")
            .with_decorations(cfg.window.decorations)
            // Needed for `[window] opacity` / the glass theme presets: each
            // pane paints its own translucent surface (see `render.rs`),
            // which only shows anything behind it if the OS window itself
            // allows transparency.
            .with_transparent(true),
        ..Default::default()
    }
}

fn main() -> eframe::Result<()> {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("mitos-terminal {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let result = eframe::run_native(
        "mitos-terminal",
        make_options(),
        Box::new(|cc| Ok(Box::new(TerminalApp::new(cc)))),
    );

    // Fallback renderer: a GPU/driver that refuses to give us a working
    // context is rare but not impossible (headless boxes, broken Mesa
    // installs, some VMs); relaunch once with Mesa's software rasteriser
    // rather than leaving the user with nothing. This has to be a fresh
    // process: winit allows only one event loop per process, so a second
    // `run_native` in this one cannot work.
    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            if std::env::var_os("LIBGL_ALWAYS_SOFTWARE").is_some() {
                // Already the software path (or forced by the user): nothing left to try.
                return Err(e);
            }
            eprintln!("[mitos-terminal] hardware-accelerated rendering failed ({e}); relaunching with a software fallback renderer");
            relaunch_with_software_rendering(e)
        }
    }
}

/// Replaces this process with a copy of itself that has `LIBGL_ALWAYS_SOFTWARE=1` set. Only
/// returns (with the original error) if that is impossible.
#[cfg(unix)]
fn relaunch_with_software_rendering(original: eframe::Error) -> eframe::Result<()> {
    use std::os::unix::process::CommandExt;
    let Ok(exe) = std::env::current_exe() else {
        return Err(original);
    };
    let failure = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env("LIBGL_ALWAYS_SOFTWARE", "1")
        .exec();
    eprintln!("[mitos-terminal] could not relaunch: {failure}");
    Err(original)
}

#[cfg(not(unix))]
fn relaunch_with_software_rendering(original: eframe::Error) -> eframe::Result<()> {
    Err(original)
}
