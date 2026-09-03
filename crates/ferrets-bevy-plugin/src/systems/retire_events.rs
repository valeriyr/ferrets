use bevy::prelude::*;
use ferrets_simulation::events::EventRecord;

/// Drops the completed tick's announcements, after every consumer of the tick
/// has run.
pub fn retire_events(mut record: ResMut<EventRecord>) {
    record.clear();
}
