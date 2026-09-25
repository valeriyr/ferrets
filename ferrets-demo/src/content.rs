//! Demo content: five races (human, orc, swarm, conclave, elves) plus neutral
//! resource sources, authored in Lua and loaded at startup.
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
    -- The elves' buildings walk: all but the moon well uproot into a form that
    -- fights and roots again where it stops.
    define_race("elves")
    -- The terrans' production buildings fly, and leave their annexes behind.
    define_race("terran")
    -- The undead raise their buildings on blight and their soldiers from the
    -- bodies everyone else leaves.
    define_race("undead")

    -- Creep covers the ground and recedes ring by ring, half a second a ring,
    -- once nothing sustains it, and whoever spreads it sees every cell of it;
    -- power is there while its pylon stands and gone the tick it falls.
    define_field("creep", { layer = GROUND, decay = { cycle = 10 }, vision = "watched" })
    define_field("power", { layer = GROUND, decay = "instant" })
    -- Blight is the ground the undead build on: it spreads from every
    -- structure they raise, heals what stands on it, and recedes slowly once
    -- nothing sustains it. Unlike creep it watches nothing — the dead see by
    -- their own eyes.
    define_field("blight", { layer = GROUND, decay = { cycle = 40 } })
    -- True sight is what a detector projects. It grants no sight of its own —
    -- a turret sees by its own eyes — but whatever hides on the ground or in
    -- the air under it is seen by whoever covers it. Lying anywhere, it
    -- reaches over water, so a cloaked flier gets no shelter from a lake.
    define_field("true_sight", { layer = "anywhere", decay = "instant", detection = GROUND | AIR })
    -- The veil an arbiter casts over the ground and the air it flies over:
    -- gone the tick it moves on, seeing nothing, detecting nothing; what it
    -- does for those inside is theirs to declare.
    define_field("veil", { layer = "anywhere", decay = "instant" })

    -- What a structure that will not stand on creep declares, and what the
    -- elves add to it: neither the swarm's ground nor the undead's.
    local NOT_ON_CREEP = { forbids = "creep" }
    local NOT_ON_BLIGHT = { forbids = "blight" }

    define_resource("gold")
    define_resource("wood")

    -- Marks the living, which is what a medic will treat and a worker will not.
    -- "building" is pre-registered by the engine.
    define_tag("biological")
    -- Marks what is mended rather than healed: the tanks, the war wagon and
    -- the mortar. A medic passes them by.
    define_tag("mechanical")
    -- Marks an undead hall grown past the necropolis. What the temple asks
    -- for is the tier, not one form of it: a hall grown all the way to the
    -- citadel answers for it as the halls of the dead do.
    define_tag("grown_hall")

    -- What a death hands on, and the deaths that hand it on: something killed
    -- it, or its time ran out. Every ground living unit of every race leaves
    -- this, so a necromancer can raise from anyone's dead. A death that takes
    -- an entity off the board rather than ending it — a builder consumed by
    -- its own site, a canceled construction, a seam built over — leaves
    -- nothing, which is why the deaths are named rather than left to the
    -- engine's rule.
    local FALLS = { time = 2, leaves = { { entity = "corpse", on = { "killed", "expired" } } } }

    -- A body: raceless, ownerless, claiming no cell, lying there thirty
    -- seconds whether or not anybody raises anything from it. The "remains"
    -- tag is the engine's own, like "building": what wears it lies where it is
    -- left, answers to nobody, and is gone when its lifetime runs out.
    define_entity("corpse", {
        location = { occupation = GROUND, size = 1, solidity = "passable" },
        tags = { "remains" },
        stats = { lifetime = 600 },
    })

    -- Projectile kinds. Each is registered by name so the renderer can draw an
    -- arrow differently from a cannonball, and so several weapons can share one.
    -- An arrow and a cannonball follow what they were fired at; a mortar shell is
    -- sent to a cell, so a target that keeps moving escapes the burst.
    define_projectile("arrow", { speed = "1.0", aim = "entity" })
    define_projectile("cannonball", { speed = "0.5", aim = "entity" })
    define_projectile("shell", { speed = "0.2", aim = "position" })

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
        price = { gold = 100, wood = 50 },
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
        price = { gold = 150 },
        time = 240,
        buff = "frenzy_ritual",
        requires = { { entity_type = "pig_farm" } },
    })

    -- The archer's self-buff: a burst of speed and damage that reverts on expiry.
    -- Five seconds at 20 Hz, long enough to watch it work and then wear off.
    define_entity_buff("frenzy", {
        lasting = { ticks = 100 },
        stack = "refresh",
        effects = { { modifiers = {
            { entity_stat = "speed", op = "percent", value = "1.0" },
            { entity_stat = "damage", op = "percent", value = "0.5" },
        } } },
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
    -- The shaman mends the living and nothing else: a filter on the aim, in
    -- the same vocabulary a transporter says whom it carries.
    define_skill("second_wind", {
        caster = "entity",
        cooldown = 120,
        cost = { energy = "20" },
        target = { kind = "allied", only = { tags = { "biological" } } },
        effect = { heal = "15" },
    })
    -- Blood rite unlocks with the frenzy ritual: the button sits greyed on
    -- every grunt until the war camp finishes the research.
    define_skill("blood_rite", {
        caster = "entity",
        cooldown = 160,
        cost = { health = "8", resources = { gold = 10 } },
        target = "caster",
        effect = { apply_buff = "frenzy" },
        requires = { { research = "frenzy_ritual" } },
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
        price = { gold = 50 },
        effect = { apply_buff = "war_drums" },
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
    -- A tree seats one worker in its canopy; a gold mine seats nobody, so a
    -- worker that sits on its source needs something built over the mine.
    define_entity("tree", {
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        resource_source = { kind = "wood", depletion = "destroy" },
        berths = { canopy = { points = { { "0.5", "0.5" } } } },
    })

    -- The spots a worker that attaches to a structure sits at: points every
    -- half cell along the footprint's edge, a fifth of a cell in from it, in
    -- loop order — so a worker crawling from one to the next goes round the
    -- walls, in reach of anything that walks up to them. A group seats one
    -- worker per spot unless it says how many.
    local function rim(width, height)
        local points = {}
        -- Counted in tenths of a cell, so every point is written from whole
        -- numbers.
        local inset = 2
        local w, h = width * 10, height * 10
        local function decimal(tenths)
            return math.floor(tenths / 10) .. "." .. tenths % 10
        end
        local function point(x, y)
            points[#points + 1] = { decimal(x), decimal(y) }
        end
        for x = 5, w - 5, 5 do point(x, inset) end
        for y = 5, h - 5, 5 do point(w - inset, y) end
        for x = w - 5, 5, -5 do point(x, h - inset) end
        for y = h - 5, 5, -5 do point(inset, y) end
        return points
    end

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
            dying = FALLS,
            price = { gold = 50 },
            train_time = 40,
            builder = { builds = builds, attendance = work.attendance },
            -- Workers mend structures and machines at the pace the thing took
            -- to build, and each pays its own share of the bill.
            repairer = {
                repairs = { tags = { "building", "mechanical" } },
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
                gold = { capacity = 5, time = 20, presence = { hidden = { crew = 1 } } },
                wood = { capacity = 5, time = 20, presence = work.wood_presence },
            },
        })
    end

    -- `berths` is the seating a race's attaching worker needs on the
    -- structure, or nil for a race whose workers never sit on their sites.
    local function main_hall(name, race, trains, berths)
        define_entity(name, {
            race = race,
            location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
            -- Enough starting headroom for the first few units; farms carry the
            -- army beyond it. Sight reaches the mine placed by the base, so
            -- the economy never depends on a lucky scout.
            stats = { max_health = 800, sight_range = 9, supply_provided = 10 },
            dying = { time = 2 },
            price = { gold = 400 },
            build_time = 200,
            trainer = { trains },
            resource_storage = { "gold", "wood" },
            tags = { "building" },
            berths = berths,
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
            price = { gold = 40, wood = 20 },
            build_time = 60,
            tags = { "building" },
        })
    end

    local function camp(name, race, trains, researches, berths)
        define_entity(name, {
            race = race,
            location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
            stats = { max_health = 500, sight_range = 6 },
            dying = { time = 2 },
            price = { gold = 200, wood = 100 },
            build_time = 120,
            -- Mends in half the time it took to raise: a camp is quicker to
            -- patch up than to put up.
            repair_ratio = "0.5",
            trainer = trains,
            researcher = researches,
            tags = { "building" },
            berths = berths,
        })
    end

    -- Human: worker, base, training camp, and a ranged unit.
    -- Peasants work in the open and swarm: any number of them can share a site, a
    -- repair or a stand of trees, each adding its own tick of work, so a gang of
    -- them raises a building in a fraction of the time one would take.
    worker("peasant", "human", { "town_hall", "training_camp", "farm", "blacksmith", "bunker" }, {
        attendance = { present = { crew = "any" } },
        repair_presence = { present = { crew = "any" } },
        wood_presence = { present = { crew = "any" } },
    })
    main_hall("town_hall", "human", "peasant")
    farm("farm", "human")
    camp("training_camp", "human", { "archer", "mortar", "medic", "gryphon" })

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
        price = { gold = 100 },
        build_time = 80,
        transporter = {
            -- Soldiers and the siege tube alike: what shelters here is
            -- whatever can be carried through a door.
            carries = { tags = { "biological", "mechanical" } },
            boarding = "own",
            fate = "eject",
            conduct = "fight",
        },
        tags = { "building" },
        -- The garrison keeps watch for what hides, a little short of how far
        -- it sees.
        field_sources = {
            { field = "true_sight", radius = 6, growth = "instant", while_constructing = "nothing", while_disabled = "nothing" },
        },
    })

    -- The human tech building: while one stands, mortars unlock, and it hosts
    -- the iron weapons upgrade.
    define_entity("blacksmith", {
        race = "human",
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        stats = { max_health = 350, sight_range = 5 },
        dying = { time = 2 },
        price = { gold = 150, wood = 80 },
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
        dying = FALLS,
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
        price = { gold = 80 },
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
        dying = FALLS,
        tags = { "biological" },
        repairer = {
            repairs = { tags = { "biological" } },
            rate = { mode = "per_tick", health = "1.0" },
            -- Stays on the map beside its patient, and works alone.
            presence = { present = { crew = 1 } },
            cost = { mode = "energy", per_health = "0.5" },
            -- Never gives up: out of energy it waits at the patient and resumes as
            -- the pool refills.
            patience = nil,
        },
        price = { gold = 100 },
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
        -- A tube on a carriage: it is mended, not healed, and what it leaves
        -- when it falls is wreckage nobody raises anything from.
        dying = { time = 2 },
        tags = { "mechanical" },
        attack = {
            -- Siege fires at the ground, so it must not take aim at what flies: the
            -- blast below already spares fliers, and a mortar allowed to *target*
            -- one would walk into range and drop shells that could never damage it.
            targets = GROUND | WATER,
            -- The shell crosses one cell every five ticks, so a target that keeps
            -- moving takes the direct hit while the burst lands behind it.
            projectile = "shell",
            -- Shelled bodies leave nothing to raise: bringing siege is how an
            -- army denies a necromancer its material.
            slain = "nothing",
            splash = {
                shape = "circular",
                bands = { {1, "0.5"}, {2, "0.25"} },
                layers = GROUND,
                friendly_fire = true,
            },
        },
        price = { gold = 120, wood = 40 },
        train_time = 90,
        selection = { priority = 10 },
        -- Siege needs the forge: no mortars until a blacksmith stands.
        requires = { { entity_type = "blacksmith" } },
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
    define_entity_stat("morph_time", 1)

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
                carries = { types = { "archer" } },
                boarding = "own",
                fate = fate,
                conduct = "fight",
            },
            price = trainable and { gold = 200, wood = 60 } or nil,
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
        price = { gold = 160, wood = 60 },
        train_time = 90,
        -- Carries the workforce and the army alike — and whoever is aboard
        -- when it is shot down goes down with it, the same bargain the
        -- gryphon's rider strikes aloft.
        transporter = {
            carries = { types = { "peon", "grunt", "shaman" } },
            boarding = "own",
            fate = "destroy",
            conduct = "shelter",
        },
        selection = { priority = 10 },
    })

    -- Orc: worker, base, war camp, and a melee unit.
    -- Peons work one to a job and climb onto what they raise: a peon works a
    -- spot on the rim of its site for a second and a half, crawls along the
    -- wall to the next, and works there, in plain view, open to a raid and in
    -- nobody's way, until the walls are up; a repair or a stand only ties one
    -- up in the open. Nothing they do goes faster for a second pair of hands.
    worker("peon", "orc", { "great_hall", "war_camp", "pig_farm", "watch_tower", "siege_works", "big_rock" }, {
        attendance = { attached = { berths = "rim", stance = { circling = { speed = "0.1", dwell = 30 } } } },
        repair_presence = { present = { crew = 1 } },
        wood_presence = { present = { crew = 1 } },
    })
    -- The peon crawls round the rim of everything it raises, so every orc
    -- structure seats it there.
    main_hall("great_hall", "orc", "peon", { rim = { points = rim(3, 3) } })

    -- The big rock: a monument to nothing. Four cells across, a minute to
    -- raise, and it does nothing at all once it stands — it is there so a
    -- gang of peons can be watched crawling round a long job.
    define_entity("big_rock", {
        race = "orc",
        location = { occupation = GROUND, size = { 4, 4 }, solidity = "solid" },
        stats = { max_health = 2000, sight_range = 2 },
        dying = { time = 2 },
        price = { gold = 50 },
        build_time = 1200,
        tags = { "building" },
        berths = { rim = { points = rim(4, 4) } },
    })

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
        price = { gold = 40, wood = 20 },
        build_time = 60,
        transporter = {
            carries = { types = { "peon" } },
            boarding = "own",
            fate = "destroy",
            conduct = "shelter",
        },
        tags = { "building" },
        berths = { rim = { points = rim(2, 2) } },
    })
    -- The orc watch tower: what it can be hit by and where it stands are
    -- different answers, like the grounded gryphon. It is rooted on the ground
    -- and blocks only the ground, so fliers pass over it freely — but it stands
    -- tall enough to be shot out of the air, which `targetable` says and
    -- `occupation` could not. Occupying the air instead would make it a wall
    -- across the sky, which is the fortress's job, not a tower's.
    --
    -- The watch tower only watches: the farthest eyes on the orc side, the
    -- orcs' one detector, and no weapon. The upgrade trades some of that
    -- watch for bolts — the guard tower below sees less, detects nothing, and
    -- is the orc answer to fliers.
    define_entity("watch_tower", {
        race = "orc",
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        targetable = GROUND | AIR,
        stats = {
            max_health = 250,
            sight_range = 12,
        },
        dying = { time = 2 },
        price = { gold = 120, wood = 40 },
        build_time = 70,
        tags = { "building" },
        berths = { rim = { points = rim(2, 2) } },
        field_sources = {
            { field = "true_sight", radius = 8, growth = "instant", while_constructing = "nothing", while_disabled = "nothing" },
        },
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
        -- The watch tower's price and the upgrade's on top of it, over the
        -- raising and the upgrading together: what the tower cost to have.
        price = { gold = 200, wood = 60 },
        build_time = 130,
    })
    camp("war_camp", "orc", { "grunt", "shaman", "zeppelin" }, { "frenzy_ritual" }, { rim = { points = rim(3, 3) } })

    -- The orc siege works: the one building that exists to train a single unit,
    -- and gated behind the war camp, so the wagon is a second-thought answer to a
    -- dug-in enemy rather than an opening move. Not a camp: it trains no
    -- infantry and hosts no research, and its own walls are thinner than one.
    define_entity("siege_works", {
        race = "orc",
        location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 400, sight_range = 5 },
        dying = { time = 2 },
        price = { gold = 180, wood = 120 },
        build_time = 130,
        repair_ratio = "0.5",
        trainer = { "war_wagon" },
        tags = { "building" },
        berths = { rim = { points = rim(3, 3) } },
        requires = { { entity_type = "war_camp" } },
    })

    define_turret("siege_cannon", {
        targets = GROUND | WATER,
        projectile = "cannonball",
        conduct = "on_the_move",
        -- A cannonball leaves no body behind, as the mortar's shell does not.
        slain = "nothing",
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
        price = { gold = 160, wood = 60 },
        train_time = 110,
        tags = { "mechanical" },
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
        dying = FALLS,
        tags = { "biological" },
        -- An axe reaches what stands on the ground or floats on the water and
        -- nothing that flies.
        attack = { targets = GROUND | WATER },
        -- Blood rite: the grunt buys the archer's frenzy with its own blood and
        -- a little gold — regeneration (above) walks the price off afterwards.
        skills = { "blood_rite" },
        price = { gold = 90 },
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
        dying = FALLS,
        tags = { "biological" },
        skills = { "second_wind" },
        price = { gold = 120 },
        train_time = 80,
        -- Support trails combat units in a mixed selection, like the medic.
        selection = { priority = 5 },
        -- A completed research as a requirement: shamans answer the ritual.
        requires = { { research = "frenzy_ritual" } },
    })

    -- ── The Swarm ──────────────────────────────────────────────────────────
    -- The hall — a hatchery, and the hive it grows into — bears larvae, and
    -- larvae are what every swarm unit grows from inside an egg: the pit only
    -- unlocks the swarmling. A drone is spent on what it builds: it walks to
    -- the site, pays, and becomes the structure going up, so the swarm has no
    -- repairers. Every structure but the hall must stand on creep; the hall
    -- itself spreads it, at full reach when it is placed by the map, and from
    -- three cells a cell every third of a second when it is built, showing a
    -- patch under itself while still going up. Structures left off creep
    -- waste away; a larva off creep is gone in a second; swarmlings run a
    -- third faster on anyone's creep.
    local ON_CREEP = { requires = "creep", of = "anyone", coverage = "every" }
    local WITHERS_OFF_CREEP = { field = "creep", of = "anyone", coverage = "every", outside = {
        modifiers = { { entity_stat = "health_drain", op = "flat", value = "0.2" } },
    } }
    local DIES_OFF_CREEP = { field = "creep", of = "anyone", coverage = "every", outside = {
        modifiers = { { entity_stat = "health_drain", op = "flat", value = "1.25" } },
    } }

    -- What every swarm structure is full of: spawn too young to be a
    -- swarmling, which lives twenty seconds and is gone. It costs no supply,
    -- is trained by nothing, and leaves no body — there was never enough of it
    -- to bury.
    define_entity("hatchling", {
        race = "swarm",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.32", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 1, max_health = 25,
            damage = 4, attack_range = 1, acquire_range = 5, attack_period = 6, damage_point = 2,
            sight_range = 6,
            lifetime = 400,
            cargo_size = 1,
        },
        dying = { time = 1 },
        tags = { "biological" },
        attack = { targets = GROUND | WATER },
        selection = { priority = 10 },
        field_effects = {
            { field = "creep", of = "anyone", coverage = "any", inside = {
                modifiers = { { entity_stat = "speed", op = "percent", value = "0.3" } },
            } },
        },
    })

    -- What a swarm structure is holding when it falls, and only when something
    -- kills it — a building called off or withered away has nothing left in
    -- it. The demo's one death that hands on units rather than a body.
    local BURSTS = { { entity = "hatchling", count = 2, on = { "killed" } } }

    -- The hatchery is the swarm's first hall. Five spots along the ground at
    -- its southern foot, one row outside the footprint, are where its larvae
    -- crawl; four sit at once, so one larva given back by a canceled egg
    -- always finds a seat. A larva waits the tick the hall stands, then one
    -- comes every eleven seconds while fewer than three crawl it. Larvae
    -- outlive the hall: they are set down beside its ruin, keep their
    -- growths, wither with the creep once nothing sustains it, and are taken
    -- in by any hall of theirs within four cells that has a seat free. Once
    -- the pit stands the hatchery grows into a hive inside a cocoon whose own
    -- berths carry the larvae across.
    local BROOD_BERTHS = { brood = { points = {
        { "0.5", "3.5" }, { "1.0", "3.5" }, { "1.5", "3.5" }, { "2.0", "3.5" }, { "2.5", "3.5" },
    }, slots = 4 } }
    local RESEATS_NEARBY = { linger = { reseat = { distance = 4 } } }
    local CREEP_RADIUS = 10
    local CREEP_GROWTH = { cycle = 6, initial_radius = 3 }
    local CREEP_SPREAD = { { field = "creep", radius = CREEP_RADIUS, growth = CREEP_GROWTH, while_constructing = "nothing", while_disabled = "full" } }

    -- Burrowing, researched at the hall: a swarmling digs in where it stands
    -- and lies unseen until a detector passes over it.
    define_research("burrow", {
        price = { gold = 100 },
        time = 150,
    })

    define_entity("hatchery", {
        race = "swarm",
        location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 800, sight_range = 9, supply_provided = 10 },
        dying = { time = 2, leaves = BURSTS },
        price = { gold = 400 },
        build_time = 200,
        resource_storage = { "gold", "wood" },
        tags = { "building" },
        berths = BROOD_BERTHS,
        breeder = { breeds = "larva", period = 220, limit = 3, initial = 1, orphans = RESEATS_NEARBY },
        researcher = { "burrow" },
        field_sources = {
            { field = "creep", radius = CREEP_RADIUS, growth = CREEP_GROWTH, while_constructing = { held = 1 }, while_disabled = "full" },
        },
        morphs = {
            { into = "hive", via = "hive_cocoon", time = 200, placement = "reserve", cancel = "refundable",
              interrupted = "reverts", reason = "change", cost = { resources = { gold = 150, wood = 100 } },
              requires = { { entity_type = "spawning_pit" } } },
        },
    })

    -- The cocoon keeps the hall's creep, storage, headroom and berths, so the
    -- larvae keep crawling its foot; it breeds none while it is worn.
    define_entity("hive_cocoon", {
        race = "swarm",
        location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 800, sight_range = 9, supply_provided = 10 },
        dying = { time = 2, leaves = BURSTS },
        resource_storage = { "gold", "wood" },
        tags = { "building" },
        berths = BROOD_BERTHS,
        field_sources = CREEP_SPREAD,
    })

    -- The hive breeds faster, opens with two larvae, and is what a ravager
    -- takes to grow.
    define_entity("hive", {
        race = "swarm",
        location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 1000, sight_range = 10, supply_provided = 10 },
        dying = { time = 2, leaves = BURSTS },
        -- Nothing builds a hive: it carries the hatchery's price and the
        -- growth's on top of it, over the raising and the growing together.
        price = { gold = 550, wood = 100 },
        build_time = 400,
        resource_storage = { "gold", "wood" },
        tags = { "building" },
        berths = BROOD_BERTHS,
        breeder = { breeds = "larva", period = 180, limit = 3, initial = 2, orphans = RESEATS_NEARBY },
        researcher = { "burrow" },
        field_sources = CREEP_SPREAD,
    })

    -- A larva is what the swarm makes its units from. It crawls its hall's
    -- brood berths holding no cells, cannot be moved, and does not survive
    -- off creep: a drain empties its pool in a second, fast but watchable.
    -- Each growth pays the unit's price and holds its supply from the first
    -- tick; a growth called off gives the larva back.
    define_entity("larva", {
        race = "swarm",
        location = { occupation = GROUND, size = 1, solidity = "passable" },
        stats = { max_health = 25, armor = 10, sight_range = 1, health_drain = "0" },
        dying = { time = 1 },
        tags = { "biological" },
        selection = { priority = 1 },
        broodling = { berths = "brood", stance = { roaming = { speed = "0.05", dwell = 40 } } },
        field_effects = { DIES_OFF_CREEP },
        morphs = {
            { into = "drone", via = "egg", time = 40, placement = "nearby", cancel = "refundable",
              interrupted = "reverts", reason = "production", cost = { resources = { gold = 50 } } },
            { into = "swarmling", via = "egg", time = 40, placement = "nearby", cancel = "refundable",
              interrupted = "reverts", reason = "production", cost = { resources = { gold = 50 } },
              requires = { { entity_type = "spawning_pit" } } },
            { into = "overlord", via = "egg", time = 60, placement = "nearby", cancel = "refundable",
              interrupted = "reverts", reason = "production", cost = { resources = { gold = 100 } } },
        },
    })

    -- The overlord feeds the swarm the way a farm does, from the air: a slow
    -- floating sac the size of a building that needs no creep under it — and
    -- the swarm's eyes for what hides: it sees through cloaks as far as it
    -- sees at all.
    define_entity("overlord", {
        race = "swarm",
        location = { occupation = AIR, size = { 2, 2 }, solidity = "solid" },
        stats = {
            speed = "0.2", turn_rate = 4, pivot_rate = 6, pivot_angle = 90, radius = "1", weight = 6,
            max_health = 200, armor = 1, sight_range = 8, supply_provided = 6,
        },
        dying = { time = 2 },
        tags = { "biological" },
        field_sources = {
            { field = "true_sight", radius = 8, growth = "instant", while_constructing = "nothing", while_disabled = "nothing" },
        },
    })

    -- The egg stands on the ground and neither moves nor fights; it blocks
    -- the cell it sits on until it opens.
    define_entity("egg", {
        race = "swarm",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = { max_health = 200, armor = 10, sight_range = 1 },
        dying = { time = 1 },
        tags = { "biological" },
        selection = { priority = 1 },
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
        dying = FALLS,
        builder = { builds = { "hatchery", "tumor", "spawning_pit" }, attendance = "consumed" },
        tags = { "biological" },
        skills = { "spew_creep" },
        resource_carrier = {
            gold = { capacity = 5, time = 20, presence = { hidden = { crew = 1 } } },
            wood = { capacity = 5, time = 20, presence = { hidden = { crew = 1 } } },
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
        price = { gold = 25 },
        build_time = 40,
        tags = { "building" },
        field_placement = { ON_CREEP },
        field_sources = {
            { field = "creep", radius = 6, growth = { cycle = 8, initial_radius = 1 }, while_constructing = "nothing", while_disabled = "full" },
        },
    })

    define_entity("spawning_pit", {
        race = "swarm",
        location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 500, sight_range = 6, health_drain = "0" },
        dying = { time = 2, leaves = BURSTS },
        price = { gold = 200, wood = 100 },
        build_time = 120,
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
        dying = FALLS,
        tags = { "biological" },
        attack = { targets = GROUND | WATER },
        selection = { priority = 10 },
        field_effects = {
            { field = "creep", of = "anyone", coverage = "any", inside = {
                modifiers = { { entity_stat = "speed", op = "percent", value = "0.3" } },
            } },
        },
        -- A swarmling grows into a ravager inside a cocoon: three seconds
        -- wrapped up and helpless but thick-skinned, for a price the hive's
        -- presence unlocks, and the price comes back if the growth is called
        -- off; the ravager lands on the nearest ground that takes it.
        morphs = {
            { into = "ravager",
              via = "cocoon",
              time = 60,
              placement = "nearby",
              cancel = "refundable",
              cost = { resources = { gold = 25, wood = 25 } },
              requires = { { entity_type = "hive" } } },
            -- Digging in takes a moment and cannot be called off; the ground
            -- it digs into is the ground it stands on.
            { into = "swarmling_burrowed",
              time = 12,
              placement = "reserve",
              cancel = "committed",
              reason = "change",
              requires = { { research = "burrow" } } },
        },
    })

    -- A burrowed swarmling: under the ground, so others walk over it and no
    -- side but its own sees it without a detector. It neither moves nor
    -- bites, heals faster than one above ground, and is dug out — onto the
    -- nearest free ground, since something may stand on top of it — by the
    -- change back.
    define_entity("swarmling_burrowed", {
        race = "swarm",
        -- Underfoot: a swarmling walks over what has dug in, and nothing is
        -- raised on top of it.
        location = { occupation = GROUND, size = 1, solidity = "underfoot" },
        stats = {
            radius = "0.5", weight = 2, max_health = 35,
            health_regen = "0.15",
            sight_range = 6,
            supply_cost = 1,
        },
        concealment = "concealed",
        dying = FALLS,
        tags = { "biological" },
        selection = { priority = 10 },
        morphs = {
            { into = "swarmling",
              time = 12,
              placement = "nearby",
              cancel = "committed",
              reason = "change" },
        },
    })

    -- The cocoon neither moves nor fights; it only endures until it opens,
    -- holding the ravager's supply the while.
    define_entity("cocoon", {
        race = "swarm",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = { max_health = 120, armor = 3, sight_range = 2 },
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
        dying = FALLS,
        tags = { "biological" },
        attack = { targets = GROUND | WATER },
        selection = { priority = 12 },
        field_effects = {
            { field = "creep", of = "anyone", coverage = "any", inside = {
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
    local POWERED = { requires = "power", of = "own", coverage = "every" }
    local UNPOWERED_IDLES = { field = "power", of = "own", coverage = "every", outside = "disable" }

    -- Every conclave unit but the arbiter itself declares that the veil of
    -- its own side, or an ally's, conceals it while it stands inside.
    local VEILED = { field = "veil", of = "allied", coverage = "every", inside = "conceal" }

    define_entity("nexus", {
        race = "conclave",
        location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 800, sight_range = 9, supply_provided = 10 },
        dying = { time = 2 },
        price = { gold = 400 },
        build_time = 200,
        trainer = { "probe" },
        resource_storage = { "gold", "wood" },
        tags = { "building" },
        field_placement = { NOT_ON_CREEP },
        field_sources = {
            { field = "power", radius = 7, growth = "instant", while_constructing = "nothing", while_disabled = "nothing" },
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
        dying = FALLS,
        price = { gold = 50 },
        train_time = 40,
        builder = { builds = { "nexus", "pylon", "gateway", "photon_cannon" }, attendance = "unattended" },
        tags = { "biological" },
        skills = { "purge_creep" },
        field_effects = { VEILED },
        resource_carrier = {
            gold = { capacity = 5, time = 20, presence = { hidden = { crew = 1 } } },
            wood = { capacity = 5, time = 20, presence = { present = { crew = 1 } } },
        },
    })

    define_entity("pylon", {
        race = "conclave",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = { max_health = 200, sight_range = 6, supply_provided = 6 },
        dying = { time = 2 },
        price = { gold = 60 },
        build_time = 50,
        tags = { "building" },
        field_placement = { NOT_ON_CREEP },
        field_sources = {
            { field = "power", radius = 6, growth = "instant", while_constructing = "nothing", while_disabled = "nothing" },
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
        price = { gold = 200, wood = 100 },
        build_time = 120,
        repair_ratio = "0.5",
        trainer = { "zealot", "dark_templar", "observer", "arbiter" },
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
        price = { gold = 120 },
        build_time = 80,
        tags = { "building" },
        attack = { targets = GROUND | WATER | AIR, projectile = "arrow" },
        field_placement = { POWERED, NOT_ON_CREEP },
        field_effects = { UNPOWERED_IDLES },
        -- The cannon is the conclave's standing detector: a shorter reach
        -- than its eyes, as a turret's is.
        field_sources = {
            { field = "true_sight", radius = 7, growth = "instant", while_constructing = "nothing", while_disabled = "nothing" },
        },
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
        dying = FALLS,
        tags = { "biological" },
        attack = { targets = GROUND | WATER },
        price = { gold = 100 },
        train_time = 60,
        selection = { priority = 10 },
        field_effects = { VEILED },
    })

    -- The dark templar is never seen by an enemy without a detector, and
    -- nothing it does gives it away: it walks, swings and kills under the
    -- cloak it was born with.
    define_entity("dark_templar", {
        race = "conclave",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 3, max_health = 80,
            damage = 20, attack_range = 1, acquire_range = 5, attack_period = 20, damage_point = 6,
            armor = 1,
            sight_range = 8,
            supply_cost = 2,
            cargo_size = 1,
        },
        concealment = "concealed",
        dying = FALLS,
        tags = { "biological" },
        attack = { targets = GROUND | WATER },
        price = { gold = 125, wood = 50 },
        train_time = 75,
        selection = { priority = 10 },
        field_effects = { VEILED },
    })

    -- The observer: a cloaked eye that flies, with no weapon, that sees what
    -- hides as far as it sees at all — the thing that finds the other side's
    -- observer.
    define_entity("observer", {
        race = "conclave",
        location = { occupation = AIR, size = 1, solidity = "solid" },
        stats = {
            speed = "0.35", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 1, max_health = 40,
            sight_range = 9,
            supply_cost = 1,
        },
        concealment = "concealed",
        dying = { time = 2 },
        tags = { "mechanical" },
        price = { gold = 50, wood = 50 },
        train_time = 50,
        selection = { priority = 5 },
        field_sources = {
            { field = "true_sight", radius = 9, growth = "instant", while_constructing = "nothing", while_disabled = "nothing" },
        },
        field_effects = { VEILED },
    })

    -- The arbiter carries the veil: everything of its side's under it is
    -- concealed, itself excepted, and the veil moves with it.
    define_entity("arbiter", {
        race = "conclave",
        location = { occupation = AIR, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", turn_rate = 20, pivot_rate = 20, radius = "0.5", weight = 4, max_health = 200,
            armor = 1,
            sight_range = 9,
            supply_cost = 3,
        },
        dying = { time = 2 },
        tags = { "mechanical" },
        price = { gold = 200, wood = 150 },
        train_time = 120,
        selection = { priority = 5 },
        field_sources = {
            { field = "veil", radius = 4, growth = "instant", while_constructing = "nothing", while_disabled = "nothing" },
        },
    })

    -- ── The Elves ──────────────────────────────────────────────────────────
    -- Nothing of the elves' is carried home: a wisp hovers in a tree and draws
    -- wood out of it without felling it, alone, or works the rim of a mine
    -- the elves have entangled, spot by spot with its fellows, draining it —
    -- and the take goes straight to the stockpile, so no elf building stores
    -- anything. A wisp is spent on what it builds, like a drone. Every
    -- structure but the moon well roots and uproots: rooted it trains or
    -- shoots at range; uprooted it walks and bites, and trains nothing.
    -- Nothing of the elves' takes root on creep or on blight, neither a
    -- wisp's site nor a walker settling down: the ground another race has
    -- made its own is ground no ancient will stand in.
    define_entity("wisp", {
        race = "elves",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 1, max_health = 30, sight_range = 4,
            build_range = 1, harvest_range = 1,
            supply_cost = 1,
            cargo_size = 1,
        },
        dying = { time = 2 },
        price = { gold = 50 },
        train_time = 40,
        builder = {
            builds = { "tree_of_life", "moon_well", "ancient_of_war", "ancient_protector", "entangled_mine" },
            attendance = "consumed",
        },
        tags = { "biological" },
        -- Gold is drawn from the rim of an entangled mine: a wisp works a spot
        -- for two seconds, then drifts straight across to another free one of
        -- its own picking, so a crowd of them never moves as one.
        -- Wood is drawn from a tree's canopy, the wisp hovering in a slow
        -- circle inside the crown.
        resource_carrier = {
            gold = {
                capacity = 5, time = 20, banking = "direct",
                presence = { attached = { berths = "rim", stance = { roaming = { speed = "0.05", dwell = 40 } } } },
            },
            wood = {
                capacity = 5, time = 20, banking = "direct", drain = 0,
                presence = { attached = { berths = "canopy", stance = { orbit = { radius = "0.3", period = 60 } } } },
            },
        },
    })

    -- Raised over a gold mine, the entangled mine is the mine from then on:
    -- it takes the gold that was left, seats five wisps round its rim, falls
    -- when they drain it, and gives the mine back with what remains if it is
    -- torn down or destroyed first. Only the elves that own it may work it.
    -- A mine is where it is, so creep under it is no bar, unlike the ancients.
    define_entity("entangled_mine", {
        race = "elves",
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        stats = { max_health = 600, sight_range = 6 },
        dying = { time = 2 },
        price = { gold = 100 },
        build_time = 100,
        tags = { "building" },
        resource_source = { kind = "gold", depletion = "destroy" },
        overbuilds = "gold_mine",
        -- Twelve spots to drift between, five wisps at a time on them.
        berths = { rim = { points = rim(2, 2), slots = 5 } },
    })

    -- A rooted form and the walker it uproots into, as one pair: the same
    -- footprint both ways, so the ground under a settling walker is exactly
    -- the ground it walked on. Rooting reserves that ground the moment it is
    -- ordered; uprooting checks nothing, since the cells are already its own.
    -- Both take two seconds, cost nothing, and can be called off. The walker
    -- keeps the building tag and the headroom the rooted form gives, and
    -- bites at what it reaches.
    local function ancient(name, size, rooted, walker)
        local uprooted = name .. "_uprooted"
        define_entity(name, {
            race = "elves",
            location = { occupation = GROUND, size = size, solidity = "solid" },
            stats = rooted.stats,
            dying = { time = 2 },
            price = rooted.price,
            build_time = rooted.build_time,
            trainer = rooted.trainer,
            attack = rooted.attack,
            tags = { "building" },
            field_placement = { NOT_ON_CREEP, NOT_ON_BLIGHT },
            field_sources = rooted.field_sources,
            morphs = {
                { into = uprooted, time = 40, placement = "revalidate", cancel = "refundable" },
            },
        })
        define_entity(uprooted, {
            race = "elves",
            location = { occupation = GROUND, size = size, solidity = "solid" },
            stats = walker.stats,
            dying = { time = 2 },
            attack = { targets = GROUND },
            tags = { "building" },
            selection = { priority = 6 },
            morphs = {
                { into = name, time = 40, placement = "reserve", cancel = "refundable" },
            },
        })
    end

    -- The hall: trains wisps rooted; uprooted it lumbers and swats.
    ancient("tree_of_life", { 3, 3 }, {
        stats = { max_health = 800, sight_range = 9, supply_provided = 10 },
        price = { gold = 400 },
        build_time = 200,
        trainer = { "wisp" },
    }, {
        stats = {
            speed = "0.1", turn_rate = 6, pivot_rate = 6, pivot_angle = 90, radius = "1.5", weight = 12,
            max_health = 800, sight_range = 9, supply_provided = 10,
            damage = 20, attack_range = 1, acquire_range = 5, attack_period = 30, damage_point = 10,
        },
    })
    -- The moon well feeds the army the way a farm does. It is the one elf
    -- structure with no legs: a well stays where it was dug.
    define_entity("moon_well", {
        race = "elves",
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        stats = { max_health = 200, sight_range = 3, supply_provided = 6 },
        dying = { time = 2 },
        price = { gold = 40, wood = 20 },
        build_time = 60,
        tags = { "building" },
        field_placement = { NOT_ON_CREEP, NOT_ON_BLIGHT },
    })
    -- The war ancient: huntresses rooted, a heavy bite uprooted.
    ancient("ancient_of_war", { 3, 3 }, {
        stats = { max_health = 500, sight_range = 6 },
        price = { gold = 200, wood = 100 },
        build_time = 120,
        trainer = { "huntress" },
    }, {
        stats = {
            speed = "0.15", turn_rate = 8, pivot_rate = 8, pivot_angle = 90, radius = "1.5", weight = 10,
            max_health = 500, sight_range = 6,
            damage = 25, attack_range = 1, acquire_range = 5, attack_period = 25, damage_point = 8,
        },
    })
    -- The tower: rooted it throws at range, over ground and water and into the
    -- air; uprooted it can only bite at what walks up to it.
    -- Rooted, the protector also sees through cloaks: the elves' one detector.
    ancient("ancient_protector", { 2, 2 }, {
        stats = {
            max_health = 300, armor = 1, sight_range = 8,
            damage = 14, attack_range = 6, acquire_range = 7, attack_period = 20, damage_point = 5,
        },
        price = { gold = 120, wood = 40 },
        build_time = 80,
        attack = { targets = GROUND | WATER | AIR, projectile = "arrow" },
        field_sources = {
            { field = "true_sight", radius = 7, growth = "instant", while_constructing = "nothing", while_disabled = "nothing" },
        },
    }, {
        stats = {
            speed = "0.15", turn_rate = 12, pivot_rate = 12, radius = "1", weight = 6,
            max_health = 300, armor = 1, sight_range = 8,
            damage = 14, attack_range = 1, acquire_range = 5, attack_period = 20, damage_point = 5,
        },
    })

    -- The huntress lies in ambush: unseen for fifteen seconds, or until she
    -- strikes or is struck — whichever comes first ends it.
    define_entity_buff("ambushing", {
        lasting = { ticks = 300 },
        stack = "refresh",
        effects = { "conceal" },
        interrupted_by = { "attack", "hit" },
    })
    define_skill("ambush", {
        caster = "entity",
        cooldown = 400,
        target = "caster",
        effect = { apply_buff = "ambushing" },
    })
    define_entity("huntress", {
        race = "elves",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.35", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 2, max_health = 55,
            damage = 9, attack_range = 1, acquire_range = 6, attack_period = 5, damage_point = 2,
            sight_range = 9,
            supply_cost = 1,
            cargo_size = 1,
        },
        dying = FALLS,
        tags = { "biological" },
        attack = { targets = GROUND | WATER },
        price = { gold = 90, wood = 10 },
        train_time = 55,
        selection = { priority = 10 },
        skills = { "ambush" },
    })

    -- ── The Terrans ─────────────────────────────────────────────────────────
    -- The race that takes its buildings with it. The command center, the
    -- barracks and the factory lift off into the air layer and set down again
    -- wherever the ground is clear, leaving whatever was docked beside them
    -- behind — and an abandoned annex answers to whoever lands next to it.
    --
    -- The SCV attends each job in its own way: it sits on the site it raises,
    -- stands next to the tree it cuts, crowds a repair with its fellows, and
    -- disappears into a refinery to load up.
    --
    -- The scanner sweep: sight over a patch of map for nine seconds, cast at
    -- any cell whether or not it can be seen, and outliving the station that
    -- cast it. It leaves no field and no unit behind — only the watch itself.
    define_skill("scanner_sweep", {
        caster = "entity",
        cooldown = 200,
        cost = { energy = "50" },
        target = "position",
        effect = { watch = { radius = 6, duration = 180, detection = GROUND | AIR } },
    })

    -- The wraith's cloak: a buff that conceals and nothing else, switched on
    -- for a price and kept up from the pool every tick, so a wraith that lets
    -- its energy run dry is seen again. Two hundred energy at a quarter a tick
    -- net of regeneration is about a minute of cloak. Switching it off is
    -- taking the buff back. Neither shooting nor casting ends it.
    define_entity_buff("cloaked", {
        lasting = { upkeep = { cost = { energy = "0.25" }, period = 1 } },
        stack = "ignore",
        effects = { "conceal" },
    })
    define_skill("cloak", {
        caster = "entity",
        cooldown = 20,
        cost = { energy = "25" },
        target = "caster",
        effect = { apply_buff = "cloaked" },
    })
    define_skill("decloak", {
        caster = "entity",
        cooldown = 0,
        target = "caster",
        effect = { remove_buff = "cloaked" },
    })

    -- Siege mechanics, researched in the tech lab and unlocking nothing but
    -- the tank's other form.
    define_research("siege_tech", {
        price = { gold = 100, wood = 50 },
        time = 160,
    })

    -- One at a time to a job, and on the job itself while it builds: the SCV
    -- takes the single berth of whatever it is raising, works a tree from the
    -- next cell over, and crowds around a repair with every other SCV sent to
    -- it.
    define_entity("scv", {
        race = "terran",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 1, max_health = 40, sight_range = 4,
            repair_speed = "1.0", repair_cost_factor = "0.25", repair_range = 1,
            build_range = 1, harvest_range = 1,
            supply_cost = 1,
            cargo_size = 1,
        },
        dying = FALLS,
        price = { gold = 50 },
        train_time = 40,
        builder = {
            builds = { "command_center", "barracks", "factory", "supply_depot", "refinery", "missile_turret" },
            attendance = { attached = { berths = "rim", stance = { roaming = { speed = "0.08", dwell = 30 } } } },
        },
        repairer = {
            repairs = { tags = { "building", "mechanical" } },
            rate = { mode = "production" },
            presence = { present = { crew = "any" } },
            cost = { mode = "pro_rata" },
            patience = 200,
        },
        tags = { "biological" },
        -- Gold is worked from inside a refinery and nowhere else: one SCV at a
        -- time disappears into it, and a bare seam is not a source it may
        -- work. Wood is cut from the next cell over, one axe to a tree.
        resource_carrier = {
            gold = { capacity = 5, time = 20, presence = { hidden = { crew = 1 } }, sources = { types = { "refinery" } } },
            wood = { capacity = 5, time = 20, presence = { present = { crew = 1 } } },
        },
    })

    -- The flying structures are pairs, as the elves' ancients are: the
    -- grounded form works, the airborne one only flies. Lift-off revalidates
    -- (nothing contests the sky) and landing reserves the ground it is coming
    -- down on, so the spot cannot be built over mid-descent.
    local LIFTS = { time = 40, placement = "revalidate", cancel = "committed" }
    local LANDS = { time = 40, placement = "reserve", cancel = "committed" }

    define_entity("command_center", {
        race = "terran",
        location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 800, sight_range = 9, supply_provided = 10, build_range = 1 },
        dying = { time = 2 },
        price = { gold = 400 },
        build_time = 200,
        trainer = { "scv" },
        resource_storage = { "gold", "wood" },
        -- It raises its own annex, standing where it stands while the work is
        -- done: the dock is the next cell over, so there is nowhere to walk.
        builder = { builds = { "comsat_station" }, attendance = { present = { crew = 1 } } },
        docks = { { at = { 3, 0 }, accepts = { types = { "comsat_station" } } } },
        berths = { rim = { points = rim(3, 3), slots = 1 } },
        tags = { "building" },
        morphs = {
            { into = "command_center_aloft", time = LIFTS.time, placement = LIFTS.placement, cancel = LIFTS.cancel },
        },
    })
    -- Aloft it trains nothing, stores nothing, docks nothing and defends
    -- itself with nothing: a building in transit, and the easiest target on
    -- the map.
    define_entity("command_center_aloft", {
        race = "terran",
        location = { occupation = AIR, size = { 3, 3 }, solidity = "solid" },
        stats = {
            speed = "0.12", turn_rate = 6, pivot_rate = 6, pivot_angle = 90, radius = "1.5", weight = 12,
            max_health = 800, sight_range = 9, supply_provided = 10,
        },
        dying = { time = 2 },
        tags = { "building" },
        selection = { priority = 6 },
        morphs = {
            { into = "command_center", time = LANDS.time, placement = LANDS.placement, cancel = LANDS.cancel },
        },
    })

    -- Infantry come from a hall of their own. It flies like the other two, but
    -- it offers no dock, so it leaves nothing behind when it goes.
    define_entity("barracks", {
        race = "terran",
        location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 500, sight_range = 6 },
        dying = { time = 2 },
        price = { gold = 150 },
        build_time = 100,
        repair_ratio = "0.5",
        trainer = { "marine" },
        berths = { rim = { points = rim(3, 3), slots = 1 } },
        tags = { "building" },
        morphs = {
            { into = "barracks_aloft", time = LIFTS.time, placement = LIFTS.placement, cancel = LIFTS.cancel },
        },
    })
    define_entity("barracks_aloft", {
        race = "terran",
        location = { occupation = AIR, size = { 3, 3 }, solidity = "solid" },
        stats = {
            speed = "0.15", turn_rate = 6, pivot_rate = 6, pivot_angle = 90, radius = "1.5", weight = 10,
            max_health = 500, sight_range = 6,
        },
        dying = { time = 2 },
        tags = { "building" },
        selection = { priority = 6 },
        morphs = {
            { into = "barracks", time = LANDS.time, placement = LANDS.placement, cancel = LANDS.cancel },
        },
    })

    define_entity("factory", {
        race = "terran",
        location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 500, sight_range = 6, build_range = 1 },
        dying = { time = 2 },
        price = { gold = 200, wood = 100 },
        build_time = 120,
        repair_ratio = "0.5",
        trainer = { "tank", "wraith" },
        builder = { builds = { "tech_lab" }, attendance = { present = { crew = 1 } } },
        docks = { { at = { 3, 0 }, accepts = { types = { "tech_lab" } } } },
        berths = { rim = { points = rim(3, 3), slots = 1 } },
        tags = { "building" },
        morphs = {
            { into = "factory_aloft", time = LIFTS.time, placement = LIFTS.placement, cancel = LIFTS.cancel },
        },
    })
    define_entity("factory_aloft", {
        race = "terran",
        location = { occupation = AIR, size = { 3, 3 }, solidity = "solid" },
        stats = {
            speed = "0.15", turn_rate = 6, pivot_rate = 6, pivot_angle = 90, radius = "1.5", weight = 10,
            max_health = 500, sight_range = 6,
        },
        dying = { time = 2 },
        tags = { "building" },
        selection = { priority = 6 },
        morphs = {
            { into = "factory", time = LANDS.time, placement = LANDS.placement, cancel = LANDS.cancel },
        },
    })

    -- The comsat station: an annex with a pool of its own, and the only
    -- building in the demo that casts. Without a command center beside it it
    -- stands switched off, waiting; land any player's command center next to
    -- it and it is theirs.
    define_entity("comsat_station", {
        race = "terran",
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        stats = {
            max_health = 250, sight_range = 6,
            -- Two hundred to a pool that refills slowly: four sweeps held in
            -- reserve, and a wait between them.
            max_energy = 200, energy_regen = "0.2",
        },
        dying = { time = 2 },
        price = { gold = 50, wood = 50 },
        build_time = 80,
        skills = { "scanner_sweep" },
        tags = { "building" },
        annex = { alone = { work = "idles", life = "endures" }, claim = "seized" },
    })

    -- The wraith: a fighter that flies, shoots at anything, and hides in
    -- plain sight for as long as its energy holds.
    define_entity("wraith", {
        race = "terran",
        location = { occupation = AIR, size = 1, solidity = "solid" },
        stats = {
            speed = "0.45", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 3, max_health = 120,
            damage = 8, attack_range = 5, acquire_range = 7, attack_period = 12, damage_point = 4,
            sight_range = 9,
            max_energy = 200, energy_regen = "0.1",
            supply_cost = 2,
        },
        dying = { time = 2 },
        tags = { "mechanical" },
        attack = { targets = GROUND | WATER | AIR },
        price = { gold = 150, wood = 100 },
        train_time = 90,
        skills = { "cloak", "decloak" },
        selection = { priority = 10 },
    })

    -- The turret's rack: a gun that bears on its own, the way a tower aims
    -- without turning its walls, and reaches only the air.
    define_turret("missile_rack", {
        targets = AIR,
        projectile = "arrow",
    })
    -- The missile turret: the terran answer to what flies and to what hides.
    -- It shoots into the air alone, and detects a little short of where it
    -- sees.
    define_entity("missile_turret", {
        race = "terran",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            max_health = 200, armor = 1,
            damage = 12, attack_range = 7, acquire_range = 8, attack_period = 15, damage_point = 5,
            sight_range = 9,
        },
        dying = { time = 2 },
        price = { gold = 75, wood = 25 },
        build_time = 60,
        berths = { rim = { points = rim(1, 1), slots = 1 } },
        tags = { "building" },
        turrets = {
            { turret = "missile_rack" },
        },
        field_sources = {
            { field = "true_sight", radius = 7, growth = "instant", while_constructing = "nothing", while_disabled = "nothing" },
        },
    })

    -- The tech lab: the annex that researches. It keeps its footing without a
    -- primary — a lab is a lab — but does nothing at all until a factory
    -- stands beside it again.
    define_entity("tech_lab", {
        race = "terran",
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        stats = { max_health = 300, sight_range = 5 },
        dying = { time = 2 },
        price = { gold = 50, wood = 25 },
        build_time = 80,
        researcher = { "siege_tech" },
        tags = { "building" },
        annex = { alone = { work = "idles", life = "endures" }, claim = "seized" },
    })

    define_entity("supply_depot", {
        race = "terran",
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        stats = { max_health = 200, sight_range = 3, supply_provided = 6 },
        dying = { time = 2 },
        price = { gold = 40, wood = 20 },
        build_time = 60,
        tags = { "building" },
        berths = { rim = { points = rim(2, 2), slots = 1 } },
    })

    -- Raised over a gold mine and mined in its place: an SCV walks inside and
    -- comes out loaded, and the mine is handed back with whatever is left if
    -- the refinery falls.
    define_entity("refinery", {
        race = "terran",
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        stats = { max_health = 500, sight_range = 4 },
        dying = { time = 2 },
        price = { gold = 75 },
        build_time = 90,
        tags = { "building" },
        resource_source = { kind = "gold", depletion = "persist" },
        overbuilds = "gold_mine",
        -- The `rim` is where an SCV stands to *raise* it. Working the gold
        -- needs no seat: the SCV goes inside, and it is the only gold source
        -- an SCV may work at all.
        berths = { rim = { points = rim(2, 2), slots = 1 } },
    })

    define_entity("marine", {
        race = "terran",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 2, max_health = 45,
            damage = 6, attack_range = 4, acquire_range = 7, attack_period = 8, damage_point = 3,
            sight_range = 8,
            supply_cost = 1,
            cargo_size = 1,
        },
        dying = FALLS,
        tags = { "biological" },
        -- A rifle: the shot lands the tick it is fired, with nothing to
        -- outrun and nothing to dodge.
        attack = { targets = GROUND | WATER | AIR },
        price = { gold = 60 },
        train_time = 45,
    })

    -- The tank: the demo's other 2x2 gun, and the counterpart to the orc
    -- war wagon. The wagon carries a turret, so its gun bears while the hull
    -- keeps its heading; the tank's gun is the hull's own, in an arc of twenty
    -- degrees, so it must come about to answer anything — and a tank caught
    -- broadside holds its fire until it has.
    --
    -- Sieged it plants itself: it gives up its engine for twice the reach and
    -- twice the shell, and the change is the ancients' rooting — reserving the
    -- ground going down, revalidating it coming back up. Only a factory with
    -- a tech lab builds one, and only siege mechanics let it dig in.
    local function tank(name, extra)
        local def = {
            race = "terran",
            location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
            dying = { time = 2 },
            tags = { "mechanical" },
            selection = { priority = 10 },
        }
        for key, value in pairs(extra) do def[key] = value end
        define_entity(name, def)
    end
    tank("tank", {
        stats = {
            speed = "0.18", turn_rate = 9, pivot_rate = 12, pivot_angle = 90,
            radius = "1", weight = 6, max_health = 160, armor = 1, sight_range = 9,
            damage = 20, attack_range = 6, acquire_range = 8, attack_period = 26, damage_point = 10,
            -- The gun is the hull: it comes about at the hull's own pace, and
            -- fires only through a narrow arc ahead of it.
            attack_arc = 20,
            supply_cost = 2,
        },
        -- The gun leaves what it kills: only the planted form shells a body
        -- to nothing, which is what makes planting the answer to raised dead.
        attack = { targets = GROUND | WATER },
        price = { gold = 150, wood = 100 },
        train_time = 100,
        requires = { { annexed = "tech_lab" } },
        morphs = {
            { into = "siege_tank", time = 60, placement = "reserve", cancel = "committed",
              requires = { { research = "siege_tech" } } },
        },
    })

    tank("siege_tank", {
        stats = {
            max_health = 160, armor = 1, sight_range = 11,
            -- Planted, the gun traverses: no arc, so it answers whatever comes
            -- into its reach from any side.
            -- Its reach outruns its eyes on purpose: planted, it shells
            -- ground it cannot see, and wants something of its own out front
            -- to spot for it.
            damage = 40, attack_range = 12, acquire_range = 13, attack_period = 40, damage_point = 16,
            supply_cost = 2,
        },
        attack = {
            targets = GROUND | WATER,
            slain = "nothing",
            -- A ring of blast around the hit: half of it one cell out, a
            -- quarter two, and it does not spare its own.
            splash = {
                shape = "circular",
                bands = { {1, "0.5"}, {2, "0.25"} },
                layers = GROUND | WATER,
                friendly_fire = true,
            },
        },
        -- Nothing builds or trains a planted tank, so it carries what the
        -- player paid to have one standing: the tank's price, and the pace of
        -- the training and the digging in together. That is what an SCV mends
        -- it against.
        price = { gold = 150, wood = 100 },
        train_time = 160,
        morphs = {
            { into = "tank", time = 60, placement = "revalidate", cancel = "committed" },
        },
    })
    --
    -- ─── The Undead ───────────────────────────────────────────────────────────
    --
    -- The sixth race, and the one that makes use of what the other five leave
    -- behind. Its ground is blight, spread by every structure it raises and
    -- needed by all but the hall and the mine; its gold comes from a mine
    -- raised over the seam with five acolytes seated round it, banking where
    -- they stand; its wood is cut by the ghoul, which fights when it is not
    -- carrying; its supply comes from ziggurats that harden into towers; and
    -- its necromancers raise skeletons out of corpses — anyone's — that stand
    -- for forty-five seconds and then fall apart.

    -- Every undead structure stands on blight and spreads its own, so a base
    -- grows its ground outward as it is built. The hall and the mine are the
    -- exceptions: they make blight where there is none.
    local ON_BLIGHT = { requires = "blight", of = "anyone", coverage = "every" }
    local function blights(radius)
        return { { field = "blight", radius = radius, growth = "instant", while_constructing = "nothing", while_disabled = "full" } }
    end
    -- The dead mend only on their own ground: off blight nothing knits.
    local HEALS_ON_BLIGHT = { field = "blight", of = "anyone", coverage = "any", inside = {
        modifiers = { { entity_stat = "health_regen", op = "flat", value = "0.1" } },
    } }
    define_entity("acolyte", {
        race = "undead",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 1, max_health = 40, sight_range = 4,
            build_range = 1, harvest_range = 1,
            health_regen = "0",
            supply_cost = 1,
            cargo_size = 1,
        },
        dying = FALLS,
        price = { gold = 50 },
        train_time = 40,
        -- It summons a structure and walks away; the site finishes on its own.
        builder = {
            builds = { "necropolis", "haunted_mine", "ziggurat", "crypt", "graveyard", "temple_of_the_damned" },
            attendance = "unattended",
        },
        tags = { "biological" },
        field_effects = { HEALS_ON_BLIGHT },
        -- Ten gold every five seconds, straight to the stockpile, from the rim
        -- of a haunted mine and nowhere else: a bare seam seats nobody.
        resource_carrier = {
            gold = {
                capacity = 10, time = 100, banking = "direct",
                sources = { types = { "haunted_mine" } },
                presence = { attached = { berths = "crypt", stance = "still" } },
            },
        },
    })

    define_entity("ghoul", {
        race = "undead",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.35", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 2, max_health = 60,
            damage = 8, attack_range = 1, acquire_range = 6, attack_period = 8, damage_point = 4,
            sight_range = 6, harvest_range = 1,
            health_regen = "0",
            supply_cost = 1,
            cargo_size = 1,
        },
        dying = FALLS,
        price = { gold = 80 },
        train_time = 50,
        tags = { "biological" },
        attack = { targets = GROUND | WATER },
        field_effects = { HEALS_ON_BLIGHT },
        -- A ghoul carrying wood is a ghoul not fighting: a running order is
        -- what keeps it out of a fight it did not pick.
        resource_carrier = {
            wood = { capacity = 20, time = 20, presence = { present = { crew = 1 } } },
        },
    })

    -- What comes out of a body: free, costing no supply, standing forty-five
    -- seconds unless the research lengthens it, and leaving nothing when it
    -- falls apart.
    define_entity("skeleton", {
        race = "undead",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.3", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 2, max_health = 50,
            damage = 7, attack_range = 1, acquire_range = 6, attack_period = 14, damage_point = 6,
            sight_range = 5,
            health_regen = "0",
            lifetime = 900,
            cargo_size = 1,
        },
        dying = { time = 2 },
        attack = { targets = GROUND | WATER },
        field_effects = { HEALS_ON_BLIGHT },
    })

    -- The reach is the skill's own, as its price is: told to raise a body
    -- across the field, a necromancer walks to six cells of it and then casts.
    -- Raising the dead is not instant: the necromancer works at it for a
    -- second, and is another half-second putting his arms down. Cut the order
    -- short before the bodies rise and the energy is not spent.
    define_skill("raise_dead", {
        caster = "entity",
        cooldown = 160,
        cost = { energy = "40" },
        -- Bodies, and bodies only: the filter names what it raises from rather
        -- than taking whatever the field happens to leave lying.
        target = { kind = "fallen", only = { types = { "corpse" } } },
        range = 6,
        cast = { point = 30, period = 45 },
        effect = { summon = { entity = "skeleton", count = 2 } },
    })
    define_player_buff("skeletal_longevity", {
        stack = "ignore",
        entity_modifiers = { { entity_stat = "lifetime", op = "flat", value = "300" } },
    })
    define_research("skeletal_longevity", {
        price = { gold = 100 },
        time = 200,
        buff = "skeletal_longevity",
    })

    define_entity("necromancer", {
        race = "undead",
        location = { occupation = GROUND, size = 1, solidity = "solid" },
        stats = {
            speed = "0.28", turn_rate = 30, pivot_rate = 30, radius = "0.5", weight = 2, max_health = 45,
            damage = 5, attack_range = 4, acquire_range = 6, attack_period = 20, damage_point = 8,
            sight_range = 8,
            max_energy = 100, energy_regen = "0.3",
            health_regen = "0",
            supply_cost = 2,
            cargo_size = 1,
        },
        dying = FALLS,
        price = { gold = 100 },
        train_time = 60,
        tags = { "biological" },
        attack = { targets = GROUND | WATER, projectile = "arrow" },
        skills = { "raise_dead" },
        field_effects = { HEALS_ON_BLIGHT },
    })

    -- The hall, in three forms. Each grows into the next through an interim
    -- form that trains nothing, so the hall is silent while it rises; a change
    -- ordered with acolytes queued is refused until the queue is empty.
    local function hall(name, into, stats, extra)
        local def = {
            race = "undead",
            location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
            stats = stats,
            dying = { time = 2 },
            tags = { "building" },
            trainer = { "acolyte" },
            resource_storage = { "wood" },
            field_sources = blights(6),
        }
        for key, value in pairs(extra or {}) do def[key] = value end
        if into then
            local rising = into .. "_rising"
            def.morphs = { { into = into, via = rising, time = 120, placement = "revalidate", cancel = "refundable",
                             cost = { resources = { gold = 150 } } } }
            -- The hall rising is still the place wood is carried to, so a
            -- ghoul with a load does not wait out the change. It wears the
            -- tags of the hall it grew from, so a tier already reached stays
            -- reached while the next one rises, and a tier is reached when
            -- its growth finishes rather than when it starts.
            define_entity(rising, {
                race = "undead",
                location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
                stats = stats,
                dying = { time = 2 },
                tags = def.tags,
                resource_storage = { "wood" },
                field_sources = blights(6),
            })
        end
        define_entity(name, def)
    end
    hall("necropolis", "halls_of_the_dead",
        { max_health = 700, sight_range = 9, supply_provided = 10 },
        { cost = { gold = 350 }, build_time = 180 })
    -- A grown hall is never built, so it carries the necropolis's price and
    -- every growth paid since, over the raising and the growing together.
    hall("halls_of_the_dead", "black_citadel",
        { max_health = 900, sight_range = 10, supply_provided = 10 },
        { tags = { "building", "grown_hall" },
          price = { gold = 500 }, build_time = 300 })
    -- The citadel is the one hall that answers for itself: bolts at whatever
    -- comes into its reach, in the air as readily as on the ground.
    hall("black_citadel", nil,
        { max_health = 1100, sight_range = 11, supply_provided = 10,
          damage = 18, attack_range = 8, acquire_range = 9, attack_period = 22, damage_point = 9 },
        { tags = { "building", "grown_hall" },
          price = { gold = 650 }, build_time = 420,
          attack = { targets = GROUND | WATER | AIR, projectile = "arrow" } })

    -- Raised over a gold seam, the haunted mine is the mine from then on: it
    -- takes the gold that was left, seats five acolytes round its rim, and
    -- gives the seam back with whatever remains if it is torn down. A mine is
    -- where the gold is, so it needs no blight — and makes its own.
    define_entity("haunted_mine", {
        race = "undead",
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        stats = { max_health = 600, sight_range = 6 },
        dying = { time = 2 },
        price = { gold = 100 },
        build_time = 100,
        tags = { "building" },
        resource_source = { kind = "gold", depletion = "destroy" },
        overbuilds = "gold_mine",
        -- Five spots in a star about the mine's middle, two on its own ground
        -- and three a half-cell outside it, so five acolytes at work stand
        -- the points of one.
        berths = { crypt = { points = {
            { "1.0", "2.2" }, { "-0.1", "1.4" }, { "0.3", "0.0" }, { "1.7", "0.0" }, { "2.1", "1.4" },
        }, slots = 5 } },
        field_sources = blights(4),
    })

    -- Supply that hardens: a ziggurat feeds ten, and for a hundred gold it
    -- becomes a tower that still feeds ten and shoots. The spirit tower
    -- answers everything; the nerubian one is the heavier gun, and flat.
    define_entity("ziggurat", {
        race = "undead",
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        stats = { max_health = 300, sight_range = 6, supply_provided = 10 },
        dying = { time = 2 },
        price = { gold = 80, wood = 30 },
        build_time = 100,
        tags = { "building" },
        field_placement = { ON_BLIGHT },
        field_sources = blights(4),
        morphs = {
            { into = "spirit_tower", time = 70, placement = "revalidate", cancel = "refundable",
              cost = { resources = { gold = 100 } },
              requires = { { entity_type = "graveyard" } } },
            { into = "nerubian_tower", time = 70, placement = "revalidate", cancel = "refundable",
              cost = { resources = { gold = 120, wood = 40 } },
              requires = { { entity_type = "graveyard" } } },
        },
    })
    -- A tower is only ever reached by hardening a ziggurat, so it carries the
    -- ziggurat's price and the hardening's on top of it, over the raising and
    -- the hardening together.
    local function tower(name, stats, attack, price, build_time, field_sources)
        define_entity(name, {
            race = "undead",
            location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
            stats = stats,
            dying = { time = 2 },
            tags = { "building" },
            attack = attack,
            price = price,
            build_time = build_time,
            field_placement = { ON_BLIGHT },
            field_sources = field_sources,
        })
    end
    -- The spirit tower spreads blight and sees through cloaks: the undead's
    -- one detector, as the ghostly eye it is.
    tower("spirit_tower",
        { max_health = 400, sight_range = 8, supply_provided = 10,
          damage = 14, attack_range = 7, acquire_range = 8, attack_period = 18, damage_point = 7 },
        { targets = GROUND | WATER | AIR, projectile = "arrow" },
        { gold = 180, wood = 30 }, 170,
        { { field = "blight", radius = 4, growth = "instant", while_constructing = "nothing", while_disabled = "full" },
          { field = "true_sight", radius = 7, growth = "instant", while_constructing = "nothing", while_disabled = "nothing" } })
    tower("nerubian_tower",
        { max_health = 500, sight_range = 8, supply_provided = 10,
          damage = 22, attack_range = 6, acquire_range = 7, attack_period = 24, damage_point = 10 },
        { targets = GROUND | WATER, projectile = "cannonball" },
        { gold = 200, wood = 70 }, 170,
        blights(4))

    define_entity("crypt", {
        race = "undead",
        location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 500, sight_range = 6 },
        dying = { time = 2 },
        price = { gold = 200, wood = 50 },
        build_time = 120,
        tags = { "building" },
        trainer = { "ghoul" },
        field_placement = { ON_BLIGHT },
        field_sources = blights(4),
    })

    -- The graveyard unlocks the towers, and nothing else: the bodies it is
    -- named for come from the field.
    define_entity("graveyard", {
        race = "undead",
        location = { occupation = GROUND, size = { 2, 2 }, solidity = "solid" },
        stats = { max_health = 300, sight_range = 5 },
        dying = { time = 2 },
        price = { gold = 120, wood = 40 },
        build_time = 90,
        tags = { "building" },
        field_placement = { ON_BLIGHT },
        field_sources = blights(4),
    })

    define_entity("temple_of_the_damned", {
        race = "undead",
        location = { occupation = GROUND, size = { 3, 3 }, solidity = "solid" },
        stats = { max_health = 450, sight_range = 6 },
        dying = { time = 2 },
        price = { gold = 250, wood = 100 },
        build_time = 140,
        tags = { "building" },
        trainer = { "necromancer" },
        researcher = { "skeletal_longevity" },
        requires = { { tag = "grown_hall" } },
        field_placement = { ON_BLIGHT },
        field_sources = blights(4),
    })

"#;

/// Loads all demo content from Lua into the registry, then validates it. Runs at
/// startup; a content error is a bug in the script above, so it panics.
pub fn register_all(mut registry: ResMut<ContentRegistry>) {
    *registry = content::load(&LuaEngine, CONTENT).expect("demo content must load");
}
