use bevy::prelude::*;
use ferrets_simulation::map::Map;

pub fn refresh_nav_hierarchy(mut map: ResMut<Map>) {
    map.refresh_hierarchy();
}
