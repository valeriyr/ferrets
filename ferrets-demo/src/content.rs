//! Demo content: two races (human, orc) plus neutral resource sources, authored
//! in Lua and loaded at startup.
//!
//! Times are in ticks (20 Hz), tuned short so mechanics are quick to test.

use bevy::prelude::*;
use ferrets_script::{content, engine::lua::LuaEngine};
use ferrets_simulation::content::registry::ContentRegistry;

/// The demo's content, as a Lua script. It declares the ground and water
/// navigation layers (named by [`crate::map::GROUND`] and
/// [`crate::map::WATER`]) and a terrain for each. Fractional stats are decimal
/// strings so they parse straight to fixed-point (no `f64`).
pub const CONTENT: &str = r#"
    local GROUND = define_layer("ground")
    local WATER = define_layer("water")

    define_terrain("grass", GROUND)
    define_terrain("water", WATER)

    define_race("human")
    define_race("orc")

    define_resource("gold")
    define_resource("wood")

    -- Marks the living, which is what a medic will treat and a worker will not.
    -- "building" is pre-registered by the engine.
    define_tag("biological")

    -- Projectile kinds. Each is registered by name so the renderer can draw an
    -- arrow differently from a cannonball, and so several weapons can share one.
    -- An arrow and a cannonball follow what they were fired at; a mortar shell is
    -- sent to a cell, so a target that keeps moving escapes the burst.
    define_projectile("arrow", { speed = "1.0", aim = "entity" })
    define_projectile("cannonball", { speed = "0.5", aim = "entity" })
    define_projectile("shell", { speed = "0.2", aim = "position" })

    -- The archer's self-buff: a burst of speed and damage that reverts on expiry.
    -- Five seconds at 20 Hz, long enough to watch it work and then wear off.
    define_buff("frenzy", {
        duration = 100,
        stack = "refresh",
        modifiers = {
            { stat = "speed", op = "percent", value = "1.0" },
            { stat = "damage", op = "percent", value = "0.5" },
        },
    })

    -- The archer's activated abilities, cast from the command card. Both are
    -- self-targeted, so the archer carries two skill buttons.
    define_skill("battle_focus", {
        cooldown = 80,
        energy_cost = "30",
        target = "self",
        effect = { apply_buff = "frenzy" },
    })
    define_skill("second_wind", {
        cooldown = 120,
        energy_cost = "20",
        target = "self",
        effect = { heal = "15" },
    })

    -- The lake boss: a raceless water fortress spawning free ships. Ships are
    -- ranged so they shell shore targets; the fortress is the boss's building.
    define_entity("ship", {
        location = { occupation = WATER, size = 1, solidity = "solid" },
        stats = {
            speed = "0.25", max_health = 80,
            damage = 12, attack_range = 5, acquire_range = 8, attack_period = 10, damage_point = 4,
            -- Sees past its acquire range so its circular vision covers the square it
            -- can auto-engage.
            sight_range = 12,
        },
        dying = { time = 2 },
        -- Shore bombardment: a slow ball, so shots at a moving target are wasted.
        projectile = "cannonball",
        train_time = 100,
    })
    define_entity("sea_fortress", {
        location = { occupation = WATER, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 1500, sight_range = 8 },
        dying = { time = 2 },
        trainer = { "ship" },
        tags = { "building" },
    })

    -- Neutral resource sources.
    define_entity("gold_mine", {
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        resource_source = { kind = "gold", depletion = "persist" },
    })
    define_entity("tree", {
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        resource_source = { kind = "wood", depletion = "destroy" },
    })

    -- The two races differ only in how their workers attend a job, not in what they
    -- charge or how fast they work, so the presences can be compared side by side:
    -- `presence` names the one to use for building, mending and chopping.
    local function worker(name, race, builds, presence)
        define_entity(name, {
            race = race,
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = {
                speed = "0.3", max_health = 30, sight_range = 4,
                -- Mends at the rate it builds, and bills a quarter of the price for
                -- a full restore, so repairing is cheaper than rebuilding. It works
                -- from the next cell over.
                repair_speed = "1.0", repair_cost_factor = "0.25", repair_range = 1,
                -- Raises a site from the next cell over, and works a seam or a
                -- stand of trees from the same distance.
                build_range = 1, harvest_range = 1,
            },
            dying = { time = 2 },
            cost = { gold = 50 },
            train_time = 40,
            builder = { builds = builds, presence = presence.build },
            -- Workers mend structures at the pace the structure took to raise, and
            -- each pays its own share of the bill.
            repairer = {
                repairs = { "building" },
                rate = { mode = "production" },
                presence = presence.repair,
                cost = { mode = "pro_rata" },
                -- Broke for ten seconds and the job is abandoned.
                patience = 200,
            },
            tags = { "biological" },
            -- A mine shaft holds one worker whoever sinks it; chopping happens in the
            -- open, and how many axes one stand takes is the race's own business.
            resource_carrier = {
                gold = { capacity = 5, time = 20, presence = "hidden" },
                wood = { capacity = 5, time = 20, presence = presence.wood },
            },
        })
    end

    local function main_hall(name, race, trains)
        define_entity(name, {
            race = race,
            location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
            stats = { max_health = 800, sight_range = 7 },
            dying = { time = 2 },
            cost = { gold = 400 },
            build_time = 200,
            trainer = { trains },
            resource_storage = { "gold", "wood" },
            tags = { "building" },
        })
    end

    local function barracks(name, race, trains)
        define_entity(name, {
            race = race,
            location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
            stats = { max_health = 500, sight_range = 6 },
            dying = { time = 2 },
            cost = { gold = 200, wood = 100 },
            build_time = 120,
            -- Mends in half the time it took to raise: a barracks is quicker to
            -- patch up than to put up.
            repair_ratio = "0.5",
            trainer = trains,
            tags = { "building" },
        })
    end

    -- Human: worker, base, barracks, and a ranged unit.
    -- Peasants work in the open and swarm: any number of them can share a site, a
    -- repair or a stand of trees, each adding its own tick of work, so a gang of
    -- them raises a building in a fraction of the time one would take.
    worker("peasant", "human", { "town_hall", "barracks" }, {
        build = "present_stacking", repair = "present_stacking", wood = "present_stacking",
    })
    main_hall("town_hall", "human", "peasant")
    barracks("barracks", "human", { "archer", "mortar", "medic" })
    define_entity("archer", {
        race = "human",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", max_health = 40,
            damage = 6, attack_range = 4, acquire_range = 7, attack_period = 7, damage_point = 3,
            -- 0.1/tick is 2 energy a second, so a 30-cost cast is earned over ~15s
            -- rather than handed back instantly: energy gates the skills, not the
            -- cooldowns.
            max_energy = 60, energy_regen = "0.1",
            -- Sees comfortably past its acquire range, so its circular vision covers
            -- what it can auto-engage.
            sight_range = 10,
        },
        dying = { time = 2 },
        tags = { "biological" },
        -- Anti-armor arrows: extra damage against the (armored) grunt.
        bonus_damage_vs = { grunt = 4 },
        -- A fast arrow: visibly in flight at range 4, but rarely wasted.
        projectile = "arrow",
        -- An energy pool (above) feeds two activated skills: a self-buff burst of
        -- speed and damage that reverts on expiry, and a small self-heal.
        skills = { "battle_focus", "second_wind" },
        cost = { gold = 80 },
        train_time = 60,
        -- Combat units lead a mixed selection over workers.
        selection = { priority = 10 },
    })

    -- Human support: a medic that restores the living at a flat rate, paying out of
    -- its own energy rather than the treasury. Nothing it does touches a building,
    -- and only one medic may work a patient at a time.
    define_entity("medic", {
        race = "human",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", max_health = 45, sight_range = 9,
            -- Half a point of energy per point of health (see the repairer cost
            -- below) means a full 200-point pool restores 400 health across a squad.
            max_energy = 200, energy_regen = "0.2",
            -- A flat point of health per tick, whatever the patient is — a unit's
            -- price says nothing about how long it takes to patch up.
            repair_speed = "1.0", repair_range = 2,
        },
        dying = { time = 2 },
        tags = { "biological" },
        repairer = {
            repairs = { "biological" },
            rate = { mode = "per_tick", health = "1.0" },
            -- Stays on the map beside its patient, and works alone.
            presence = "present",
            cost = { mode = "energy", per_health = "0.5" },
            -- Never gives up: out of energy it waits at the patient and resumes as
            -- the pool refills.
            patience = nil,
        },
        cost = { gold = 100 },
        train_time = 70,
        selection = { priority = 8 },
    })

    -- Human siege: a mortar whose shell travels and bursts, so its damage lands
    -- where the shot was aimed rather than on whatever it was tracking.
    define_entity("mortar", {
        race = "human",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.2", max_health = 35,
            damage = 14, attack_range = 7, acquire_range = 9, attack_period = 20, damage_point = 8,
            sight_range = 11,
        },
        dying = { time = 2 },
        tags = { "biological" },
        -- The shell crosses one cell every five ticks, so a target that keeps moving
        -- takes the direct hit while the burst lands behind it.
        projectile = "shell",
        splash = {
            shape = "circular",
            bands = { {1, "0.5"}, {2, "0.25"} },
            layers = GROUND,
            friendly_fire = true,
        },
        cost = { gold = 120, wood = 40 },
        train_time = 90,
        selection = { priority = 10 },
    })

    -- Orc: worker, base, barracks, and a melee unit.
    -- Peons work one to a job and disappear into what they raise: a site swallows its
    -- peon until the walls are up, where a repair or a stand only ties one up in the
    -- open. Nothing they do goes faster for a second pair of hands.
    worker("peon", "orc", { "great_hall", "orc_barracks" }, {
        build = "hidden", repair = "present", wood = "present",
    })
    main_hall("great_hall", "orc", "peon")
    barracks("orc_barracks", "orc", { "grunt" })
    define_entity("grunt", {
        race = "orc",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", max_health = 60,
            damage = 10, attack_range = 1, acquire_range = 5, attack_period = 6, damage_point = 3,
            -- Heavy melee: flat armor blunts each incoming hit.
            armor = 3,
            -- 0.05/tick is a point a second, so a mauled grunt walks off its wounds
            -- over about a minute instead of needing to be replaced.
            health_regen = "0.05",
            sight_range = 8,
        },
        dying = { time = 2 },
        tags = { "biological" },
        cost = { gold = 90 },
        train_time = 70,
        selection = { priority = 10 },
    })
"#;

/// Loads all demo content from Lua into the registry, then validates it. Runs at
/// startup; a content error is a bug in the script above, so it panics.
pub fn register_all(mut registry: ResMut<ContentRegistry>) {
    *registry = content::load(&LuaEngine, CONTENT).expect("demo content must load");
}
