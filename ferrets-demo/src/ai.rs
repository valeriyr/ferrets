//! The demo's AIs: one dedicated economy-then-army brain per race, sharing a
//! common Lua prelude, authored in Lua and installed for every AI slot this
//! node computes.
//!
//! The scripts are deterministic (integer arithmetic, `ipairs` over the
//! ordered view arrays only), so they are valid under either AI hosting mode.

use std::collections::BTreeMap;

use bevy::prelude::*;
use ferrets_bevy_plugin::ai::{AiRuntimes, install_ai_runtimes, sourced_ai_players};
use ferrets_content::registry::ContentRegistry;
use ferrets_script::{
    ai::view::content::ContentView,
    engine::{ScriptEngine, lua::LuaEngine},
};
use ferrets_simulation::session::{
    GameSession, ai_detection::AiDetection, ai_vision::AiVision, player_id::PlayerId,
};

/// The chassis every race brain runs on: pure helpers plus the economy, build,
/// research, and attack routines. Prepended to each brain, so its locals are
/// in scope for the race's `define_ai`. Routines that spend take a mutable
/// `budget` table (`gold`, `wood`, `supply`) so one think never over-commits
/// the stockpile.
const COMMON_AI: &str = r#"
    local MAX_WORKERS = 5
    local ARMY_ATTACK_AT = 8
    local MAX_QUEUE = 2

    -- Candidate structure cells relative to the hall's origin, tried in turn.
    local OFFSETS = { { 0, 4 }, { 4, 4 }, { -4, 0 }, { 0, -4 }, { 4, -4 } }

    local function cost_of(type_name, kind)
        for _, entry in ipairs(content.entities[type_name].price) do
            if entry.kind == kind then return entry.amount end
        end
        return 0
    end

    local function afford(budget, type_name)
        return budget.gold >= cost_of(type_name, "gold")
            and budget.wood >= cost_of(type_name, "wood")
    end

    local function pay(budget, type_name)
        budget.gold = budget.gold - cost_of(type_name, "gold")
        budget.wood = budget.wood - cost_of(type_name, "wood")
    end

    local function contains(list, name)
        for _, entry in ipairs(list) do
            if entry == name then return true end
        end
        return false
    end

    local function count_queued(entities, type_name)
        local queued = 0
        for _, e in ipairs(entities) do
            for _, name in ipairs(e.train_queue) do
                if name == type_name then queued = queued + 1 end
            end
        end
        return queued
    end

    -- The accepted candidate nearest to `from` by squared cell distance;
    -- earlier (lower-id) candidates win ties.
    local function nearest(from, candidates, accept)
        local best, best_distance = nil, nil
        for _, e in ipairs(candidates) do
            if accept == nil or accept(e) then
                local dx, dy = e.x - from.x, e.y - from.y
                local distance = dx * dx + dy * dy
                if best == nil or distance < best_distance then
                    best, best_distance = e, distance
                end
            end
        end
        return best
    end

    local function within(a, b, cells)
        local dx, dy = a.x - b.x, a.y - b.y
        return dx * dx + dy * dy <= cells * cells
    end

    -- My entities split by type name; `group` reads a split with a default.
    local function muster(view)
        local groups = {}
        for _, e in ipairs(view.my_entities) do
            local list = groups[e.type_name]
            if list == nil then
                list = {}
                groups[e.type_name] = list
            end
            list[#list + 1] = e
        end
        return groups
    end

    local function group(groups, name)
        return groups[name] or {}
    end

    local function any_standing(list)
        for _, e in ipairs(list) do
            if not e.under_construction then return true end
        end
        return false
    end

    local function budget_of(view)
        return {
            gold = view.resources.gold or 0,
            wood = view.resources.wood or 0,
            supply = view.supply.provided - view.supply.used,
        }
    end

    -- Keeps the worker line going from the hall.
    local function keep_workers(commands, budget, hall, halls, workers, worker_type)
        if hall ~= nil and not hall.under_construction
            and #workers + count_queued(halls, worker_type) < MAX_WORKERS
            and #hall.train_queue < MAX_QUEUE
            and budget.supply >= 1
            and afford(budget, worker_type) then
            commands[#commands + 1] =
                { kind = "train", trainer = hall.id, type_name = worker_type }
            pay(budget, worker_type)
            budget.supply = budget.supply - 1
        end
    end

    -- Whether an annex standing in the candidate structure's dock would have
    -- room at `(x, y)`. `annex` names the dock's offset and what stands in it,
    -- or is nil for a structure that docks nothing.
    --
    -- The hall is the one building certain to stand beside a candidate cell, so
    -- it is what the annex is held against: a dock on the far side of a
    -- structure sited to the hall's left points back at the hall, and the gap
    -- between them is narrower than the annex needs.
    local function fits_annex(view, hall, x, y, annex)
        if annex == nil then return true end
        local annex_size = content.entities[annex.type_name].size
        local hall_size = content.entities[hall.type_name].size
        local annex_x = x + annex.dx
        local annex_y = y + annex.dy
        if annex_x < 0 or annex_y < 0
            or annex_x + annex_size.w > view.map.width
            or annex_y + annex_size.h > view.map.height then
            return false
        end
        -- Clear of the hall on one axis or the other.
        return annex_x >= hall.x + hall_size.w
            or hall.x >= annex_x + annex_size.w
            or annex_y >= hall.y + hall_size.h
            or hall.y >= annex_y + annex_size.h
    end

    -- Puts up `wanted` (one structure at a time) at ring offsets around the
    -- hall. In flight means a site is visibly going up, or a builder was sent
    -- recently and may still be walking — a deadline, not a builder watch,
    -- because a past builder gone back to harvesting never reads idle again.
    -- An invalid placement is a silent no-op that leaves no site behind, so
    -- when the deadline lapses the offset ring advances to the next candidate.
    -- Returns the chosen builder's id.
    local function build_next(commands, state, view, workers, hall, wanted, budget, annex)
        for _, e in ipairs(view.my_entities) do
            if e.under_construction then return nil end
        end
        if state.build_deadline ~= nil and view.tick < state.build_deadline then
            return nil
        end
        if wanted == nil or hall == nil or not afford(budget, wanted) then
            return nil
        end

        local builder = nil
        for _, w in ipairs(workers) do
            if w.idle and not w.hidden then builder = w break end
        end
        builder = builder or workers[1]
        if builder == nil then return nil end

        for _ = 1, #OFFSETS do
            local offset = OFFSETS[state.build_offset or 1]
            state.build_offset = (state.build_offset or 1) % #OFFSETS + 1
            local x = hall.x + offset[1]
            local y = hall.y + offset[2]
            -- Three cells for the widest structure, and two more beyond it so
            -- there is room on the far side for whatever it docks.
            if x >= 0 and y >= 0
                and x + 5 <= view.map.width and y + 5 <= view.map.height
                and fits_annex(view, hall, x, y, annex) then
                commands[#commands + 1] = {
                    kind = "build", builder = builder.id,
                    type_name = wanted, x = x, y = y,
                }
                -- Ten seconds to walk there and place before a retry.
                state.build_deadline = view.tick + 200
                return builder.id
            end
        end
        return nil
    end

    -- Idle workers gather gold; the first one fetches wood while `need_wood`.
    local function assign_harvesters(commands, view, workers, need_wood, builder_id)
        for _, w in ipairs(workers) do
            if w.idle and not w.hidden and w.id ~= builder_id then
                local target = nil
                if need_wood then
                    target = nearest(w, view.neutral_entities, function(e)
                        return e.type_name == "tree" and (e.resource_amount or 0) > 0
                    end)
                    need_wood = false
                end
                if target == nil then
                    target = nearest(w, view.neutral_entities, function(e)
                        return e.type_name == "gold_mine" and (e.resource_amount or 0) > 0
                    end)
                end
                if target ~= nil then
                    commands[#commands + 1] = { kind = "select", id = w.id }
                    commands[#commands + 1] = { kind = "send", target = target.id }
                end
            end
        end
    end

    -- Queues one `type_name` on `building` when the budget allows. Returns
    -- whether the order was placed.
    local function train_from(commands, budget, building, type_name)
        if not building.under_construction and not building.disabled
            and #building.train_queue < MAX_QUEUE
            and budget.supply >= 1
            and afford(budget, type_name) then
            commands[#commands + 1] =
                { kind = "train", trainer = building.id, type_name = type_name }
            pay(budget, type_name)
            budget.supply = budget.supply - 1
            return true
        end
        return false
    end

    -- Starts `research` at the first standing host when the budget covers it;
    -- while it cannot, its price stays earmarked — training may only spend the
    -- surplus, so the stockpile climbs toward the upgrade instead of being
    -- drunk by the army. A command whose requirements are unmet is refused
    -- before payment, so retrying every think costs nothing.
    local function buy_research(commands, budget, view, research, hosts)
        if contains(view.researched, research)
            or contains(view.researching, research) then
            return
        end
        if not any_standing(hosts) then return end
        local price = content.researches[research].price
        local gold, wood = price.gold or 0, price.wood or 0
        if budget.gold >= gold and budget.wood >= wood then
            for _, host in ipairs(hosts) do
                if not host.under_construction then
                    commands[#commands + 1] =
                        { kind = "research", researcher = host.id, research = research }
                    break
                end
            end
        end
        budget.gold = budget.gold - gold
        budget.wood = budget.wood - wood
    end

    -- Earmarks a wanted type's price the same way, so what is cheap does not
    -- race what is dear to the stockpile — nil reserves nothing.
    local function reserve(budget, wanted)
        if wanted ~= nil then
            budget.gold = budget.gold - cost_of(wanted, "gold")
            budget.wood = budget.wood - cost_of(wanted, "wood")
        end
    end

    -- The stockpile price of `type_name` changing into `into`, per kind.
    local function morph_cost_of(type_name, into, kind)
        for _, morph in ipairs(content.entities[type_name].morphs or {}) do
            if morph.into == into then
                for _, entry in ipairs(morph.price) do
                    if entry.kind == kind then return entry.amount end
                end
                return 0
            end
        end
        return 0
    end

    -- Orders `unit` to change into `into` when the budget covers it, busy or
    -- not: a fighter is pulled out of its march to grow. Returns whether the
    -- order was placed.
    local function morph_any(commands, budget, unit, into)
        local gold = morph_cost_of(unit.type_name, into, "gold")
        local wood = morph_cost_of(unit.type_name, into, "wood")
        if not unit.hidden and budget.gold >= gold and budget.wood >= wood then
            commands[#commands + 1] = { kind = "select", id = unit.id }
            commands[#commands + 1] = { kind = "morph", type_name = into }
            budget.gold = budget.gold - gold
            budget.wood = budget.wood - wood
            return true
        end
        return false
    end

    -- Like morph_any, but leaves a busy unit to its work.
    local function morph_one(commands, budget, unit, into)
        if not unit.idle then return false end
        return morph_any(commands, budget, unit, into)
    end

    -- Once the wave is big enough, pushes it out: fighters attack-move onto
    -- the nearest enemy in sight — or, with fog hiding every enemy, toward the
    -- far side of the map to scout one out — and escorts (healers) walk along.
    -- Returns whether anything marched.
    local function attack_wave(commands, view, fighters, walkers, hall)
        if #fighters < ARMY_ATTACK_AT then return false end
        local scout_x, scout_y
        if hall ~= nil then
            scout_x = view.map.width - 1 - hall.x
            scout_y = view.map.height - 1 - hall.y
        end
        local marched = false
        local send = function(unit, kind)
            local target = nearest(unit, view.enemy_entities)
            local tx, ty
            if target ~= nil then
                tx, ty = target.x, target.y
            else
                tx, ty = scout_x, scout_y
            end
            if tx ~= nil then
                commands[#commands + 1] = { kind = "select", id = unit.id }
                commands[#commands + 1] = { kind = kind, x = tx, y = ty }
                marched = true
            end
        end
        -- A garrisoned fighter reads idle (no orders while aboard) but cannot
        -- march; it holds its post instead.
        for _, f in ipairs(fighters) do
            if f.idle and not f.hidden then send(f, "attack_move") end
        end
        -- Walkers are sent, not set on anything: a healer has nothing to attack
        -- with, and a gun that shoots while it rolls works what it passes on the
        -- way without being told to stop for it.
        for _, e in ipairs(walkers) do
            if e.idle and not e.hidden then send(e, "move") end
        end
        return marched
    end
"#;

/// The human brain: peasant economy, training camps, then the blacksmith for the
/// iron weapons upgrade and the mortars it unlocks, then a bunker a pair of
/// archers mans for base defense; a medic walks with every few archers,
/// archers burn energy on battle focus when a foe is in reach, and war drums
/// sound as a wave marches.
const HUMAN_AI: &str = r#"
    define_ai("human", {
        period = 20,
        vision = "filtered",
        detection = "detectors",
        think = function(state, view)
            local commands = {}
            local budget = budget_of(view)
            local groups = muster(view)
            local halls = group(groups, "town_hall")
            local workers = group(groups, "peasant")
            local camps = group(groups, "training_camp")
            local smithies = group(groups, "blacksmith")
            local bunkers = group(groups, "bunker")
            local archers = group(groups, "archer")
            local mortars = group(groups, "mortar")
            local medics = group(groups, "medic")
            local hall = halls[1]

            keep_workers(commands, budget, hall, halls, workers, "peasant")

            -- The camps, then the forge that unlocks mortars and hosts the
            -- weapon upgrade — a one-time purchase farms would otherwise
            -- always outbid — then a farm whenever headroom runs dry, and
            -- once the army production is fed, the bunker the defense mans.
            local wanted = nil
            if #camps == 0 then
                wanted = "training_camp"
            elseif #smithies == 0 then
                wanted = "blacksmith"
            elseif budget.supply < 2 then
                wanted = "farm"
            elseif #bunkers == 0 then
                wanted = "bunker"
            end
            local builder_id =
                build_next(commands, state, view, workers, hall, wanted, budget)
            -- Wood feeds whatever structure is pending and the upgrade after it.
            local need_wood = (wanted ~= nil and budget.wood < cost_of(wanted, "wood"))
                or (not contains(view.researched, "iron_weapons") and budget.wood < 50)
            assign_harvesters(commands, view, workers, need_wood, builder_id)

            -- The upgrade and the pending structure hold their price back from
            -- the army before any unit is queued.
            buy_research(commands, budget, view, "iron_weapons", smithies)
            reserve(budget, wanted)

            -- Army mix: a medic per four archers, a pair of mortars once the
            -- forge stands (they require it), archers otherwise.
            for _, b in ipairs(camps) do
                local trained = "archer"
                if (#medics + count_queued(camps, "medic")) * 4
                    < #archers + count_queued(camps, "archer") then
                    trained = "medic"
                elseif any_standing(smithies)
                    and #mortars + count_queued(camps, "mortar") < 2 then
                    trained = "mortar"
                end
                if train_from(commands, budget, b, trained) then break end
            end

            -- Man the bunker: once the army has archers to spare, two step
            -- inside and fire their own bows out, untouchable while it
            -- stands; the rest form the wave. Boarded explicitly — a smart
            -- send onto a damaged bunker would read as a repair intent.
            local bunker = bunkers[1]
            if bunker ~= nil and not bunker.under_construction and #archers > 2 then
                local manned = #bunker.passengers
                for _, a in ipairs(archers) do
                    if manned >= 2 then break end
                    if a.idle and not a.hidden then
                        commands[#commands + 1] = { kind = "select", id = a.id }
                        commands[#commands + 1] = { kind = "board", target = bunker.id }
                        manned = manned + 1
                    end
                end
            end

            -- Battle focus: an archer with a foe in reach burns its energy on
            -- the damage burst; a cast still cooling down is refused for free.
            for _, a in ipairs(archers) do
                if (a.energy or 0) >= 30 then
                    local foe = nearest(a, view.enemy_entities)
                    if foe ~= nil and within(a, foe, 7) then
                        commands[#commands + 1] =
                            { kind = "use_skill", skill = "battle_focus", caster = a.id }
                    end
                end
            end

            local fighters = {}
            for _, e in ipairs(archers) do fighters[#fighters + 1] = e end
            for _, e in ipairs(mortars) do fighters[#fighters + 1] = e end
            if attack_wave(commands, view, fighters, medics, hall) then
                -- War drums speed the wave out; refused while cooling or broke.
                commands[#commands + 1] =
                    { kind = "use_skill", skill = "war_drums", caster = "player" }
            end

            return commands
        end,
    })
"#;

/// The orc brain: peon economy, war camp, then the pig farm the frenzy ritual
/// waits on; shamans join the grunts once the ritual is in (they require it)
/// and mend the wounded, grunts buy frenzy with their own blood when a foe is
/// at the gates, peons crawl into a pig farm while raiders are near and come
/// back out to work once they leave, and war drums sound as a wave marches.
const ORC_AI: &str = r#"
    define_ai("orc", {
        period = 20,
        vision = "filtered",
        detection = "detectors",
        think = function(state, view)
            local commands = {}
            local budget = budget_of(view)
            local groups = muster(view)
            local halls = group(groups, "great_hall")
            local workers = group(groups, "peon")
            local camps = group(groups, "war_camp")
            local farms = group(groups, "pig_farm")
            local works = group(groups, "siege_works")
            local grunts = group(groups, "grunt")
            local shamans = group(groups, "shaman")
            local wagons = group(groups, "war_wagon")
            local watch_towers = group(groups, "watch_tower")
            local guard_towers = group(groups, "guard_tower")
            local hall = halls[1]

            keep_workers(commands, budget, hall, halls, workers, "peon")

            -- The war camp, a first pig farm right after it — the frenzy
            -- ritual waits on one — then a farm whenever headroom runs dry,
            -- once production is fed, the siege works the wagons come from (it
            -- requires the camp, so that part of the order is the content's),
            -- and two watch towers after that: one kept as the eyes that find
            -- a cloaked raider, the other hardened into the guard tower.
            local wanted = nil
            if #camps == 0 then
                wanted = "war_camp"
            elseif #farms == 0 then
                wanted = "pig_farm"
            elseif budget.supply < 2 then
                wanted = "pig_farm"
            elseif #works == 0 then
                wanted = "siege_works"
            elseif #watch_towers + #guard_towers < 2 then
                wanted = "watch_tower"
            end
            local builder_id =
                build_next(commands, state, view, workers, hall, wanted, budget)

            -- A tower is hardened only while another watcher stands: the gun
            -- that answers a flier costs the tower its detection, and the side
            -- keeps one watcher.
            if #watch_towers >= 2 and #guard_towers == 0 then
                for _, t in ipairs(watch_towers) do
                    if not t.under_construction and morph_one(commands, budget, t, "guard_tower") then
                        break
                    end
                end
            end
            local need_wood = (wanted ~= nil and budget.wood < cost_of(wanted, "wood"))
                or (#works > 0 and budget.wood < cost_of("war_wagon", "wood"))
            assign_harvesters(commands, view, workers, need_wood, builder_id)

            -- The ritual and the pending structure hold their price back from
            -- the army before any unit is queued.
            buy_research(commands, budget, view, "frenzy_ritual", camps)
            reserve(budget, wanted)

            -- A pair of wagons from the siege works, and their price held back
            -- from the camp: a wagon costs several grunts, and grunts queued
            -- against it would drink the stockpile every think it fell short.
            local wagon = nil
            if any_standing(works) and #wagons + count_queued(works, "war_wagon") < 2 then
                wagon = "war_wagon"
            end
            for _, w in ipairs(works) do
                if wagon ~= nil and train_from(commands, budget, w, wagon) then
                    wagon = nil
                    break
                end
            end
            reserve(budget, wagon)

            -- Army mix: grunts, and a shaman per four once the ritual is in
            -- (shamans require it).
            local ritual_done = contains(view.researched, "frenzy_ritual")
            for _, c in ipairs(camps) do
                local trained = "grunt"
                if ritual_done
                    and (#shamans + count_queued(camps, "shaman")) * 4
                        < #grunts + count_queued(camps, "grunt") then
                    trained = "shaman"
                end
                if train_from(commands, budget, c, trained) then break end
            end

            -- The farms shelter the workforce: a peon with a raider close by
            -- crawls into the nearest one with room and sits the raid out —
            -- and once no enemy is near a farm, whoever hides inside is let
            -- back out to work. Boarded explicitly: a raided farm is usually
            -- a damaged farm, and a smart send onto one would put the peon to
            -- work repairing it in the open instead of hiding inside.
            for _, w in ipairs(workers) do
                if not w.hidden then
                    local threat = nearest(w, view.enemy_entities)
                    if threat ~= nil and within(w, threat, 6) then
                        local shelter = nearest(w, farms, function(f)
                            return not f.under_construction and #f.passengers < 4
                        end)
                        if shelter ~= nil then
                            commands[#commands + 1] = { kind = "select", id = w.id }
                            commands[#commands + 1] = { kind = "board", target = shelter.id }
                        end
                    end
                end
            end
            for _, f in ipairs(farms) do
                if #f.passengers > 0 then
                    local threat = nearest(f, view.enemy_entities)
                    if threat == nil or not within(f, threat, 10) then
                        commands[#commands + 1] = { kind = "unload", transport = f.id }
                    end
                end
            end

            -- Blood rite: a healthy grunt with a foe at the gates buys frenzy
            -- with its own blood; before the ritual, while cooling, or too
            -- wounded to pay, the cast is refused for free.
            if ritual_done then
                for _, g in ipairs(grunts) do
                    if (g.health or 0) > 20 then
                        local foe = nearest(g, view.enemy_entities)
                        if foe ~= nil and within(g, foe, 5) then
                            commands[#commands + 1] =
                                { kind = "use_skill", skill = "blood_rite", caster = g.id }
                        end
                    end
                end
            end

            -- Second wind: each shaman mends the most battered ally in view
            -- that has lost half its health.
            for _, s in ipairs(shamans) do
                if (s.energy or 0) >= 20 then
                    local patient = nearest(s, view.my_entities, function(e)
                        local max = content.entities[e.type_name].max_health
                        return e.id ~= s.id and e.health ~= nil and max ~= nil
                            and e.health * 2 < max
                    end)
                    if patient ~= nil then
                        commands[#commands + 1] = {
                            kind = "use_skill", skill = "second_wind",
                            caster = s.id, target = patient.id,
                        }
                    end
                end
            end

            -- The shamans and the wagons walk with the wave: a wagon told to
            -- attack-move would stop at reach and fight like anything else, and
            -- the whole point of its mounted cannon is that it need not.
            local walkers = {}
            for _, e in ipairs(shamans) do walkers[#walkers + 1] = e end
            for _, e in ipairs(wagons) do walkers[#walkers + 1] = e end
            if attack_wave(commands, view, grunts, walkers, hall) then
                -- War drums speed the wave out; refused while cooling or broke.
                commands[#commands + 1] =
                    { kind = "use_skill", skill = "war_drums", caster = "player" }
            end

            return commands
        end,
    })
"#;

/// The swarm brain: larvae grown into drones while the worker line is short
/// and into an overlord whenever headroom runs dry, then drones spent on the
/// spawning pit that unlocks the swarmling and a tumor to walk the creep
/// outward; once the pit stands the hatchery grows into a hive and larvae
/// into swarmlings, a swarmling in four cocoons into a ravager, and the swarm
/// musters and marches as a wave.
const SWARM_AI: &str = r#"
    define_ai("swarm", {
        period = 20,
        vision = "filtered",
        detection = "detectors",
        think = function(state, view)
            local commands = {}
            local budget = budget_of(view)
            local groups = muster(view)
            local hatcheries = group(groups, "hatchery")
            local hive_cocoons = group(groups, "hive_cocoon")
            local hives = group(groups, "hive")
            local larvae = group(groups, "larva")
            local eggs = group(groups, "egg")
            local drones = group(groups, "drone")
            local pits = group(groups, "spawning_pit")
            local tumors = group(groups, "tumor")
            local swarmlings = group(groups, "swarmling")
            -- What a killed structure spills fights for the few seconds it
            -- has, rather than standing over the ruin waiting to be noticed.
            local hatchlings = group(groups, "hatchling")
            local cocoons = group(groups, "cocoon")
            local ravagers = group(groups, "ravager")
            local hall = hives[1] or hive_cocoons[1] or hatcheries[1]

            -- Grows one larva into `into`, a unit that takes `supply`, when
            -- the budget and the headroom allow; a larva already taken this
            -- think is passed over. Returns whether the order was placed.
            local taken = {}
            local function grow(into, supply)
                if budget.supply < supply then return false end
                for _, larva in ipairs(larvae) do
                    if not taken[larva.id] and morph_one(commands, budget, larva, into) then
                        taken[larva.id] = true
                        budget.supply = budget.supply - supply
                        return true
                    end
                end
                return false
            end

            -- Headroom first: with the supply nearly spent and no egg on the
            -- way, one larva becomes an overlord. Then the worker line: every
            -- egg is taken to be a drone on the way while drones are short.
            if budget.supply < 2 and #eggs == 0 then grow("overlord", 0) end
            if #drones + #eggs < MAX_WORKERS then grow("drone", 1) end

            -- The pit first, and once the army is fed a tumor that carries the
            -- creep outward. Each costs the drone that becomes it, which the
            -- worker line replaces.
            local wanted = nil
            if #pits == 0 then
                wanted = "spawning_pit"
            elseif #tumors == 0 then
                wanted = "tumor"
            end
            local builder_id =
                build_next(commands, state, view, drones, hall, wanted, budget)
            -- Wood is cut for what is next in line: the wanted structure,
            -- then the hive, then the ravagers the hive unlocks.
            local wood_wanted = 0
            if wanted ~= nil then wood_wanted = cost_of(wanted, "wood") end
            if any_standing(pits) then
                if #hives == 0 and #hive_cocoons == 0 then
                    wood_wanted = wood_wanted + morph_cost_of("hatchery", "hive", "wood")
                else
                    wood_wanted = wood_wanted + morph_cost_of("swarmling", "ravager", "wood")
                end
            end
            assign_harvesters(commands, view, drones, budget.wood < wood_wanted, builder_id)

            -- The pending structure holds its price back from the army.
            reserve(budget, wanted)

            -- The hall grows into a hive once the pit stands, its larvae
            -- riding the cocoon's berths across; until it can pay, the
            -- hive's price is held back from the army.
            if any_standing(pits) and #hives == 0 and #hive_cocoons == 0 then
                local grown = false
                for _, hatchery in ipairs(hatcheries) do
                    if morph_one(commands, budget, hatchery, "hive") then
                        grown = true
                        break
                    end
                end
                if not grown then
                    budget.gold = budget.gold - morph_cost_of("hatchery", "hive", "gold")
                    budget.wood = budget.wood - morph_cost_of("hatchery", "hive", "wood")
                end
            end

            -- One swarmling in four grows into a ravager, cocoons counting
            -- as ravagers on the way; the growth is refused for free before
            -- the hive stands.
            if any_standing(hives)
                and (#ravagers + #cocoons) * 4 < #swarmlings then
                for _, s in ipairs(swarmlings) do
                    if morph_any(commands, budget, s, "ravager") then break end
                end
            end

            if any_standing(pits) then grow("swarmling", 1) end

            local fighters = {}
            for _, e in ipairs(swarmlings) do fighters[#fighters + 1] = e end
            for _, e in ipairs(ravagers) do fighters[#fighters + 1] = e end
            for _, e in ipairs(hatchlings) do fighters[#fighters + 1] = e end
            attack_wave(commands, view, fighters, {}, hall)

            return commands
        end,
    })
"#;

/// The conclave brain: probe economy, a pylon for headroom and a gateway in
/// the nexus's own power, the probes placing each site and leaving it to warp
/// in on its own; a photon cannon guards the base once the army is fed, and
/// zealots muster and march as a wave.
const CONCLAVE_AI: &str = r#"
    define_ai("conclave", {
        period = 20,
        vision = "filtered",
        detection = "detectors",
        think = function(state, view)
            local commands = {}
            local budget = budget_of(view)
            local groups = muster(view)
            local halls = group(groups, "nexus")
            local probes = group(groups, "probe")
            local pylons = group(groups, "pylon")
            local gateways = group(groups, "gateway")
            local cannons = group(groups, "photon_cannon")
            local zealots = group(groups, "zealot")
            local hall = halls[1]

            keep_workers(commands, budget, hall, halls, probes, "probe")

            -- The gateway stands in the nexus's power, so it comes first; a
            -- pylon whenever headroom runs dry, and once production is fed the
            -- cannon.
            local wanted = nil
            if #gateways == 0 then
                wanted = "gateway"
            elseif budget.supply < 2 then
                wanted = "pylon"
            elseif #cannons == 0 then
                wanted = "photon_cannon"
            end
            local builder_id =
                build_next(commands, state, view, probes, hall, wanted, budget)
            local need_wood = wanted ~= nil and budget.wood < cost_of(wanted, "wood")
            assign_harvesters(commands, view, probes, need_wood, builder_id)

            -- The pending structure holds its price back from the army.
            reserve(budget, wanted)

            -- An observer once two zealots stand, so the army marches with
            -- an eye that sees through cloaks; zealots otherwise.
            local observers = group(groups, "observer")
            local next_unit = (#observers == 0 and #zealots >= 2) and "observer" or "zealot"
            for _, gateway in ipairs(gateways) do
                if train_from(commands, budget, gateway, next_unit) then break end
            end

            attack_wave(commands, view, zealots, {}, hall)

            return commands
        end,
    })
"#;

/// The elf brain: an entangled mine over the nearest gold mine first, wisps
/// drifting round its rim and sitting in the trees, and wisps spent on the
/// ancient of war the huntresses come from, a moon well whenever headroom runs
/// dry, then a protector; huntresses muster and march as a wave. Nothing
/// uproots: the walking forms are the player's to try.
const ELVES_AI: &str = r#"
    define_ai("elves", {
        period = 20,
        vision = "filtered",
        detection = "detectors",
        think = function(state, view)
            local commands = {}
            local budget = budget_of(view)
            local groups = muster(view)
            local trees = group(groups, "tree_of_life")
            local wisps = group(groups, "wisp")
            local mines = group(groups, "entangled_mine")
            local ancients = group(groups, "ancient_of_war")
            local wells = group(groups, "moon_well")
            local protectors = group(groups, "ancient_protector")
            local huntresses = group(groups, "huntress")
            local tree = trees[1]

            keep_workers(commands, budget, tree, trees, wisps, "wisp")

            -- The entangled mine comes first, over the gold mine nearest the
            -- hall: without it no wisp draws gold at all. It is placed on the
            -- mine's own cells rather than at a ring offset, with the same
            -- deadline discipline build_next keeps.
            local builder_id = nil
            local wanted = nil
            if #mines == 0 then
                local seam = tree and nearest(tree, view.neutral_entities, function(e)
                    return e.type_name == "gold_mine" and (e.resource_amount or 0) > 0
                end)
                local in_flight = false
                for _, e in ipairs(view.my_entities) do
                    if e.under_construction then in_flight = true end
                end
                if seam ~= nil and not in_flight
                    and (state.build_deadline == nil or view.tick >= state.build_deadline)
                    and afford(budget, "entangled_mine") then
                    local builder = nil
                    for _, w in ipairs(wisps) do
                        if w.idle and not w.hidden then builder = w break end
                    end
                    builder = builder or wisps[1]
                    if builder ~= nil then
                        commands[#commands + 1] = {
                            kind = "build", builder = builder.id,
                            type_name = "entangled_mine", x = seam.x, y = seam.y,
                        }
                        state.build_deadline = view.tick + 200
                        builder_id = builder.id
                    end
                end
                reserve(budget, "entangled_mine")
            else
                -- The ancient of war, a well whenever headroom runs dry, and
                -- once the army is fed a protector. Each costs the wisp that
                -- becomes it, which the worker line replaces.
                if #ancients == 0 then
                    wanted = "ancient_of_war"
                elseif budget.supply < 2 then
                    wanted = "moon_well"
                elseif #protectors == 0 then
                    wanted = "ancient_protector"
                end
                builder_id = build_next(commands, state, view, wisps, tree, wanted, budget)
            end

            -- Idle wisps sit on the entangled mine while it has room, one in a
            -- tree whenever anything wants wood.
            local need_wood = (wanted ~= nil and budget.wood < cost_of(wanted, "wood"))
                or (#ancients > 0 and budget.wood < cost_of("huntress", "wood"))
            for _, w in ipairs(wisps) do
                if w.idle and not w.hidden and w.id ~= builder_id then
                    local target = nil
                    if need_wood then
                        target = nearest(w, view.neutral_entities, function(e)
                            return e.type_name == "tree" and (e.resource_amount or 0) > 0
                        end)
                        need_wood = false
                    end
                    if target == nil then
                        target = nearest(w, mines, function(m)
                            return not m.under_construction and (m.resource_amount or 0) > 0
                        end)
                    end
                    if target ~= nil then
                        commands[#commands + 1] = { kind = "select", id = w.id }
                        commands[#commands + 1] = { kind = "send", target = target.id }
                    end
                end
            end

            -- The pending structure holds its price back from the army.
            reserve(budget, wanted)

            for _, a in ipairs(ancients) do
                if train_from(commands, budget, a, "huntress") then break end
            end

            attack_wave(commands, view, huntresses, {}, tree)

            return commands
        end,
    })
"#;

/// The terran brain: a refinery over the nearest seam — without one no SCV
/// draws gold at all — then a barracks for marines, depots for headroom, a
/// factory, and a tech lab docked to it so the factory may train tanks;
/// a comsat station on the command center, and marines with a tank per few of
/// them marching as a wave. It builds its annexes from the buildings that hold
/// their docks, and never lifts off: the flying forms are the player's to try.
const TERRAN_AI: &str = r#"
    define_ai("terran", {
        period = 20,
        vision = "filtered",
        detection = "detectors",
        think = function(state, view)
            local commands = {}
            local budget = budget_of(view)
            local groups = muster(view)
            local centers = group(groups, "command_center")
            local scvs = group(groups, "scv")
            local barracks = group(groups, "barracks")
            local factories = group(groups, "factory")
            local labs = group(groups, "tech_lab")
            local comsats = group(groups, "comsat_station")
            local depots = group(groups, "supply_depot")
            local refineries = group(groups, "refinery")
            local turrets = group(groups, "missile_turret")
            local marines = group(groups, "marine")
            local tanks = group(groups, "tank")
            local center = centers[1]

            keep_workers(commands, budget, center, centers, scvs, "scv")

            -- Where each terran primary offers its dock, and what stands
            -- there. The content declares it and the annex must be founded on
            -- that cell exactly; the AI view carries no docks, so the brain
            -- states it again — and siting a primary has to honour it too, or
            -- the annex has nowhere to go.
            local DOCKS = {
                factory = { dx = 3, dy = 0, type_name = "tech_lab" },
                command_center = { dx = 3, dy = 0, type_name = "comsat_station" },
            }

            -- An annex is raised by the building whose dock it stands in, so
            -- the command names that building as the builder and the dock's
            -- own cell as the spot.
            local function dock(commands, primary, annex)
                if primary == nil or primary.under_construction then return false end
                if not afford(budget, annex.type_name) then return false end
                commands[#commands + 1] = {
                    kind = "build", builder = primary.id,
                    type_name = annex.type_name,
                    x = primary.x + annex.dx, y = primary.y + annex.dy,
                }
                pay(budget, annex.type_name)
                return true
            end

            -- The refinery goes over the gold seam nearest the base and is
            -- mined in its place. The SCV's gold entry names the refinery as
            -- its only source, so until one stands there is no gold to be had
            -- and wood is the whole economy.
            local wanted = nil
            local builder_id = nil
            if #refineries == 0 then
                local seam = center and nearest(center, view.neutral_entities, function(e)
                    return e.type_name == "gold_mine" and (e.resource_amount or 0) > 0
                end)
                local in_flight = false
                for _, e in ipairs(view.my_entities) do
                    if e.under_construction then in_flight = true end
                end
                if seam ~= nil and not in_flight
                    and (state.build_deadline == nil or view.tick >= state.build_deadline)
                    and afford(budget, "refinery") then
                    local builder = nil
                    for _, w in ipairs(scvs) do
                        if w.idle and not w.hidden then builder = w break end
                    end
                    builder = builder or scvs[1]
                    if builder ~= nil then
                        commands[#commands + 1] = {
                            kind = "build", builder = builder.id,
                            type_name = "refinery", x = seam.x, y = seam.y,
                        }
                        state.build_deadline = view.tick + 200
                        builder_id = builder.id
                    end
                end
                reserve(budget, "refinery")
                -- Headroom is still worth having while the gold is off: a
                -- refinery lost late must not stop the base growing.
                if budget.supply < 2 then
                    wanted = "supply_depot"
                    builder_id = build_next(commands, state, view, scvs, center,
                        wanted, budget, DOCKS[wanted]) or builder_id
                end
            else
                if #barracks == 0 then
                    wanted = "barracks"
                elseif budget.supply < 2 then
                    wanted = "supply_depot"
                elseif #factories == 0 then
                    wanted = "factory"
                -- One turret once the factory stands: the eyes that find a
                -- cloaked raider, and the gun that answers a flier.
                elseif #turrets == 0 then
                    wanted = "missile_turret"
                end
                builder_id = build_next(commands, state, view, scvs, center,
                    wanted, budget, wanted and DOCKS[wanted])
            end

            -- The annexes, once the buildings that hold them stand: the lab
            -- first, since the tanks wait on it, then the station.
            if #labs == 0 and #factories > 0 then
                dock(commands, factories[1], DOCKS.factory)
            elseif #comsats == 0 and #labs > 0 then
                dock(commands, center, DOCKS.command_center)
            end

            -- Idle SCVs work the refinery, one at a time and inside it, and a
            -- bare seam is not a source they may work at all — so until the
            -- refinery stands there is only wood to fetch.
            local need_wood = (wanted ~= nil and budget.wood < cost_of(wanted, "wood"))
                or (#labs == 0 and budget.wood < cost_of("tech_lab", "wood"))
                or (#comsats == 0 and #labs > 0
                    and budget.wood < cost_of("comsat_station", "wood"))
                -- Tanks cost wood where no other race's army does, so once the
                -- lab that gates them stands, an axe stays on wood for them.
                or (#labs > 0 and budget.wood < cost_of("tank", "wood"))
                or #refineries == 0
            for _, w in ipairs(scvs) do
                if w.idle and not w.hidden and w.id ~= builder_id then
                    local target = nil
                    if need_wood then
                        target = nearest(w, view.neutral_entities, function(e)
                            return e.type_name == "tree" and (e.resource_amount or 0) > 0
                        end)
                        -- Once a refinery stands one axe is enough; before it
                        -- does there is no gold to work, so the rest cut too.
                        if #refineries > 0 then need_wood = false end
                    end
                    if target == nil then
                        target = nearest(w, refineries, function(r)
                            return not r.under_construction and (r.resource_amount or 0) > 0
                        end)
                    end
                    if target ~= nil then
                        commands[#commands + 1] = { kind = "select", id = w.id }
                        commands[#commands + 1] = { kind = "send", target = target.id }
                    end
                end
            end

            -- The pending structure holds its price back from the army. Siege
            -- mechanics are left to a player: this brain never plants a tank,
            -- and a research it would not use is a research it should not
            -- hold gold for.
            reserve(budget, wanted)

            -- Marines from the barracks, and a tank per three of them from the
            -- factory — which may only train one with a lab docked to it.
            for _, b in ipairs(barracks) do
                if train_from(commands, budget, b, "marine") then break end
            end
            if any_standing(labs)
                and (#tanks + count_queued(factories, "tank")) * 3 < #marines
            then
                for _, f in ipairs(factories) do
                    if train_from(commands, budget, f, "tank") then break end
                end
            end

            attack_wave(commands, view, marines, tanks, center)

            return commands
        end,
    })
"#;

/// The undead brain: a haunted mine over the nearest seam first — no acolyte
/// draws gold without one — then a crypt for ghouls, ziggurats for headroom,
/// a graveyard, and the temple its necromancers come from. Ghouls cut the wood
/// and make the wave; necromancers walk with it and raise whatever bodies the
/// fighting leaves, anyone's, through the fog they see by.
const UNDEAD_AI: &str = r#"
    define_ai("undead", {
        period = 20,
        vision = "filtered",
        detection = "detectors",
        think = function(state, view)
            local commands = {}
            local budget = budget_of(view)
            local groups = muster(view)
            -- The hall in all three of its forms: the temple asks for one
            -- grown past the necropolis, and acolytes are trained from
            -- whichever form stands.
            local necropolises = group(groups, "necropolis")
            local grown_halls = group(groups, "halls_of_the_dead")
            local citadels = group(groups, "black_citadel")
            local halls = {}
            for _, list in ipairs({ necropolises, grown_halls, citadels }) do
                for _, h in ipairs(list) do halls[#halls + 1] = h end
            end
            local acolytes = group(groups, "acolyte")
            local mines = group(groups, "haunted_mine")
            local crypts = group(groups, "crypt")
            local ziggurats = group(groups, "ziggurat")
            local spirit_towers = group(groups, "spirit_tower")
            local graveyards = group(groups, "graveyard")
            local temples = group(groups, "temple_of_the_damned")
            local ghouls = group(groups, "ghoul")
            local necromancers = group(groups, "necromancer")
            local skeletons = group(groups, "skeleton")
            local hall = halls[1]

            keep_workers(commands, budget, hall, halls, acolytes, "acolyte")

            -- The hall grows once the crypt stands: the temple, and with it
            -- every necromancer, wants a hall past the necropolis, and the
            -- growth is worth more than another ghoul in the meantime.
            if any_standing(crypts) and #grown_halls == 0 and #citadels == 0 then
                for _, n in ipairs(necropolises) do
                    if not n.under_construction
                        and morph_one(commands, budget, n, "halls_of_the_dead") then
                        break
                    end
                end
            end

            -- One ziggurat is hardened into the spirit tower that sees
            -- through cloaks, once a second stands to keep the headroom.
            if #spirit_towers == 0 and #ziggurats >= 2 then
                for _, z in ipairs(ziggurats) do
                    if not z.under_construction
                        and morph_one(commands, budget, z, "spirit_tower") then
                        break
                    end
                end
            end

            -- The haunted mine comes first, raised on the seam nearest the
            -- hall: an acolyte works its rim and nothing else, so until one
            -- stands there is no gold at all.
            local builder_id = nil
            local wanted = nil
            if #mines == 0 then
                local seam = hall and nearest(hall, view.neutral_entities, function(e)
                    return e.type_name == "gold_mine" and (e.resource_amount or 0) > 0
                end)
                local in_flight = false
                for _, e in ipairs(view.my_entities) do
                    if e.under_construction then in_flight = true end
                end
                if seam ~= nil and not in_flight
                    and (state.build_deadline == nil or view.tick >= state.build_deadline)
                    and afford(budget, "haunted_mine") then
                    local builder = nil
                    for _, w in ipairs(acolytes) do
                        if w.idle and not w.hidden then builder = w break end
                    end
                    builder = builder or acolytes[1]
                    if builder ~= nil then
                        commands[#commands + 1] = {
                            kind = "build", builder = builder.id,
                            type_name = "haunted_mine", x = seam.x, y = seam.y,
                        }
                        state.build_deadline = view.tick + 200
                        builder_id = builder.id
                    end
                end
                reserve(budget, "haunted_mine")
            else
                -- Every structure but the hall and the mine wants blight under
                -- it, which those two spread: the ring offsets the chassis
                -- tries sit well inside the hall's own patch.
                if #crypts == 0 then
                    wanted = "crypt"
                elseif budget.supply < 2 then
                    wanted = "ziggurat"
                elseif #graveyards == 0 then
                    wanted = "graveyard"
                elseif #temples == 0 then
                    wanted = "temple_of_the_damned"
                end
                builder_id = build_next(commands, state, view, acolytes, hall, wanted, budget)
            end

            -- Idle acolytes sit round the mine while it has room; the ghouls
            -- do the cutting, so an acolyte never touches a tree.
            for _, w in ipairs(acolytes) do
                if w.idle and not w.hidden and w.id ~= builder_id then
                    local target = nearest(w, mines, function(m)
                        return not m.under_construction and (m.resource_amount or 0) > 0
                    end)
                    if target ~= nil then
                        commands[#commands + 1] = { kind = "select", id = w.id }
                        commands[#commands + 1] = { kind = "send", target = target.id }
                    end
                end
            end

            -- One ghoul stays on wood whenever anything is waiting on it; the
            -- rest muster. A ghoul carrying a load never picks a fight, which
            -- is the point of the axe being in a soldier's hands.
            local need_wood = (wanted ~= nil and budget.wood < cost_of(wanted, "wood"))
                or (#crypts == 0 and budget.wood < cost_of("crypt", "wood"))
            local fighters = {}
            for _, g in ipairs(ghouls) do
                if need_wood and g.idle and not g.hidden then
                    local tree = nearest(g, view.neutral_entities, function(e)
                        return e.type_name == "tree" and (e.resource_amount or 0) > 0
                    end)
                    if tree ~= nil then
                        commands[#commands + 1] = { kind = "select", id = g.id }
                        commands[#commands + 1] = { kind = "send", target = tree.id }
                        need_wood = false
                    end
                elseif g.carrying == nil then
                    fighters[#fighters + 1] = g
                end
            end

            -- The pending structure holds its price back from the army.
            reserve(budget, wanted)

            for _, c in ipairs(crypts) do
                if train_from(commands, budget, c, "ghoul") then break end
            end
            -- One necromancer per four ghouls: the raise is worth more than
            -- another body in the line, but only with a line to raise beside.
            if any_standing(temples) and #necromancers * 4 < #ghouls then
                for _, t in ipairs(temples) do
                    if train_from(commands, budget, t, "necromancer") then break end
                end
            end
            buy_research(commands, budget, view, "skeletal_longevity", temples)

            -- Raise the dead: the nearest body each necromancer with the
            -- energy for it can see, whoever fell there. The cast is an order
            -- — the necromancer walks into the skill's reach and raises — so
            -- one already casting is left to finish.
            local claimed = {}
            for _, n in ipairs(necromancers) do
                if (n.energy or 0) >= 40 and n.idle then
                    local body = nearest(n, view.remains, function(r)
                        return not claimed[r.id]
                    end)
                    if body ~= nil then
                        claimed[body.id] = true
                        commands[#commands + 1] = {
                            kind = "use_skill", skill = "raise_dead",
                            caster = n.id, target = body.id,
                        }
                    end
                end
            end

            -- The raised walk with the wave while they last; the necromancers
            -- follow it rather than lead it.
            local walkers = {}
            for _, e in ipairs(necromancers) do walkers[#walkers + 1] = e end
            for _, e in ipairs(skeletons) do walkers[#walkers + 1] = e end
            attack_wave(commands, view, fighters, walkers, hall)

            return commands
        end,
    })
"#;

/// The human brain's full source: the shared chassis plus its `define_ai`.
pub fn human_ai() -> String {
    format!("{COMMON_AI}\n{HUMAN_AI}")
}

/// The orc brain's full source: the shared chassis plus its `define_ai`.
pub fn orc_ai() -> String {
    format!("{COMMON_AI}\n{ORC_AI}")
}

/// The swarm brain's full source: the shared chassis plus its `define_ai`.
pub fn swarm_ai() -> String {
    format!("{COMMON_AI}\n{SWARM_AI}")
}

/// The conclave brain's full source: the shared chassis plus its `define_ai`.
pub fn conclave_ai() -> String {
    format!("{COMMON_AI}\n{CONCLAVE_AI}")
}

/// The elf brain's full source: the shared chassis plus its `define_ai`.
pub fn elves_ai() -> String {
    format!("{COMMON_AI}\n{ELVES_AI}")
}

/// The terran brain's full source: the shared chassis plus its `define_ai`.
pub fn terran_ai() -> String {
    format!("{COMMON_AI}\n{TERRAN_AI}")
}

/// The undead brain's full source: the shared chassis plus its `define_ai`.
pub fn undead_ai() -> String {
    format!("{COMMON_AI}\n{UNDEAD_AI}")
}

/// The brain source a race's AI slots load, or `None` for a race with no
/// demo brain — its slots idle on unmanned input.
fn race_brain(race: &str) -> Option<String> {
    match race {
        "human" => Some(human_ai()),
        "orc" => Some(orc_ai()),
        "swarm" => Some(swarm_ai()),
        "conclave" => Some(conclave_ai()),
        "elves" => Some(elves_ai()),
        "terran" => Some(terran_ai()),
        "undead" => Some(undead_ai()),
        _ => None,
    }
}

/// The vision and detection the race's demo brain declares — filled into the
/// seat, so every node (and a replay) resolves the brain's commands
/// identically. A race with no brain observes through the fog and detects
/// through its detectors.
pub fn race_senses(race: &str, registry: &ContentRegistry) -> (AiVision, AiDetection) {
    match race_brain(race) {
        Some(script) => brain_senses(&script, registry),
        None => (AiVision::Filtered, AiDetection::Detectors),
    }
}

/// The vision and detection the boss brain declares, for the environment
/// seats it drives.
pub fn environment_senses(registry: &ContentRegistry) -> (AiVision, AiDetection) {
    brain_senses(BOSS_AI_SCRIPT, registry)
}

/// The vision and detection `script` declares. A brain that fails to load
/// (reported when the brains install) observes through the fog and detects
/// through its detectors.
fn brain_senses(script: &str, registry: &ContentRegistry) -> (AiVision, AiDetection) {
    let content = ContentView::from_registry(registry);
    match LuaEngine.load_ai(script, &content) {
        Ok(runtime) => (runtime.vision(), runtime.detection()),
        Err(_) => (AiVision::Filtered, AiDetection::Detectors),
    }
}

/// The boss brain, for the environment slot holding the lake. Thinks once a
/// second: keeps the fleet manned from the fortress and shells the nearest
/// enemy within aggro range with every idle ship. Ships never wander — an
/// unreachable or out-of-range target simply leaves them guarding the lake.
pub const BOSS_AI_SCRIPT: &str = r#"
    local AGGRO_SQ = 100
    local MAX_SHIPS = 4
    local MAX_QUEUE = 2

    define_ai("default", {
        period = 20,
        vision = "filtered",
        detection = "detectors",
        think = function(state, view)
            local commands = {}

            local ships, fortresses = {}, {}
            for _, e in ipairs(view.my_entities) do
                if e.type_name == "ship" then ships[#ships + 1] = e
                elseif e.type_name == "sea_fortress" then fortresses[#fortresses + 1] = e
                end
            end

            -- Keep the fleet manned.
            local queued = 0
            for _, f in ipairs(fortresses) do queued = queued + #f.train_queue end
            for _, f in ipairs(fortresses) do
                if not f.under_construction and #ships + queued < MAX_SHIPS
                    and #f.train_queue < MAX_QUEUE then
                    commands[#commands + 1] =
                        { kind = "train", trainer = f.id, type_name = "ship" }
                    queued = queued + 1
                end
            end

            -- Each idle ship shells the nearest enemy within aggro range.
            for _, s in ipairs(ships) do
                if s.idle then
                    local best, best_distance = nil, nil
                    for _, e in ipairs(view.enemy_entities) do
                        local dx, dy = e.x - s.x, e.y - s.y
                        local distance = dx * dx + dy * dy
                        if distance <= AGGRO_SQ
                            and (best == nil or distance < best_distance) then
                            best, best_distance = e, distance
                        end
                    end
                    if best ~= nil then
                        commands[#commands + 1] = { kind = "select", id = s.id }
                        commands[#commands + 1] = { kind = "attack", target = best.id }
                    end
                end
            end

            return commands
        end,
    })
"#;

/// Builds one demo-AI runtime per AI slot this node sources (which nodes those
/// are follows the session's AI hosting mode) and installs them: the boss
/// brain for environment slots, the race's dedicated brain for the rest. A
/// race with no brain idles on unmanned input; a script failure degrades to
/// idle AI slots — logged, never a stalled game.
pub fn install_demo_ai(world: &mut World) {
    let ai_players = sourced_ai_players(world);
    if ai_players.is_empty() {
        return;
    }

    let environments: Vec<PlayerId> = {
        let session = world.resource::<GameSession>();
        session.environment_slots().map(|slot| slot.id()).collect()
    };

    let content = ContentView::from_registry(world.resource::<ContentRegistry>());
    let mut runtimes = BTreeMap::new();
    for (player, race) in ai_players {
        let script = if environments.contains(&player) {
            BOSS_AI_SCRIPT.to_string()
        } else {
            match race_brain(&race) {
                Some(script) => script,
                None => {
                    eprintln!("no demo ai for race '{race}'; the slot idles");
                    continue;
                }
            }
        };
        match LuaEngine.load_ai(&script, &content) {
            Ok(runtime) => {
                runtimes.insert(player, runtime);
            }
            Err(error) => {
                eprintln!("demo ai failed to load: {error}");
                return;
            }
        }
    }
    install_ai_runtimes(world, AiRuntimes(runtimes));
}
