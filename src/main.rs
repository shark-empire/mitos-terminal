//! Binary entry point. Everything else lives in the library crate
//! (`mitos_terminal`, see `lib.rs`) so it can be unit- and integration-tested
//! without a display; this file only builds the window and starts eframe.

use mitos_terminal::app::TerminalApp;

fn make_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 650.0])
            .with_min_inner_size([320.0, 160.0])
            .with_title("MITOS Terminal")
            // Needed for `[window] opacity` / the "glass" look: the pane
            // background is painted with its own alpha (see `app.rs`), which
            // only shows anything behind it if the OS window itself allows
            // transparency.
            .with_transparent(true),
        ..Default::default()
    }
}

fn main() -> eframe::Result<()> {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("mitos-terminal {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let result = eframe::run_native("mitos-terminal", make_options(), Box::new(|cc| Ok(Box::new(TerminalApp::new(cc)))));

    // Fallback renderer: a GPU/driver that refuses to give us a working
    // context is rare but not impossible (headless boxes, broken Mesa
    // installs, some VMs); retry once with Mesa's software rasteriser rather
    // than leaving the user with nothing.
    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!("[mitos-terminal] hardware-accelerated rendering failed ({e}); retrying with a software fallback renderer");
            std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
            eframe::run_native("mitos-terminal", make_options(), Box::new(|cc| Ok(Box::new(TerminalApp::new(cc)))))
        }
    }
}
