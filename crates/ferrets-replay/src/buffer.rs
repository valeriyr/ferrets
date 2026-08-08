//! In-memory byte streams for recording and playback.

use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
};

/// An owned, `Send + 'static` byte sink whose clones share one buffer: bytes
/// written through any clone can be read back through any other.
#[derive(Clone, Default)]
pub struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl SharedBuffer {
    /// The bytes written so far.
    pub fn bytes(&self) -> Vec<u8> {
        self.0.lock().unwrap().clone()
    }
}

impl Write for SharedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
