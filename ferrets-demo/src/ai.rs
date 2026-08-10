//! The demo's AIs: one dedicated economy-then-army brain per race, sharing a
//! common Lua prelude, authored in Lua and installed for every AI slot this
//! node computes.
//!
//! The scripts are deterministic (integer arithmetic, `ipairs` over the
//! ordered view arrays only), so they are valid under either AI hosting mode.

use std::collections::BTreeMap;

use bevy::prelude::*;
use ferrets_bevy_plugin::ai::{AiRuntimes, install_ai_runtimes, sourced_ai_players};
use ferrets_script::{
    ai::view::content::ContentView,
    engine::{ScriptEngine, lua::LuaEngine},
};
use ferrets_simulation::{
    content::registry::ContentRegistry,
    session::{GameSession, player_slot::PlayerId},
};

/// The chassis both race brains run on: pure helpers plus the economy, build,
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
        for _, entry in ipairs(content.entities[type_name].cost) do
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

    -- Puts up `wanted` (one structure at a time) at ring offsets around the
    -- hall. In flight means a site is visibly going up, or a builder was sent
    -- recently and may still be walking — a deadline, not a builder watch,
    -- because a past builder gone back to harvesting never reads idle again.
    -- An invalid placement is a silent no-op that leaves no site behind, so
    -- when the deadline lapses the offset ring advances to the next candidate.
    -- Returns the chosen builder's id.
    local function build_next(commands, state, view, workers, hall, wanted, budget)
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
            if x >= 0 and y >= 0
                and x + 3 <= view.map.width and y + 3 <= view.map.height then
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
        if not building.under_construction and #building.train_queue < MAX_QUEUE
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
        local cost = content.researches[research].cost
        local gold, wood = cost.gold or 0, cost.wood or 0
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

    -- Earmarks the pending structure's price the same way, so training does
    -- not race the builder to the stockpile.
    local function reserve_build(budget, wanted)
        if wanted ~= nil then
            budget.gold = budget.gold - cost_of(wanted, "gold")
            budget.wood = budget.wood - cost_of(wanted, "wood")
        end
    end

    -- Once the wave is big enough, pushes it out: fighters attack-move onto
    -- the nearest enemy in sight — or, with fog hiding every enemy, toward the
    -- far side of the map to scout one out — and escorts (healers) walk along.
    -- Returns whether anything marched.
    local function attack_wave(commands, view, fighters, escorts, hall)
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
        for _, e in ipairs(escorts) do
            if e.idle and not e.hidden then send(e, "move") end
        end
        return marched
    end
"#;

/// The human brain: peasant economy, barracks, then the blacksmith for the
/// iron weapons upgrade and the mortars it unlocks, then a bunker a pair of
/// archers mans for base defense; a medic walks with every few archers,
/// archers burn energy on battle focus when a foe is in reach, and war drums
/// sound as a wave marches.
const HUMAN_AI: &str = r#"
    define_ai("human", {
        period = 20,
        vision = "filtered",
        think = function(state, view)
            local commands = {}
            local budget = budget_of(view)
            local groups = muster(view)
            local halls = group(groups, "town_hall")
            local workers = group(groups, "peasant")
            local barracks = group(groups, "barracks")
            local smithies = group(groups, "blacksmith")
            local bunkers = group(groups, "bunker")
            local archers = group(groups, "archer")
            local mortars = group(groups, "mortar")
            local medics = group(groups, "medic")
            local hall = halls[1]

            keep_workers(commands, budget, hall, halls, workers, "peasant")

            -- The barracks, then the forge that unlocks mortars and hosts the
            -- weapon upgrade — a one-time purchase farms would otherwise
            -- always outbid — then a farm whenever headroom runs dry, and
            -- once the army production is fed, the bunker the defense mans.
            local wanted = nil
            if #barracks == 0 then
                wanted = "barracks"
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
            reserve_build(budget, wanted)

            -- Army mix: a medic per four archers, a pair of mortars once the
            -- forge stands (they require it), archers otherwise.
            for _, b in ipairs(barracks) do
                local trained = "archer"
                if (#medics + count_queued(barracks, "medic")) * 4
                    < #archers + count_queued(barracks, "archer") then
                    trained = "medic"
                elseif any_standing(smithies)
                    and #mortars + count_queued(barracks, "mortar") < 2 then
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
        think = function(state, view)
            local commands = {}
            local budget = budget_of(view)
            local groups = muster(view)
            local halls = group(groups, "great_hall")
            local workers = group(groups, "peon")
            local camps = group(groups, "war_camp")
            local farms = group(groups, "pig_farm")
            local grunts = group(groups, "grunt")
            local shamans = group(groups, "shaman")
            local hall = halls[1]

            keep_workers(commands, budget, hall, halls, workers, "peon")

            -- The war camp, a first pig farm right after it — the frenzy
            -- ritual waits on one — then a farm whenever headroom runs dry.
            local wanted = nil
            if #camps == 0 then
                wanted = "war_camp"
            elseif #farms == 0 then
                wanted = "pig_farm"
            elseif budget.supply < 2 then
                wanted = "pig_farm"
            end
            local builder_id =
                build_next(commands, state, view, workers, hall, wanted, budget)
            local need_wood = wanted ~= nil and budget.wood < cost_of(wanted, "wood")
            assign_harvesters(commands, view, workers, need_wood, builder_id)

            -- The ritual and the pending structure hold their price back from
            -- the army before any unit is queued.
            buy_research(commands, budget, view, "frenzy_ritual", camps)
            reserve_build(budget, wanted)

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

            if attack_wave(commands, view, grunts, shamans, hall) then
                -- War drums speed the wave out; refused while cooling or broke.
                commands[#commands + 1] =
                    { kind = "use_skill", skill = "war_drums", caster = "player" }
            end

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
            match race.as_str() {
                "human" => human_ai(),
                "orc" => orc_ai(),
                other => {
                    eprintln!("no demo ai for race '{other}'; the slot idles");
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
