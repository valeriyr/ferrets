//! Streaming replay writer.

use std::io::Write;

use crate::{format, header::ReplayHeader, record::TickRecord};

/// Writes a replay to an output stream as the game runs.
///
/// Each record is flushed as it is written, so the output holds a valid replay up
/// to the last completed tick even if the process stops abruptly.
pub struct Recorder {
    writer: Box<dyn Write + Send>,
}

impl Recorder {
    /// Starts a recording, writing the magic prelude and `header` to `writer`.
    pub fn new(writer: impl Write + Send + 'static, header: &ReplayHeader) -> crate::Result<Self> {
        let mut writer: Box<dyn Write + Send> = Box::new(writer);

        format::write_prelude(&mut writer, header)?;
        writer.flush()?;

        Ok(Self { writer })
    }

    /// Appends one tick's record and flushes it to the stream.
    pub fn record(&mut self, record: &TickRecord) -> crate::Result<()> {
        format::write_record(&mut self.writer, record)?;

        self.writer.flush()?;

        Ok(())
    }
}
