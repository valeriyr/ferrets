//! Demo content: four races (human, orc, swarm, conclave) plus neutral resource
//! sources, authored in Lua and loaded at startup.
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
    -- Two more races carry the fields: the swarm builds on creep it spreads,
    -- the conclave builds in the power its pylons project.
    define_race("swarm")
    define_race("conclave")

    -- Creep covers the ground and recedes ring by ring, half a second a ring,
    -- once nothing sustains it, and whoever spreads it sees every cell of it;
    -- power is there while its pylon stands and gone the tick it falls.
    define_field("creep", { layer = GROUND, decay = { cycle = 10 }, vision = "watched" })
    define_field("power", { layer = GROUND, decay = "instant" })

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
            speed = "0.25", turn_rate = 6, pivot_rate = 9, radius = "0.5", weight = 6, max_health = 80,
            damage = 12, attack_range = 5, acquire_range = 8, attack_period = 10, damage_point = 4,
            -- Sees past its acquire range so its circular vision covers the square it
            -- can auto-engage.
            sight_range = 12,
            supply_cost = 1,
        },
        dying = { time = 2 },
        -- Shore bombardment: a slow ball, so shots at a moving target are wasted.
        attack = { targets = GROUND | WATER | AIR, projectile = "cannonball" },
        train_time = 100,
    })
    -- The fortress is tall enough to be in the way of what flies: it holds the
    -- water layer under it and the air layer over it at once, so fliers must go
    -- around a keep that ships must also go around. It is the only thing on the
    -- map that closes the air, which is what gives the air layer any shape at
    -- all. Occupying the air also makes it a legal target for anti-air, since
    -- targetability follows occupation unless a type says otherwise.
    -- Four guns, one at each corner, which is what a building has instead of
    -- turning: the keep stands square to the map wherever it was put and only the
    -- guns come about — at sixty degrees a second, so bringing one onto something
    -- behind it takes three seconds, and the corner already facing a
    -- threat is the one that answers it. Their reach clears the lake it sits in
    -- (nine cells of water from the middle) and a couple of cells of shore beyond,
    -- so approaching the boss by land is answered rather than merely watched; it
    -- sees further still, since a gun that acquires what it cannot see is a gun
    -- waiting for a target to walk into it. Each fires through sixty degrees,
    -- thirty either side of where it points, so a raid on one side is worked by
    -- the two guns that bear and ignored by the two that do not. The shell is slow
    -- and aimed at a place rather than a body, so one laboriously-aimed round is a
    -- round a moving target can walk out from under.
    define_turret("keep_gun", {
        targets = GROUND | WATER | AIR,
        projectile = "shell",
    })
    define_entity("sea_fortress", {
        location = { occupation = WATER | AIR, size = { 5, 5 }, solidity = "solid" },
        stats = {
            max_health = 1500, sight_range = 16, supply_provided = 5,
            damage = 10, attack_range = 12, acquire_range = 14, attack_period = 30,
            damage_point = 12, aim_rate = 3, attack_arc = 60,
        },
        dying = { time = 2 },
        -- One gun at each corner, all reading the same numbers: four times the
        -- fire of the old single mount, so each round is a quarter of what that
        -- one carried. They spread their own targets, which is what four guns on
        -- one keep are for — a raid of four is answered four times over rather
        -- than one of them shot at four times.
        turrets = {
            { turret = "keep_gun", at = { 0, 0 }, size = { 2, 2 } },
            { turret = "keep_gun", at = { 3, 0 }, size = { 2, 2 } },
            { turret = "keep_gun", at = { 0, 3 }, size = { 2, 2 } },
            { turret = "keep_gun", at = { 3, 3 }, size = { 2, 2 } },
        },
        turret_fire = "spread",
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
    -- charge or how fast they work, so the ways can be compared side by side:
    -- `work` names the builder attendance and the presence kept while mending
    -- and while chopping.
    local function worker(name, race, builds, work)
        define_entity(name, {
            race = race,
            location = { occupation = GROUND, size = 1, solidity = "solid" },
            stats = {
                -- The baseline weight everything else is authored against: a
                -- worker is what a crowd is made of, and what gives way in one.
                speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 1, max_health = 30, sight_range = 4,
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
            builder = { builds = builds, attendance = work.attendance },
            -- Workers mend structures at the pace the structure took to raise, and
            -- each pays its own share of the bill.
            repairer = {
                repairs = { "building" },
                rate = { mode = "production" },
                presence = work.repair_presence,
                cost = { mode = "pro_rata" },
                -- Broke for ten seconds and the job is abandoned.
                patience = 200,
            },
            tags = { "biological" },
            -- A mine shaft holds one worker whoever sinks it; chopping happens in the
            -- open, and how many axes one stand takes is the race's own business.
            resource_carrier = {
                gold = { capacity = 5, time = 20, presence = "hidden" },
                wood = { capacity = 5, time = 20, presence = work.wood_presence },
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
        attendance = "present_stacking",
        repair_presence = "present_stacking",
        wood_presence = "present_stacking",
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
            speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 1, max_health = 40,
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
        attack = {
            -- The human answer to everything that moves, whatever layer it moves on.
            targets = GROUND | WATER | AIR,
            -- A fast arrow: visibly in flight at range 4, but rarely wasted.
            projectile = "arrow",
        },
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
            speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 1, max_health = 45, sight_range = 9,
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
            -- Tube, carriage and crew: heavier than the infantry it walks with,
            -- though nothing like the grunt that can shoulder it aside.
            speed = "0.2", turn_rate = 6, pivot_rate = 12, pivot_angle = 90, radius = "0.5", weight = 3, max_health = 35,
            damage = 14, attack_range = 7, acquire_range = 9, attack_period = 20, damage_point = 8,
            sight_range = 11,
            supply_cost = 1,
            -- The tube and its crew take two shelter slots.
            cargo_size = 2,
        },
        dying = { time = 2 },
        tags = { "biological" },
        attack = {
            -- Siege fires at the ground, so it must not take aim at what flies: the
            -- blast below already spares fliers, and a mortar allowed to *target*
            -- one would walk into range and drop shells that could never damage it.
            targets = GROUND | WATER,
            -- The shell crosses one cell every five ticks, so a target that keeps
            -- moving takes the direct hit while the burst lands behind it.
            projectile = "shell",
            splash = {
                shape = "circular",
                bands = { {1, "0.5"}, {2, "0.25"} },
                layers = GROUND,
                friendly_fire = true,
            },
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
                speed = speed, turn_rate = 18, pivot_rate = 24, pivot_angle = 90,
                radius = "1", weight = 4, max_health = 180, sight_range = 10,
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
            -- A gas envelope the size of a building: the heaviest thing that
            -- flies, so a gryphon meeting one aloft is the one that gives way.
            speed = "0.35", turn_rate = 6, pivot_rate = 9, pivot_angle = 90, radius = "1", weight = 8, max_health = 150, sight_range = 10,
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
    worker("peon", "orc", { "great_hall", "war_camp", "pig_farm", "watch_tower", "siege_works" }, {
        attendance = "hidden", repair_presence = "present", wood_presence = "present",
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
        attack = { targets = GROUND | WATER | AIR, projectile = "arrow" },
        tags = { "building" },
    })
    barracks("war_camp", "orc", { "grunt", "shaman", "zeppelin" }, { "frenzy_ritual" })

    -- The orc siege works: the one building that exists to train a single unit,
    -- and gated behind the war camp, so the wagon is a second-thought answer to a
    -- dug-in enemy rather than an opening move. Not a barracks: it trains no
    -- infantry and hosts no research, and its own walls are thinner than one.
    define_entity("siege_works", {
        race = "orc",
        location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 400, sight_range = 5 },
        dying = { time = 2 },
        cost = { gold = 180, wood = 120 },
        build_time = 130,
        repair_ratio = "0.5",
        trainer = { "war_wagon" },
        tags = { "building" },
        requires = { "war_camp" },
    })

    define_turret("siege_cannon", {
        targets = GROUND | WATER,
        projectile = "cannonball",
        conduct = "on_the_move",
    })

    -- The orc war wagon: the demo's turreted mover, and the only unit whose gun
    -- and hull point different ways. It shoots from a mounted cannon rather than
    -- by turning to face what it shoots, so it can hold a heading while its gun
    -- comes round — which is what carrying a turret buys, and what its own
    -- `aim_rate` paces. The hull is ponderous by comparison: it comes about more
    -- slowly than the gun and plants its wheels for anything past a right
    -- angle (`pivot_angle`), so a wagon told to reverse stands still for half a
    -- second before it rolls, while the gun it carries is already round.
    --
    -- A flat gun: it answers what stands on the ground or floats, and nothing
    -- that flies — the guard tower and the zeppelin are the orc answer to air.
    -- The arc is narrow, so a target it is not yet bearing on is a target it
    -- holds its fire at, and a wagon caught square is a wagon that cannot answer
    -- for a moment.
    define_entity("war_wagon", {
        race = "orc",
        -- Two cells on a side, like the gryphon: the widest body a two-wide gap
        -- lets through, and a footprint that has to round a corner rather than
        -- slip past it.
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        stats = {
            -- Slow, and heavy enough that a grunt walking into one gives way.
            -- Its body is the circle inscribed in that footprint, radius one.
            speed = "0.2", turn_rate = 9, pivot_rate = 18, pivot_angle = 90,
            radius = "1", weight = 6, max_health = 90,
            -- One heavy shot on a long cycle: it out-ranges a grunt several times
            -- over and loses to anything that closes while it is reloading.
            damage = 24, attack_range = 6, acquire_range = 8, attack_period = 24,
            damage_point = 10,
            -- The gun comes round a third faster than the rolling hull — half a
            -- turn in fifteen ticks against the hull's twenty — through a
            -- forty-five degree arc.
            aim_rate = 12, attack_arc = 45,
            armor = 2,
            sight_range = 9,
            supply_cost = 2,
        },
        dying = { time = 2 },
        -- The one gun in the demo that does not stop to shoot: it bears on its
        -- own, so the hull can keep to its orders while the cannon works whatever
        -- the wagon rolls past. Mounted amidships, which is where its shells
        -- leave from.
        turrets = {
            { turret = "siege_cannon", at = { 0, 0 }, size = { 2, 2 } },
        },
        cost = { gold = 160, wood = 60 },
        train_time = 110,
        -- Siege leads a mixed selection, like the mortar it answers.
        selection = { priority = 10 },
    })
    define_entity("grunt", {
        race = "orc",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            -- Heavy melee in body as well as armor: four times a worker's
            -- weight on the same one-cell footprint, so a peon walking into a
            -- standing grunt is the one that gives way and slides around it.
            speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 4, max_health = 60,
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
        attack = { targets = GROUND | WATER },
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
            speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 1, max_health = 35,
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

    -- ── The Swarm ──────────────────────────────────────────────────────────
    -- A drone is spent on what it builds: it walks to the site, pays, and
    -- becomes the structure going up, so the swarm has no repairers. Every
    -- structure but the hive must stand on creep; the hive itself spreads it,
    -- at full reach when it is placed by the map, and from three cells a cell
    -- every third of a second when it is built, showing a patch under itself
    -- while still going up. Structures left off creep waste away; swarmlings run a third
    -- faster on anyone's creep.
    local ON_CREEP = { requires = "creep", of = "anyone", coverage = "footprint" }
    local WITHERS_OFF_CREEP = { field = "creep", of = "anyone", outside = {
        modifiers = { { entity_stat = "health_drain", op = "flat", value = "0.2" } },
    } }

    define_entity("hive", {
        race = "swarm",
        location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 800, sight_range = 9, supply_provided = 10 },
        dying = { time = 2 },
        cost = { gold = 400 },
        build_time = 200,
        trainer = { "drone" },
        resource_storage = { "gold", "wood" },
        tags = { "building" },
        field_sources = {
            { field = "creep", radius = 10, growth = { cycle = 6, initial_radius = 3 }, while_constructing = 1 },
        },
    })

    -- Drones spew a patch of creep on any cell they can see, which lets the
    -- swarm plant a tumor away from home. The patch has nothing sustaining it,
    -- so it recedes unless a tumor takes root on it in time.
    define_skill("spew_creep", {
        caster = "entity",
        cooldown = 200,
        target = "position",
        effect = { field = { field = "creep", radius = 2, action = "cover" } },
    })
    define_entity("drone", {
        race = "swarm",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 1, max_health = 30, sight_range = 4,
            build_range = 1, harvest_range = 1,
            supply_cost = 1,
            cargo_size = 1,
        },
        dying = { time = 2 },
        cost = { gold = 50 },
        train_time = 40,
        builder = { builds = { "hive", "tumor", "spawning_pit", "brood_nest" }, attendance = "consumed" },
        tags = { "biological" },
        skills = { "spew_creep" },
        resource_carrier = {
            gold = { capacity = 5, time = 20, presence = "hidden" },
            wood = { capacity = 5, time = 20, presence = "hidden" },
        },
    })

    -- A tumor is cheap, small, and only ever planted on creep; it spreads a
    -- patch of its own so the creep can be walked outward tumor by tumor. It
    -- barely sees past itself: the creep is what watches.
    define_entity("tumor", {
        race = "swarm",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = { max_health = 50, sight_range = 1 },
        dying = { time = 2 },
        cost = { gold = 25 },
        build_time = 40,
        tags = { "building" },
        field_placement = { ON_CREEP },
        field_sources = {
            { field = "creep", radius = 6, growth = { cycle = 8, initial_radius = 1 } },
        },
    })

    -- The brood nest feeds the swarm the way a farm does.
    define_entity("brood_nest", {
        race = "swarm",
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        stats = { max_health = 200, sight_range = 3, supply_provided = 6, health_drain = "0" },
        dying = { time = 2 },
        cost = { gold = 40, wood = 20 },
        build_time = 60,
        tags = { "building" },
        field_placement = { ON_CREEP },
        field_effects = { WITHERS_OFF_CREEP },
    })

    define_entity("spawning_pit", {
        race = "swarm",
        location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 500, sight_range = 6, health_drain = "0" },
        dying = { time = 2 },
        cost = { gold = 200, wood = 100 },
        build_time = 120,
        trainer = { "swarmling" },
        tags = { "building" },
        field_placement = { ON_CREEP },
        field_effects = { WITHERS_OFF_CREEP },
    })

    define_entity("swarmling", {
        race = "swarm",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 2, max_health = 35,
            damage = 5, attack_range = 1, acquire_range = 5, attack_period = 4, damage_point = 2,
            health_regen = "0.05",
            sight_range = 8,
            supply_cost = 1,
            cargo_size = 1,
        },
        dying = { time = 2 },
        tags = { "biological" },
        attack = { targets = GROUND | WATER },
        cost = { gold = 50 },
        train_time = 40,
        selection = { priority = 10 },
        field_effects = {
            { field = "creep", of = "anyone", inside = {
                modifiers = { { entity_stat = "speed", op = "percent", value = "0.3" } },
            } },
        },
        -- A swarmling grows into a ravager inside a cocoon: three seconds
        -- wrapped up and helpless but thick-skinned, for a price the pit's
        -- presence unlocks, and the price comes back if the growth is called
        -- off or finds no room to finish.
        morphs = {
            { into = "ravager",
              via = "cocoon",
              time = 60,
              placement = "revalidate",
              cancel = "refundable",
              cost = { resources = { gold = 25, wood = 25 } },
              requires = { "spawning_pit" } },
        },
    })

    -- The cocoon neither moves nor fights; it only endures until it opens.
    define_entity("cocoon", {
        race = "swarm",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = { max_health = 120, armor = 3, sight_range = 2, supply_cost = 1 },
        dying = { time = 2 },
        tags = { "biological" },
    })

    -- What comes out: heavier, harder-hitting, slower, and twice the mouth
    -- to feed.
    define_entity("ravager", {
        race = "swarm",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.25", turn_rate = 24, pivot_rate = 24, radius = "0.5", weight = 3, max_health = 90,
            damage = 12, attack_range = 1, acquire_range = 5, attack_period = 6, damage_point = 3,
            armor = 1,
            health_regen = "0.05",
            sight_range = 8,
            supply_cost = 2,
            cargo_size = 1,
        },
        dying = { time = 2 },
        tags = { "biological" },
        attack = { targets = GROUND | WATER },
        selection = { priority = 12 },
        field_effects = {
            { field = "creep", of = "anyone", inside = {
                modifiers = { { entity_stat = "speed", op = "percent", value = "0.3" } },
            } },
        },
    })

    -- ── The Conclave ───────────────────────────────────────────────────────
    -- Every structure but the nexus and the pylon must be warped in with its
    -- whole footprint inside the conclave's own power, and stands idle — no
    -- training, no firing — while it is not. The nexus and the pylons project that
    -- power. A probe only places a structure and leaves: the warp-in finishes
    -- on its own, so no probe ever stands at a site or mends anything. Nothing
    -- of the conclave's is built on creep, and a pylon that finishes burns
    -- away creep nothing sustains around it.
    local POWERED = { requires = "power", of = "own", coverage = "footprint" }
    local NOT_ON_CREEP = { forbids = "creep" }
    local UNPOWERED_IDLES = { field = "power", of = "own", outside = "disabled" }

    define_entity("nexus", {
        race = "conclave",
        location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 800, sight_range = 9, supply_provided = 10 },
        dying = { time = 2 },
        cost = { gold = 400 },
        build_time = 200,
        trainer = { "probe" },
        resource_storage = { "gold", "wood" },
        tags = { "building" },
        field_placement = { NOT_ON_CREEP },
        field_sources = {
            { field = "power", radius = 7, growth = "instant" },
        },
    })

    -- Probes purge creep nothing sustains from any cell they can see, the way
    -- a finished pylon does around itself.
    define_skill("purge_creep", {
        caster = "entity",
        cooldown = 200,
        target = "position",
        effect = { field = { field = "creep", radius = 3, action = "clear" } },
    })
    define_entity("probe", {
        race = "conclave",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 1, max_health = 30, sight_range = 4,
            build_range = 1, harvest_range = 1,
            supply_cost = 1,
            cargo_size = 1,
        },
        dying = { time = 2 },
        cost = { gold = 50 },
        train_time = 40,
        builder = { builds = { "nexus", "pylon", "gateway", "photon_cannon" }, attendance = "unattended" },
        tags = { "biological" },
        skills = { "purge_creep" },
        resource_carrier = {
            gold = { capacity = 5, time = 20, presence = "hidden" },
            wood = { capacity = 5, time = 20, presence = "present" },
        },
    })

    define_entity("pylon", {
        race = "conclave",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = { max_health = 200, sight_range = 6, supply_provided = 6 },
        dying = { time = 2 },
        cost = { gold = 60 },
        build_time = 50,
        tags = { "building" },
        field_placement = { NOT_ON_CREEP },
        field_sources = {
            { field = "power", radius = 6, growth = "instant" },
        },
        on_stand = {
            { field = { field = "creep", radius = 6, action = "clear" } },
        },
    })

    define_entity("gateway", {
        race = "conclave",
        location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 500, sight_range = 6 },
        dying = { time = 2 },
        cost = { gold = 200, wood = 100 },
        build_time = 120,
        repair_ratio = "0.5",
        trainer = { "zealot" },
        tags = { "building" },
        field_placement = { POWERED, NOT_ON_CREEP },
        field_effects = { UNPOWERED_IDLES },
    })

    define_entity("photon_cannon", {
        race = "conclave",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            max_health = 300, armor = 1,
            damage = 12, attack_range = 6, acquire_range = 7, attack_period = 20, damage_point = 5,
            sight_range = 8,
        },
        dying = { time = 2 },
        cost = { gold = 120 },
        build_time = 80,
        tags = { "building" },
        attack = { targets = GROUND | WATER | AIR, projectile = "arrow" },
        field_placement = { POWERED, NOT_ON_CREEP },
        field_effects = { UNPOWERED_IDLES },
    })

    define_entity("zealot", {
        race = "conclave",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 3, max_health = 60,
            damage = 8, attack_range = 1, acquire_range = 5, attack_period = 5, damage_point = 2,
            armor = 1,
            sight_range = 8,
            supply_cost = 1,
            cargo_size = 1,
        },
        dying = { time = 2 },
        tags = { "biological" },
        attack = { targets = GROUND | WATER },
        cost = { gold = 100 },
        train_time = 60,
        selection = { priority = 10 },
    })
"#;

/// Loads all demo content from Lua into the registry, then validates it. Runs at
/// startup; a content error is a bug in the script above, so it panics.
pub fn register_all(mut registry: ResMut<ContentRegistry>) {
    *registry = content::load(&LuaEngine, CONTENT).expect("demo content must load");
}
