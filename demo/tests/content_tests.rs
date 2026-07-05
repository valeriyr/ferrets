//! The demo's embedded content script: it loads and validates.

use demo::content::CONTENT;
use ferrets_script::{content, engine::lua::LuaEngine};

#[test]
fn content_loads_and_validates() {
    let registry = content::load(&LuaEngine, CONTENT).expect("demo content loads");

    for name in [
        "gold_mine",
        "tree",
        "peasant",
        "town_hall",
        "barracks",
        "archer",
        "peon",
        "great_hall",
        "orc_barracks",
        "grunt",
    ] {
        assert!(registry.entity(name).is_some(), "missing entity '{name}'");
    }
    assert!(registry.has_race("human") && registry.has_race("orc"));
}
