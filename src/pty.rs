use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use anyhow::Result;

pub struct MitosPty {
    pub master: Box<dyn portable_pty::MasterPty + Send>,
}

impl MitosPty {
    pub fn new(cols: u16, rows: u16) -> Result<Self> {
        let pty_system = NativePtySystem::default();
        
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        // TODO: Change "sh" to "mitos-shell" once your shell is compiled and in PATH
        let mut cmd = CommandBuilder::new("sh"); 
        cmd.env("TERM", "xterm-256color");
        
        let _child = pair.slave.spawn_command(cmd)?;
        Ok(Self { master: pair.master })
    }
    
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(PtySize {
            rows, cols, pixel_width: 0, pixel_height: 0,
        })?;
        Ok(())
    }
}
