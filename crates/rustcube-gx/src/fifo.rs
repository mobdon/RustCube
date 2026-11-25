//! The GX FIFO as a byte sink: what the game writes to the write-gather
//! pipe (0xCC008000) arrives here in order, in pieces of any size, and runs
//! through the same command parser as display lists.
use crate::api::{Gx, GxMemory};

/// Buffers partial commands across writes and runs each complete one.
#[derive(Default)]
pub struct GxFifo {
    pub gx: Gx,
    pending: Vec<u8>,
}

impl GxFifo {
    pub fn new(gx: Gx) -> Self {
        Self {
            gx,
            pending: Vec::new(),
        }
    }

    /// Bytes the pipe received (big-endian, as written); `mem` serves
    /// vertex arrays, indexed loads, textures and called display lists.
    pub fn write(&mut self, bytes: &[u8], mem: &dyn GxMemory) {
        self.pending.extend_from_slice(bytes);
        let mut at = 0;
        while let Some(n) = self.gx.command_len(&self.pending[at..]) {
            self.gx.run_display_list(&self.pending[at..at + n], mem);
            at += n;
        }
        self.pending.drain(..at);
    }

    /// Bytes held for an incomplete command.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
}
