//! Demo content: two races (human, orc) plus neutral resource sources, authored
//! in Lua and loaded at startup.
//!
//! Times are in ticks (20 Hz), tuned short so mechanics are quick to test.

use bevy::prelude::*;
use ferrets_content::registry::ContentRegistry;
use ferrets_script::{content, engine::lua::LuaEngine};

/// The demo's content, as a Lua script. It declares the ground, water and air
/// navigation layers (named by [`crate::map::GROUND`], [`crate::map::WATER`]
/// and [`crate::map::AIR`]) and a terrain for each surface. Fractional stats
/// are decimal strings so they parse straight to fixed-point (no `f64`).
pub const CONTENT: &str = r#"
    local GROUND = define_layer("ground")
    local WATER = define_layer("water")
    local AIR = define_layer("air")

    -- Every surface is flyable, so the air layer is open where the ground and
    -- water layers are not: a flier crosses the lake and passes over whatever
    -- stands on the shore, because nothing on those layers claims this one.
    define_terrain("grass", GROUND | AIR)
    define_terrain("water", WATER | AIR)

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
    define_entity_buff("frenzy", {
        duration = 100,
        stack = "refresh",
        modifiers = {
            { entity_stat = "speed", op = "percent", value = "1.0" },
            { entity_stat = "damage", op = "percent", value = "0.5" },
        },
    })

    -- Activated abilities, cast from the command card. Battle focus is the
    -- archer's self-buff; blood rite is the grunt's — the same frenzy at a
    -- different price, health and gold instead of energy, so the cost arms can
    -- be compared side by side. Second wind is the shaman's targeted mend: it
    -- takes an allied unit, so its button arms a target click.
    define_skill("battle_focus", {
        caster = "entity",
        cooldown = 80,
        cost = { energy = "30" },
        target = "caster",
        effect = { apply_buff = "frenzy" },
    })
    define_skill("second_wind", {
        caster = "entity",
        cooldown = 120,
        cost = { energy = "20" },
        target = "ally",
        effect = { heal = "15" },
    })
    -- Blood rite unlocks with the frenzy ritual (defined below): the button
    -- sits greyed on every grunt until the war camp finishes the research.
    define_skill("blood_rite", {
        caster = "entity",
        cooldown = 160,
        cost = { health = "8", resources = { gold = 10 } },
        target = "caster",
        effect = { apply_buff = "frenzy" },
        requires = { "frenzy_ritual" },
    })

    -- A player-level rallying call: every unit the caster owns moves half again
    -- as fast for five seconds, paid from the stockpile and cooled down per
    -- player. Cast from its own HUD button. The skill is just the trigger; the
    -- effect is an ordinary buff, held by the player instead of any unit.
    define_player_buff("war_drums", {
        duration = 100,
        stack = "refresh",
        entity_modifiers = {
            { entity_stat = "speed", op = "percent", value = "0.5" },
        },
    })
    define_skill("war_drums", {
        caster = "player",
        cooldown = 300,
        cost = { resources = { gold = 50 } },
        effect = { apply_buff = "war_drums" },
    })

    -- Upgrades: a research that completes applies a permanent player buff, so
    -- every unit the player owns — standing or yet to be trained — carries it
    -- through the ordinary recompute. Iron weapons is the human weapon upgrade,
    -- researched at the blacksmith; the frenzy ritual quickens every orc
    -- attack, researched at the war camp once a pig farm stands.
    define_player_buff("iron_weapons", {
        stack = "ignore",
        entity_modifiers = {
            { entity_stat = "damage", op = "flat", value = "2" },
        },
    })
    define_research("iron_weapons", {
        cost = { gold = 100, wood = 50 },
        time = 200,
        buff = "iron_weapons",
    })
    define_player_buff("frenzy_ritual", {
        stack = "ignore",
        entity_modifiers = {
            { entity_stat = "attack_period", op = "percent", value = "-0.25" },
        },
    })
    define_research("frenzy_ritual", {
        cost = { gold = 150 },
        time = 240,
        buff = "frenzy_ritual",
        requires = { "pig_farm" },
    })

    -- The lake boss: a raceless water fortress spawning free ships. Ships are
    -- ranged so they shell shore targets; the fortress is the boss's building.
    define_entity("ship", {
        location = { occupation = WATER, size = 1, solidity = "solid" },
        stats = {
            speed = "0.25", radius = "0.5", max_health = 80,
            damage = 12, attack_range = 5, acquire_range = 8, attack_period = 10, damage_point = 4,
            -- Sees past its acquire range so its circular vision covers the square it
            -- can auto-engage.
            sight_range = 12,
            supply_cost = 1,
        },
        dying = { time = 2 },
        -- Shore bombardment: a slow ball, so shots at a moving target are wasted.
        projectile = "cannonball",
        targets = GROUND | WATER | AIR,
        train_time = 100,
    })
    -- The fortress is tall enough to be in the way of what flies: it holds the
    -- water layer under it and the air layer over it at once, so fliers must go
    -- around a keep that ships must also go around. It is the only thing on the
    -- map that closes the air, which is what gives the air layer any shape at
    -- all. Occupying the air also makes it a legal target for anti-air, since
    -- targetability follows occupation unless a type says otherwise.
    define_entity("sea_fortress", {
        location = { occupation = WATER | AIR, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 1500, sight_range = 8, supply_provided = 5 },
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
                speed = "0.3", radius = "0.5", max_health = 30, sight_range = 4,
                -- Mends at the rate it builds, and bills a quarter of the price for
                -- a full restore, so repairing is cheaper than rebuilding. It works
                -- from the next cell over.
                repair_speed = "1.0", repair_cost_factor = "0.25", repair_range = 1,
                -- Raises a site from the next cell over, and works a seam or a
                -- stand of trees from the same distance.
                build_range = 1, harvest_range = 1,
                supply_cost = 1,
                -- One shelter slot: a worker fits in a bunker or a pig farm.
                cargo_size = 1,
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
            -- Enough starting headroom for the first few units; farms carry the
            -- army beyond it. Sight reaches the mine placed by the base, so
            -- the economy never depends on a lucky scout.
            stats = { max_health = 800, sight_range = 9, supply_provided = 10 },
            dying = { time = 2 },
            cost = { gold = 400 },
            build_time = 200,
            trainer = { trains },
            resource_storage = { "gold", "wood" },
            tags = { "building" },
        })
    end

    -- Farms feed the army: each adds headroom for a handful of units, and losing
    -- one blocks new training until the headroom recovers.
    local function farm(name, race)
        define_entity(name, {
            race = race,
            location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
            stats = { max_health = 200, sight_range = 3, supply_provided = 6 },
            dying = { time = 2 },
            cost = { gold = 40, wood = 20 },
            build_time = 60,
            tags = { "building" },
        })
    end

    local function barracks(name, race, trains, researches)
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
            researcher = researches,
            tags = { "building" },
        })
    end

    -- Human: worker, base, barracks, and a ranged unit.
    -- Peasants work in the open and swarm: any number of them can share a site, a
    -- repair or a stand of trees, each adding its own tick of work, so a gang of
    -- them raises a building in a fraction of the time one would take.
    worker("peasant", "human", { "town_hall", "barracks", "farm", "blacksmith", "bunker" }, {
        build = "present_stacking", repair = "present_stacking", wood = "present_stacking",
    })
    main_hall("town_hall", "human", "peasant")
    farm("farm", "human")
    barracks("barracks", "human", { "archer", "mortar", "medic", "gryphon" })

    -- The human garrison: the living step inside and the armed among them fire
    -- their own weapons out, untouchable until the walls come down — and when
    -- they do, whoever fits through the ruins walks away.
    define_entity("bunker", {
        race = "human",
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        stats = {
            max_health = 400, sight_range = 7,
            cargo_capacity = 4,
            -- Boarding steps over the threshold; unloading spills everyone out
            -- at once, so a garrison empties the moment it is told to.
            load_range = 1, unload_range = 1, load_period = 0, unload_period = 0,
        },
        dying = { time = 2 },
        -- Stone and earthworks: no call on the wood line, which the demo
        -- economy keeps stretched over the upgrades.
        cost = { gold = 100 },
        build_time = 80,
        transporter = {
            carries = { "biological" },
            boarding = "own",
            fate = "eject",
            conduct = "fight",
        },
        tags = { "building" },
    })

    -- The human tech building: while one stands, mortars unlock, and it hosts
    -- the iron weapons upgrade.
    define_entity("blacksmith", {
        race = "human",
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        stats = { max_health = 350, sight_range = 5 },
        dying = { time = 2 },
        cost = { gold = 150, wood = 80 },
        build_time = 100,
        researcher = { "iron_weapons" },
        tags = { "building" },
    })
    define_entity("archer", {
        race = "human",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", radius = "0.5", max_health = 40,
            damage = 6, attack_range = 4, acquire_range = 7, attack_period = 7, damage_point = 3,
            -- 0.1/tick is 2 energy a second, so a 30-cost cast is earned over ~15s
            -- rather than handed back instantly: energy gates the skills, not the
            -- cooldowns.
            max_energy = 60, energy_regen = "0.1",
            -- Sees comfortably past its acquire range, so its circular vision covers
            -- what it can auto-engage.
            sight_range = 10,
            supply_cost = 1,
            cargo_size = 1,
        },
        dying = { time = 2 },
        tags = { "biological" },
        -- Anti-armor arrows: extra damage against the (armored) grunt.
        bonus_damage_vs = { grunt = 4 },
        -- A fast arrow: visibly in flight at range 4, but rarely wasted.
        projectile = "arrow",
        -- The human answer to everything that moves, whatever layer it moves on.
        targets = GROUND | WATER | AIR,
        -- An energy pool (above) feeds the self-buff burst of speed and damage
        -- that reverts on expiry.
        skills = { "battle_focus" },
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
            speed = "0.3", radius = "0.5", max_health = 45, sight_range = 9,
            -- Half a point of energy per point of health (see the repairer cost
            -- below) means a full 200-point pool restores 400 health across a squad.
            max_energy = 200, energy_regen = "0.2",
            -- A flat point of health per tick, whatever the patient is — a unit's
            -- price says nothing about how long it takes to patch up.
            repair_speed = "1.0", repair_range = 2,
            supply_cost = 1,
            cargo_size = 1,
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
            speed = "0.2", radius = "0.5", max_health = 35,
            damage = 14, attack_range = 7, acquire_range = 9, attack_period = 20, damage_point = 8,
            sight_range = 11,
            supply_cost = 1,
            -- The tube and its crew take two shelter slots.
            cargo_size = 2,
        },
        dying = { time = 2 },
        tags = { "biological" },
        -- The shell crosses one cell every five ticks, so a target that keeps moving
        -- takes the direct hit while the burst lands behind it.
        projectile = "shell",
        -- Siege fires at the ground, so it must not take aim at what flies: the
        -- blast below already spares fliers, and a mortar allowed to *target* one
        -- would walk into range and drop shells that could never damage it.
        targets = GROUND | WATER,
        splash = {
            shape = "circular",
            bands = { {1, "0.5"}, {2, "0.25"} },
            layers = GROUND,
            friendly_fire = true,
        },
        cost = { gold = 120, wood = 40 },
        train_time = 90,
        selection = { priority = 10 },
        -- Siege needs the forge: no mortars until a blacksmith stands.
        requires = { "blacksmith" },
    })

    -- The human gryphon: one unit in two forms, and the demo's only thing that
    -- changes what it *is* while it lives. Grounded it walks and blocks ground;
    -- aloft it flies over everything. Both forms are 2x2 and carry one archer who
    -- shoots from inside, so the beast has no weapon of its own and the passenger
    -- is its only answer to anything.
    --
    -- The take-off window, as content's own stat: the engine has no built-in
    -- notion of a morph-time stat — a transition's `time` may name any declared
    -- stat, and this is the one the gryphon's take-off reads. A stat rather
    -- than a plain tick count so a buff or research could quicken it.
    define_entity_stat("morph_time")

    -- Each form names the other, which is why the pair is authored as two types:
    -- everything that differs between them — layer, speed, what reaches them — is
    -- a type property, and the change is just an edge between the two. Only the
    -- grounded form is trainable; the aloft one exists solely as the other end
    -- of the change.
    local function gryphon(name, occupation, targetable, speed, fate, morphs, trainable)
        define_entity(name, {
            race = "human",
            location = { occupation = occupation, size = { 2, 2 }, solidity = "solid" },
            targetable = targetable,
            morphs = morphs,
            stats = {
                speed = speed, radius = "1", max_health = 180, sight_range = 10,
                supply_cost = 2,
                -- One rider, who fights from the saddle.
                cargo_capacity = 1,
                load_range = 1, unload_range = 1, load_period = 0, unload_period = 0,
                -- A second to change form, during which it can do nothing else.
                -- The window is the whole cost besides the wing-beat energy:
                -- there is no cooldown, because the commitment is what makes
                -- taking off a decision. A stat so a research could quicken it
                -- — carried by the grounded form alone, since only the
                -- take-off reads it and a buffed dead stat on the aloft form
                -- would only mislead.
                morph_time = trainable and 20 or nil,
                -- The pool the take-off draws from; shared by both forms so it
                -- carries across the change.
                max_energy = 60, energy_regen = "0.2",
            },
            dying = { time = 2 },
            transporter = {
                carries = { "archer" },
                boarding = "own",
                fate = fate,
                conduct = "fight",
            },
            cost = trainable and { gold = 200, wood = 60 } or nil,
            train_time = trainable and 110 or nil,
            selection = { priority = 10 },
        })
    end
    -- Grounded, the beast stands tall enough to be shot out of the air — the
    -- case occupation alone cannot express, since holding the air layer would
    -- also wall the sky off. Aloft it is answerable where it lives and nowhere
    -- else: taking off is exactly what shakes an axe, which is the whole reason
    -- to climb.
    --
    -- The two edges wear different terms on purpose. Taking off checks nothing
    -- early — the sky is rarely contested — and costs a beat of energy; landing
    -- is free but *reserves* its ground the moment it is ordered, so the spot
    -- underneath cannot be built over or wandered onto while the beast descends.
    -- Both are committed: mid-change there is no changing back.
    --
    -- The rider's fate follows the altitude: a beast cut down on the ground
    -- spills its archer alive beside the wreck, but one shot out of the sky
    -- takes saddle and rider down together — which is the risk that prices
    -- the ride.
    gryphon("gryphon", GROUND, GROUND | AIR, "0.3", "eject", {
        { into = "gryphon_aloft",
          time = { stat = "morph_time" },
          placement = "revalidate",
          cancel = "committed",
          cost = { energy = "20" } },
    }, true)
    gryphon("gryphon_aloft", AIR, AIR, "0.45", "destroy", {
        -- A plain tick count, where the take-off reads its stat: landing pace
        -- is nothing anyone would research.
        { into = "gryphon",
          time = 20,
          placement = "reserve",
          cancel = "committed" },
    }, false)

    -- The orc air transport: a 2x2 flier, and the demo's first mover wider than
    -- one cell. Its footprint is what the planner has to fit, so it only routes
    -- through two-wide gaps — and its body is the circle inscribed in that
    -- footprint, radius one, which is the widest a 2x2 may carry.
    define_entity("zeppelin", {
        race = "orc",
        location = { occupation = AIR, size = { 2, 2 }, solidity = "solid" },
        stats = {
            speed = "0.35", radius = "1", max_health = 150, sight_range = 10,
            supply_cost = 2,
            cargo_capacity = 4,
            -- A gangplank: one body a second each way, as the pig farm's is.
            load_range = 1, unload_range = 1,
            load_period = 20, unload_period = 20,
        },
        dying = { time = 2 },
        cost = { gold = 160, wood = 60 },
        train_time = 90,
        -- Carries the workforce and the army alike — and whoever is aboard
        -- when it is shot down goes down with it, the same bargain the
        -- gryphon's rider strikes aloft.
        transporter = {
            carries = { "peon", "grunt", "shaman" },
            boarding = "own",
            fate = "destroy",
            conduct = "shelter",
        },
        selection = { priority = 10 },
    })

    -- Orc: worker, base, barracks, and a melee unit.
    -- Peons work one to a job and disappear into what they raise: a site swallows its
    -- peon until the walls are up, where a repair or a stand only ties one up in the
    -- open. Nothing they do goes faster for a second pair of hands.
    worker("peon", "orc", { "great_hall", "war_camp", "pig_farm", "watch_tower" }, {
        build = "hidden", repair = "present", wood = "present",
    })
    main_hall("great_hall", "orc", "peon")

    -- The orc farm is also a shelter, for the workforce alone: peons crawl in
    -- one at a time and sit out a raid unseen — the army stands and fights.
    -- Nobody fights from a pig sty, and whoever is still inside when it burns
    -- burns with it. (Named by type, not tag: the one place the demo admits
    -- by exact type name.)
    define_entity("pig_farm", {
        race = "orc",
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        stats = {
            max_health = 200, sight_range = 3, supply_provided = 6,
            cargo_capacity = 4,
            -- A crawl space, not a door: one body a second each way.
            load_range = 1, unload_range = 1,
            load_period = 20, unload_period = 20,
        },
        dying = { time = 2 },
        cost = { gold = 40, wood = 20 },
        build_time = 60,
        transporter = {
            carries = { "peon" },
            boarding = "own",
            fate = "destroy",
            conduct = "shelter",
        },
        tags = { "building" },
    })
    -- The orc watch tower: what it can be hit by and where it stands are
    -- different answers, like the grounded gryphon. It is rooted on the ground
    -- and blocks only the ground, so fliers pass over it freely — but it stands
    -- tall enough to be shot out of the air, which `targetable` says and
    -- `occupation` could not. Occupying the air instead would make it a wall
    -- across the sky, which is the fortress's job, not a tower's.
    --
    -- The watch tower only watches: the farthest eyes on the orc side and no
    -- weapon. The upgrade trades some of that watch for bolts — the guard
    -- tower below sees less and is the orc answer to fliers.
    define_entity("watch_tower", {
        race = "orc",
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        targetable = GROUND | AIR,
        stats = {
            max_health = 250,
            sight_range = 12,
        },
        dying = { time = 2 },
        cost = { gold = 120, wood = 40 },
        build_time = 70,
        tags = { "building" },
        -- The demo's building upgrade: a paid, refundable change in place. The
        -- money is committed up front and comes back in full if the upgrade is
        -- called off — which is what makes starting one cheap to reconsider.
        morphs = {
            { into = "guard_tower",
              time = 60,
              placement = "reserve",
              cancel = "refundable",
              cost = { resources = { gold = 80, wood = 20 } } },
        },
    })
    -- What the watch tower upgrades into: the same tower, now armed, its bolts
    -- reaching every layer. Never built directly — the only way here is
    -- through the change above.
    define_entity("guard_tower", {
        race = "orc",
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        targetable = GROUND | AIR,
        stats = {
            max_health = 350,
            damage = 14, attack_range = 7, acquire_range = 9, attack_period = 12, damage_point = 5,
            sight_range = 10,
        },
        dying = { time = 2 },
        projectile = "arrow",
        targets = GROUND | WATER | AIR,
        tags = { "building" },
    })
    barracks("war_camp", "orc", { "grunt", "shaman", "zeppelin" }, { "frenzy_ritual" })
    define_entity("grunt", {
        race = "orc",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", radius = "0.5", max_health = 60,
            damage = 10, attack_range = 1, acquire_range = 5, attack_period = 6, damage_point = 3,
            -- Heavy melee: flat armor blunts each incoming hit.
            armor = 3,
            -- 0.05/tick is a point a second, so a mauled grunt walks off its wounds
            -- over about a minute instead of needing to be replaced.
            health_regen = "0.05",
            sight_range = 8,
            supply_cost = 1,
            cargo_size = 1,
        },
        dying = { time = 2 },
        tags = { "biological" },
        -- An axe reaches what stands on the ground or floats on the water and
        -- nothing that flies.
        targets = GROUND | WATER,
        -- Blood rite: the grunt buys the archer's frenzy with its own blood and
        -- a little gold — regeneration (above) walks the price off afterwards.
        skills = { "blood_rite" },
        cost = { gold = 90 },
        train_time = 70,
        selection = { priority = 10 },
    })

    -- Orc support: a shaman that mends one allied unit at a time from its energy
    -- pool. The heal takes a target, so casting is button-then-click.
    define_entity("shaman", {
        race = "orc",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", radius = "0.5", max_health = 35,
            max_energy = 80, energy_regen = "0.2",
            sight_range = 8,
            supply_cost = 1,
            cargo_size = 1,
        },
        dying = { time = 2 },
        tags = { "biological" },
        skills = { "second_wind" },
        cost = { gold = 120 },
        train_time = 80,
        -- Support trails combat units in a mixed selection, like the medic.
        selection = { priority = 5 },
        -- A completed research as a requirement: shamans answer the ritual.
        requires = { "frenzy_ritual" },
    })
"#;

/// Loads all demo content from Lua into the registry, then validates it. Runs at
/// startup; a content error is a bug in the script above, so it panics.
pub fn register_all(mut registry: ResMut<ContentRegistry>) {
    *registry = content::load(&LuaEngine, CONTENT).expect("demo content must load");
}
