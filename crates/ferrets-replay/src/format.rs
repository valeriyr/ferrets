//! On-disk replay encoding: a magic prelude, the header, then a stream of
//! length-prefixed records.
//!
//! Each chunk is a `u32` little-endian length followed by its `bcs` bytes. A
//! recording flushes after every chunk, so a stream cut short mid-write ends in a
//! truncated chunk; reading treats that as end-of-stream and keeps every complete
//! chunk before it.

use std::io::{Read, Write};

use crate::{
    error::ReplayError,
    header::{FORMAT_VERSION, ReplayHeader},
    record::TickRecord,
};

/// Identifies a ferrets replay stream.
const MAGIC: [u8; 4] = *b"FREP";

/// How much of a buffer a read filled.
#[derive(PartialEq, Eq)]
enum ReadOutcome {
    /// The buffer was filled completely.
    Full,
    /// The stream ended after some but not all of the buffer.
    Partial,
    /// The stream ended before any bytes were read.
    Eof,
}

/// Writes the magic prelude and the header.
pub fn write_prelude(writer: &mut dyn Write, header: &ReplayHeader) -> crate::Result<()> {
    writer.write_all(&MAGIC)?;
    write_chunk(writer, &bcs::to_bytes(header)?)
}

/// Writes one tick record.
pub fn write_record(writer: &mut dyn Write, record: &TickRecord) -> crate::Result<()> {
    write_chunk(writer, &bcs::to_bytes(record)?)
}

/// Reads and validates the magic prelude and header.
pub fn read_prelude<R: Read>(reader: &mut R) -> crate::Result<ReplayHeader> {
    let mut magic = [0u8; 4];
    if read(reader, &mut magic)? != ReadOutcome::Full || magic != MAGIC {
        return Err(ReplayError::BadMagic);
    }

    // The header is mandatory: unlike a trailing record, a missing or truncated
    // one means the stream ended before a valid replay was even written.
    let bytes = read_chunk(reader)?
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::UnexpectedEof))?;
    let header: ReplayHeader = bcs::from_bytes(&bytes)?;

    if header.format_version != FORMAT_VERSION {
        return Err(ReplayError::UnsupportedVersion {
            found: header.format_version,
            expected: FORMAT_VERSION,
        });
    }

    Ok(header)
}

/// Reads the next tick record, or `None` at the end of the stream (including a
/// truncated trailing chunk from a recording that was cut short).
pub fn read_record<R: Read>(reader: &mut R) -> crate::Result<Option<TickRecord>> {
    match read_chunk(reader)? {
        Some(bytes) => Ok(Some(bcs::from_bytes(&bytes)?)),
        None => Ok(None),
    }
}

/// Writes a `u32` little-endian length prefix followed by `bytes`.
fn write_chunk(writer: &mut dyn Write, bytes: &[u8]) -> crate::Result<()> {
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(bytes)?;
    Ok(())
}

/// Reads one length-prefixed chunk, or `None` at a clean or truncated end.
fn read_chunk<R: Read>(reader: &mut R) -> crate::Result<Option<Vec<u8>>> {
    let mut len_bytes = [0u8; 4];
    if read(reader, &mut len_bytes)? != ReadOutcome::Full {
        return Ok(None);
    }

    let len = u32::from_le_bytes(len_bytes) as usize;
    let mut bytes = vec![0u8; len];
    if read(reader, &mut bytes)? != ReadOutcome::Full {
        return Ok(None);
    }

    Ok(Some(bytes))
}

/// Reads until `buf` is full or the stream ends, retrying on interruption.
fn read<R: Read>(reader: &mut R, buf: &mut [u8]) -> crate::Result<ReadOutcome> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }

    let outcome = if filled == 0 {
        ReadOutcome::Eof
    } else if filled == buf.len() {
        ReadOutcome::Full
    } else {
        ReadOutcome::Partial
    };

    Ok(outcome)
}
