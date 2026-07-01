//! A loaded replay: its header and per-tick recorded input.

use std::collections::BTreeMap;
use std::io::Read;

use ferrets_simulation::command::PlayerCommand;
use ferrets_simulation::session::player_slot::PlayerId;

use crate::format;
use crate::header::ReplayHeader;
use crate::record::TickRecord;

/// A replay loaded from a stream.
#[derive(Debug)]
pub struct Replay {
    header: ReplayHeader,
    ticks: BTreeMap<u32, TickRecord>,
}

impl Replay {
    /// Reads a replay from `reader`, validating the magic prelude and format
    /// version. A truncated trailing record (a recording cut short) is dropped;
    /// every complete record before it is kept.
    pub fn read(mut reader: impl Read) -> crate::Result<Self> {
        let header = format::read_prelude(&mut reader)?;

        let mut ticks = BTreeMap::new();
        while let Some(record) = format::read_record(&mut reader)? {
            ticks.insert(record.tick, record);
        }

        Ok(Self { header, ticks })
    }

    /// The setup the game is rebuilt from.
    pub fn header(&self) -> &ReplayHeader {
        &self.header
    }

    /// The last recorded tick, or `None` if nothing was recorded.
    pub fn last_tick(&self) -> Option<u32> {
        self.ticks.keys().next_back().copied()
    }

    /// The per-player commands recorded for `tick`; empty if the tick was not
    /// recorded or every player was idle.
    pub fn inputs_at(&self, tick: u32) -> &[(PlayerId, Vec<PlayerCommand>)] {
        self.ticks.get(&tick).map_or(&[], |record| &record.inputs)
    }

    /// The state checksum recorded for `tick`, if one was sampled there.
    pub fn checksum_at(&self, tick: u32) -> Option<u64> {
        self.ticks.get(&tick).and_then(|record| record.checksum)
    }
}
