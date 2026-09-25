//! The content-DSL binding: `define_*` host functions and the table readers
//! that map an entity table onto the [`EntityTypeDef`] builder.

use ferrets_geometry::{cell_pos::CellPos, cell_size::CellSize};
use std::{cell::RefCell, rc::Rc};

use ferrets_content::{
    annex::{AloneConduct, AnnexClaim, AnnexLife},
    attack::{Delivery, Slain, Weapon},
    berths::BerthGroup,
    brood::{Lingering, OrphanFate},
    build::BuilderAttendance,
    cost::Cost,
    detection::Detection,
    dying::{Bequest, LeftBy},
    entity_buffs::{EntityBuffDef, Lasting},
    entity_effect::EntityEffect,
    entity_stats::EntityStatId,
    entity_type_def::EntityTypeDef,
    field::{
        Emission, FieldDecay, FieldDef, FieldEffect, FieldGrowth, FieldId, FieldLayer,
        FieldPlacement, FieldSide, FieldSourceDef, FieldVision,
    },
    kinds::{Kind, Kinds},
    morph::{MorphInterrupted, MorphReason, MorphTransition},
    player_buffs::PlayerBuffDef,
    price::Price,
    projectile::ProjectileDef,
    quantity::Quantity,
    registry::ContentRegistry,
    repair::{RepairCost, RepairRate},
    requirement::Requirement,
    research::{ResearchDef, ResearchId},
    resource::{Banking, HarvestData},
    skills::{
        Casting, EntityCastEffect, EntityCastTarget, PlayerCastEffect, Reach, SkillCaster, SkillDef,
    },
    splash::SplashDef,
    stand::StandingAct,
    stats::{EntityModifier, ModifierOp, PlayerModifier},
    turret::{TurretDef, TurretMount, TurretStats, WeaponConduct},
    work::{Attachment, BerthStance, CrewLimit, WorkPresence},
};
use ferrets_math::{FixedI64, FixedU64, fixed_vec2::FixedVec2};
use ferrets_pathfinder::layer_mask::LayerMask;
use mlua::{Lua, Table, Value};

use crate::{content, error::ScriptError};

/// Installs the `define_*` globals, each registering into `registry` — the one
/// assigner of every derived id, so what a script observes (the layer id
/// `define_layer` returns) is what the finished registry holds.
pub(super) fn register(lua: &Lua, registry: &Rc<RefCell<ContentRegistry>>) -> mlua::Result<()> {
    let globals = lua.globals();

    let races = Rc::clone(registry);
    globals.set(
        "define_race",
        lua.create_function(move |_, name: String| {
            races.borrow_mut().register_race(name);
            Ok(())
        })?,
    )?;

    let resources = Rc::clone(registry);
    globals.set(
        "define_resource",
        lua.create_function(move |_, kind: String| {
            resources.borrow_mut().register_resource(kind);
            Ok(())
        })?,
    )?;

    let tags = Rc::clone(registry);
    globals.set(
        "define_tag",
        lua.create_function(move |_, tag: String| {
            tags.borrow_mut().register_tag(tag);
            Ok(())
        })?,
    )?;

    let layers = Rc::clone(registry);
    globals.set(
        "define_layer",
        lua.create_function(move |_, name: String| Ok(*layers.borrow_mut().register_layer(name)))?,
    )?;

    let lookup = Rc::clone(registry);
    globals.set(
        "layer_id",
        lua.create_function(move |_, name: String| match lookup.borrow().layer(&name) {
            Some(id) => Ok(*id),
            None => Err(mlua::Error::external(ScriptError::ContentError(format!(
                "layer '{name}' is not defined"
            )))),
        })?,
    )?;

    let terrains = Rc::clone(registry);
    globals.set(
        "define_terrain",
        lua.create_function(move |_, (name, passable): (String, u32)| {
            terrains.borrow_mut().register_terrain(name, passable);
            Ok(())
        })?,
    )?;

    let fields = Rc::clone(registry);
    globals.set(
        "define_field",
        lua.create_function(move |_, (name, table): (String, Table)| {
            let field = parse_field(&table).map_err(mlua::Error::external)?;
            fields.borrow_mut().register_field(name, field);
            Ok(())
        })?,
    )?;

    let stats = Rc::clone(registry);
    globals.set(
        "define_entity_stat",
        lua.create_function(move |_, (name, floor): (String, Value)| {
            let floor = fixed_value("entity stat floor", &floor).map_err(mlua::Error::external)?;
            stats.borrow_mut().register_entity_stat(name, floor);
            Ok(())
        })?,
    )?;

    let player_stats = Rc::clone(registry);
    globals.set(
        "define_player_stat",
        lua.create_function(move |_, name: String| {
            player_stats.borrow_mut().register_player_stat(name);
            Ok(())
        })?,
    )?;

    let entity_buffs = Rc::clone(registry);
    globals.set(
        "define_entity_buff",
        lua.create_function(move |_, (name, table): (String, Table)| {
            let buff =
                parse_entity_buff(&table, &entity_buffs.borrow()).map_err(mlua::Error::external)?;
            entity_buffs.borrow_mut().register_entity_buff(name, buff);
            Ok(())
        })?,
    )?;

    let player_buffs = Rc::clone(registry);
    globals.set(
        "define_player_buff",
        lua.create_function(move |_, (name, table): (String, Table)| {
            let buff =
                parse_player_buff(&table, &player_buffs.borrow()).map_err(mlua::Error::external)?;
            player_buffs.borrow_mut().register_player_buff(name, buff);
            Ok(())
        })?,
    )?;

    let projectiles = Rc::clone(registry);
    globals.set(
        "define_projectile",
        lua.create_function(move |_, (name, table): (String, Table)| {
            let projectile = parse_projectile(&table).map_err(mlua::Error::external)?;
            projectiles
                .borrow_mut()
                .register_projectile(name, projectile);
            Ok(())
        })?,
    )?;

    let turrets = Rc::clone(registry);
    globals.set(
        "define_turret",
        lua.create_function(move |_, (name, table): (String, Table)| {
            let turret = parse_turret(&table, &turrets.borrow()).map_err(mlua::Error::external)?;
            turrets.borrow_mut().register_turret(name, turret);
            Ok(())
        })?,
    )?;

    let skills = Rc::clone(registry);
    globals.set(
        "define_skill",
        lua.create_function(move |_, (name, table): (String, Table)| {
            let skill = parse_skill(&table, &skills.borrow()).map_err(mlua::Error::external)?;
            skills.borrow_mut().register_skill(name, skill);
            Ok(())
        })?,
    )?;

    let researches = Rc::clone(registry);
    globals.set(
        "define_research",
        lua.create_function(move |_, (name, table): (String, Table)| {
            let research =
                parse_research(&table, &researches.borrow()).map_err(mlua::Error::external)?;
            researches.borrow_mut().register_research(name, research);
            Ok(())
        })?,
    )?;

    let entities = Rc::clone(registry);
    globals.set(
        "define_entity",
        lua.create_function(move |_, (name, table): (String, Table)| {
            let def =
                build_entity(&name, &table, &entities.borrow()).map_err(mlua::Error::external)?;
            entities.borrow_mut().register(def);
            Ok(())
        })?,
    )?;

    Ok(())
}

/// Builds one entity type from its Lua table, mapping each present field onto the
/// corresponding [`EntityTypeDef`] builder.
fn build_entity(
    name: &str,
    table: &Table,
    registry: &ContentRegistry,
) -> crate::Result<EntityTypeDef> {
    let mut def = EntityTypeDef::new(name);

    if let Some(race) = optional::<String>(table, "race")? {
        def = def.with_race(race);
    }

    let location = required::<Table>(table, "location")?;
    let occupation = required::<u32>(&location, "occupation")?;
    let solidity = required::<String>(&location, "solidity")?;
    def = def.with_location(
        occupation,
        cell_size(&location)?,
        content::solidity(&solidity)?,
    );

    if let Some(stats) = optional::<Table>(table, "stats")? {
        for (stat, value) in parse_stats(&stats, registry)? {
            def = def.with_stat(stat, value);
        }
    }
    if let Some(dying) = optional::<Table>(table, "dying")? {
        let leaves = parse_leaves(&dying)?;
        def = match optional::<u32>(&dying, "time")? {
            Some(time) => def.with_dying(time, leaves),
            None => def.with_leaves(leaves),
        };
    }
    if let Some(price) = optional::<Table>(table, "price")? {
        def = def.with_price(pairs::<u32>(&price, "price")?);
    }
    if let Some(train_time) = optional::<u32>(table, "train_time")? {
        def = def.with_train_time(train_time);
    }
    if let Some(build_time) = optional::<u32>(table, "build_time")? {
        def = def.with_build_time(build_time);
    }
    if let Some(trainer) = optional::<Vec<String>>(table, "trainer")? {
        def = def.with_trainer(trainer);
    }
    if let Some(transporter) = optional::<Table>(table, "transporter")? {
        let carries = parse_kinds(&required::<Value>(&transporter, "carries")?, "carries")?;
        let boarding = required::<String>(&transporter, "boarding")?;
        let fate = required::<String>(&transporter, "fate")?;
        let conduct = required::<String>(&transporter, "conduct")?;
        def = def.with_transporter(
            carries,
            content::affiliation(&boarding)?,
            content::passenger_fate(&fate)?,
            content::passenger_conduct(&conduct)?,
        );
    }
    if let Some(researcher) = optional::<Vec<String>>(table, "researcher")? {
        let ids = researcher
            .iter()
            .map(|name| {
                registry.research(name).ok_or_else(|| {
                    ScriptError::ContentError(format!("research '{name}' is not defined"))
                })
            })
            .collect::<crate::Result<Vec<_>>>()?;
        def = def.with_researcher(ids);
    }
    let requires = parse_requires(table, registry)?;
    if !requires.is_empty() {
        def = def.with_requires(requires);
    }
    if let Some(builder) = optional::<Table>(table, "builder")? {
        let builds = required::<Vec<String>>(&builder, "builds")?;
        let attendance = required::<Value>(&builder, "attendance")?;
        def = def.with_builder(builds, builder_attendance(&attendance)?);
    }
    if let Some(repairer) = optional::<Table>(table, "repairer")? {
        let repairs = parse_kinds(&required::<Value>(&repairer, "repairs")?, "repairs")?;
        let presence = required::<Value>(&repairer, "presence")?;
        // Off unless declared, and an omitted patience waits indefinitely.
        let self_repair = optional::<bool>(&repairer, "self_repair")?.unwrap_or(false);
        let patience = optional::<u32>(&repairer, "patience")?;
        def = def.with_repairer(
            repairs,
            parse_repair_rate(&repairer)?,
            work_presence(&presence)?,
            self_repair,
            parse_repair_cost(&repairer)?,
            patience,
        );
    }
    if let Some(ratio) = optional::<String>(table, "repair_ratio")? {
        def = def.with_repair_ratio(content::fixed(&ratio)?);
    }
    if let Some(source) = optional::<Table>(table, "resource_source")? {
        let kind = required::<String>(&source, "kind")?;
        let depletion = required::<String>(&source, "depletion")?;
        def = def.with_resource_source(kind, content::depletion(&depletion)?);
    }
    if let Some(carrier) = optional::<Table>(table, "resource_carrier")? {
        def = def.with_resource_carrier(harvest_kinds(&carrier)?);
    }
    if let Some(storage) = optional::<Vec<String>>(table, "resource_storage")? {
        def = def.with_resource_storage(storage);
    }
    if let Some(berths) = optional::<Table>(table, "berths")? {
        def = def.with_berths(berth_groups(&berths)?);
    }
    if let Some(over) = optional::<String>(table, "overbuilds")? {
        def = def.with_overbuilds(over);
    }
    if let Some(docks) = optional::<Vec<Table>>(table, "docks")? {
        def = def.with_docks(parse_docks(docks)?);
    }
    if let Some(annex) = optional::<Table>(table, "annex")? {
        let (alone, claim) = parse_annex(&annex)?;
        def = def.with_annex(alone, claim);
    }
    if let Some(brood) = optional::<Table>(table, "breeder")? {
        let breeds = required::<String>(&brood, "breeds")?;
        let period = parse_quantity(
            "brood period",
            "tick",
            &required::<Value>(&brood, "period")?,
            registry,
        )?;
        let limit = required::<usize>(&brood, "limit")?;
        let initial = optional::<usize>(&brood, "initial")?.unwrap_or(0);
        let orphans = orphan_fate(&required::<Value>(&brood, "orphans")?)?;
        def = def.with_breeder(breeds, period, limit, initial, orphans);
    }
    if let Some(broodling) = optional::<Table>(table, "broodling")? {
        def = def.with_broodling(attachment(&broodling)?);
    }
    if let Some(tags) = optional::<Vec<String>>(table, "tags")? {
        def = def.with_tags(tags);
    }
    if let Some(selection) = optional::<Table>(table, "selection")? {
        let priority = optional::<u32>(&selection, "priority")?.unwrap_or(0);
        let class = optional::<String>(&selection, "class")?;
        def = def.with_selection(priority, class.as_deref());
    }
    if let Some(bonuses) = optional::<Table>(table, "bonus_damage_vs")? {
        def = def.with_bonus_damage_vs(pairs::<u32>(&bonuses, "bonus_damage_vs")?);
    }
    // One block, because a weapon is one thing the body either points or does
    // not: what it reaches, how its hit travels, what it spreads over. The layers
    // it reaches are required inside it — a weapon that reaches nothing could
    // never fire, and stating the rest without them says nothing.
    if let Some(attack) = optional::<Table>(table, "attack")? {
        let (targets, delivery, splash, slain) = parse_weapon(&attack, registry)?;
        def = def.with_attack_def(targets, delivery, splash, slain);
    }
    if let Some(turrets) = optional::<Vec<Table>>(table, "turrets")? {
        def = def.with_turrets(parse_turret_mounts(turrets, registry)?);
    }
    if let Some(fire) = optional::<String>(table, "turret_fire")? {
        def = def.with_turret_fire(content::turret_fire(&fire)?);
    }
    // A weapon may omit its acquisition range; the attack range is the default.
    // Read after the weapons, since whether there is one to arm is what decides it.
    if def.can_attack()
        && def.base_stat(EntityStatId::ACQUIRE_RANGE).is_none()
        && let Some(range) = def.base_stat(EntityStatId::ATTACK_RANGE)
    {
        def = def.with_stat(EntityStatId::ACQUIRE_RANGE, range);
    }
    if let Some(targetable) = optional::<u32>(table, "targetable")? {
        def = def.with_targetable(LayerMask::from(targetable));
    }
    if let Some(concealment) = optional::<String>(table, "concealment")? {
        def = def.with_concealment(content::concealment(&concealment)?);
    }
    if let Some(morphs) = optional::<Vec<Table>>(table, "morphs")? {
        def = def.with_morphs(parse_morphs(morphs, registry)?);
    }
    if let Some(sources) = optional::<Vec<Table>>(table, "field_sources")? {
        def = def.with_field_sources(parse_field_sources(&sources, registry)?);
    }
    if let Some(rules) = optional::<Vec<Table>>(table, "field_placement")? {
        def = def.with_field_placement(parse_field_placement(&rules, registry)?);
    }
    if let Some(effects) = optional::<Vec<Table>>(table, "field_effects")? {
        def = def.with_field_effects(parse_field_effects(&effects, registry)?);
    }
    if let Some(acts) = optional::<Vec<Table>>(table, "on_stand")? {
        def = def.with_standing_acts(parse_on_stand(&acts, registry)?);
    }
    if let Some(skills) = optional::<Vec<String>>(table, "skills")? {
        let ids = skills
            .iter()
            .map(|name| {
                registry.skill(name).ok_or_else(|| {
                    ScriptError::ContentError(format!("skill '{name}' is not defined"))
                })
            })
            .collect::<crate::Result<Vec<_>>>()?;
        def = def.with_skills(ids);
    }

    Ok(def)
}

/// Reads a repairer's `rate` block: `{ mode = ... }`, where the mode is
/// `"production"` or `"per_tick"` with a `health` amount.
///
/// Required, because the two pace the work so differently that inferring one would
/// hide the choice: a structure mended against its build time and a casualty patched
/// up at a flat rate are both ordinary content.
fn parse_repair_rate(repairer: &Table) -> crate::Result<RepairRate> {
    let rate = required::<Table>(repairer, "rate")?;
    let mode = required::<String>(&rate, "mode")?;
    match mode.as_str() {
        "production" => Ok(RepairRate::Production),
        "per_tick" => {
            let health = required::<String>(&rate, "health")?;
            Ok(RepairRate::PerTick(content::fixed(&health)?))
        }
        other => Err(content::unexpected(
            "repair rate mode",
            &["'production'", "'per_tick'"],
            &content::quoted(other),
        )),
    }
}

/// Reads a repairer's `cost` block: `{ mode = ... }`, where the mode is `"free"`,
/// `"pro_rata"`, `"per_tick"` with a `resources` table of amounts, or `"energy"`
/// with a `per_health` rate.
///
/// Required, and `"free"` has to be said out loud: free work is a balance stance
/// rather than an absence, and inferring it would turn a misspelled field name into
/// unlimited free repair.
fn parse_repair_cost(repairer: &Table) -> crate::Result<RepairCost> {
    let cost = required::<Table>(repairer, "cost")?;
    let mode = required::<String>(&cost, "mode")?;
    match mode.as_str() {
        "free" => Ok(RepairCost::Free),
        "pro_rata" => Ok(RepairCost::ProRata),
        "per_tick" => {
            let resources = required::<Table>(&cost, "resources")?;
            Ok(RepairCost::PerTick(
                pairs::<u32>(&resources, "resources")?.into_iter().collect(),
            ))
        }
        "energy" => {
            let per_health = required::<String>(&cost, "per_health")?;
            Ok(RepairCost::Energy(content::fixed(&per_health)?))
        }
        other => Err(content::unexpected(
            "repair cost mode",
            &["'free'", "'pro_rata'", "'per_tick'", "'energy'"],
            &content::quoted(other),
        )),
    }
}

/// Reads one projectile: `{ speed, aim }` — a decimal string in cells per tick, and
/// whether the hit resolves against the target entity or the cell it was sent to.
fn parse_projectile(table: &Table) -> crate::Result<ProjectileDef> {
    let speed = required::<String>(table, "speed")?;
    let aim = required::<String>(table, "aim")?;
    Ok(ProjectileDef::new(
        content::fixed(&speed)?,
        content::attack_aim(&aim)?,
    ))
}

/// Reads what a cast is aimed at: `"caster"`, `"position"`, `"ally"`,
/// `"enemy"` or `"fallen"` on their own, or any of the last three as a table
/// with a filter — `{ kind = "ally", only = { tags = { "biological" } } }`.
fn parse_cast_target(value: &Value) -> crate::Result<EntityCastTarget> {
    let (kind, only) = match value {
        Value::String(kind) => (kind.to_string_lossy(), Kinds::Any),
        Value::Table(target) => {
            let kind = required::<String>(target, "kind")?;
            let only = match optional::<Value>(target, "only")? {
                Some(only) => parse_kinds(&only, "cast target filter")?,
                None => Kinds::Any,
            };
            (kind, only)
        }
        other => {
            return Err(content::unexpected(
                "skill target",
                &["a target name", "a { kind = ..., only = ... } table"],
                &found(other),
            ));
        }
    };
    match kind.as_str() {
        "caster" => Ok(EntityCastTarget::Caster),
        "position" => Ok(EntityCastTarget::Position),
        "fallen" => Ok(EntityCastTarget::Fallen { kinds: only }),
        // Anything else names whose it must be, in the same words every other
        // capability uses.
        whose => {
            let side = content::affiliation(whose).map_err(|_| {
                content::unexpected(
                    "skill target",
                    &[
                        "'caster'",
                        "'position'",
                        "'fallen'",
                        "'own'",
                        "'allied'",
                        "'enemy'",
                        "'anyone'",
                    ],
                    &content::quoted(whose),
                )
            })?;
            Ok(EntityCastTarget::Standing { side, kinds: only })
        }
    }
}

/// Reads a filter: `{ types = {...}, tags = {...} }`, or `"any"` for one that
/// names everything. Which vocabulary each name is drawn from is said rather
/// than guessed, since a type and a tag may share a name.
fn parse_kinds(value: &Value, what: &str) -> crate::Result<Kinds> {
    match value {
        Value::String(any) if any == "any" => Ok(Kinds::Any),
        Value::Table(kinds) => {
            let mut named: Vec<Kind> = Vec::new();
            for name in optional::<Vec<String>>(kinds, "types")?.unwrap_or_default() {
                named.push(Kind::Type(name));
            }
            for name in optional::<Vec<String>>(kinds, "tags")?.unwrap_or_default() {
                named.push(Kind::Tag(name));
            }
            Ok(Kinds::only(named))
        }
        other => Err(content::unexpected(
            what,
            &["'any'", "a { types = {...}, tags = {...} } table"],
            &found(other),
        )),
    }
}

/// Reads what a death hands on: each entry names the entity type left standing,
/// how many of it (one unless said), and the deaths that leave it — the
/// engine's own rule unless the entry names them itself.
fn parse_leaves(dying: &Table) -> crate::Result<Vec<Bequest>> {
    let Some(leaves) = optional::<Vec<Table>>(dying, "leaves")? else {
        return Ok(Vec::new());
    };
    let mut bequests = Vec::with_capacity(leaves.len());
    for entry in leaves {
        let entity_type = required::<String>(&entry, "entity")?;
        let count = optional::<u32>(&entry, "count")?.unwrap_or(1);
        let causes = match optional::<Vec<String>>(&entry, "on")? {
            None => LeftBy::Ordinary,
            Some(names) => LeftBy::Named(
                names
                    .iter()
                    .map(|name| content::death_kind(name))
                    .collect::<crate::Result<Vec<_>>>()?,
            ),
        };
        bequests.push(Bequest::new(&entity_type, count, causes));
    }
    Ok(bequests)
}

/// Reads the weapon a block states: what it reaches, how its hit travels, what
/// that hit spreads over, and what it leaves of what it kills. Shared by the
/// body's own weapon and every turret's.
fn parse_weapon(
    table: &Table,
    registry: &ContentRegistry,
) -> crate::Result<(LayerMask, Delivery, Option<SplashDef>, Slain)> {
    let targets = required::<u32>(table, "targets")?;
    let delivery = match optional::<String>(table, "projectile")? {
        None => Delivery::Instant,
        Some(projectile) => {
            let id = registry.projectile(&projectile).ok_or_else(|| {
                ScriptError::ContentError(format!("projectile '{projectile}' is not defined"))
            })?;
            Delivery::Projectile(id)
        }
    };
    let splash = match optional::<Table>(table, "splash")? {
        None => None,
        Some(splash) => {
            let shape = required::<String>(&splash, "shape")?;
            let shape = content::splash_shape(&shape)?;
            let bands = splash_bands(&splash)?;
            let layers = required::<u32>(&splash, "layers")?;
            let friendly_fire = required_flag(&splash, "friendly_fire")?;
            Some(SplashDef::new(
                shape,
                bands,
                LayerMask::from(layers),
                friendly_fire,
            ))
        }
    };
    let slain = match optional::<String>(table, "slain")? {
        None => Slain::Remains,
        Some(slain) => content::slain(&slain)?,
    };
    Ok((LayerMask::from(targets), delivery, splash, slain))
}

/// Reads one turret: the weapon it fires, which of the mounting type's stats each
/// of its numbers reads, and whether it works a target while the body walks.
fn parse_turret(table: &Table, registry: &ContentRegistry) -> crate::Result<TurretDef> {
    let (targets, delivery, splash, slain) = parse_weapon(table, registry)?;
    let conduct = match optional::<String>(table, "conduct")? {
        None => WeaponConduct::Halts,
        Some(conduct) => content::weapon_conduct(&conduct)?,
    };
    let mut stats = TurretStats::default();
    if let Some(named) = optional::<Table>(table, "stats")? {
        for (field, slot) in [
            ("damage", &mut stats.damage),
            ("range", &mut stats.range),
            ("acquire_range", &mut stats.acquire_range),
            ("period", &mut stats.period),
            ("damage_point", &mut stats.damage_point),
            ("aim_rate", &mut stats.aim_rate),
            ("arc", &mut stats.arc),
        ] {
            let Some(name) = optional::<String>(&named, field)? else {
                continue;
            };
            *slot = registry.entity_stat(&name).ok_or_else(|| {
                ScriptError::ContentError(format!("stat '{name}' is not defined"))
            })?;
        }
    }
    Ok(TurretDef::new(
        Weapon::new(targets, delivery, splash, slain),
        stats,
        conduct,
    ))
}

/// Reads a type's turret mounts: which gun each is, and the patch of the
/// footprint it sits on — `at` from the footprint's own origin, `size` in cells.
fn parse_turret_mounts(
    mounts: Vec<Table>,
    registry: &ContentRegistry,
) -> crate::Result<Vec<TurretMount>> {
    let mut out = Vec::with_capacity(mounts.len());
    for mount in mounts {
        let name = required::<String>(&mount, "turret")?;
        let turret = registry
            .turret(&name)
            .ok_or_else(|| ScriptError::ContentError(format!("turret '{name}' is not defined")))?;
        let at = optional::<Table>(&mount, "at")?;
        let origin = match at {
            None => CellPos::new(0, 0),
            Some(at) => CellPos::new(
                at.get::<u32>(1)
                    .map_err(|error| field_error("turret mount x", error))?,
                at.get::<u32>(2)
                    .map_err(|error| field_error("turret mount y", error))?,
            ),
        };
        let size = match optional::<Table>(&mount, "size")? {
            None => CellSize::ONE,
            Some(size) => CellSize::new(
                size.get::<u32>(1)
                    .map_err(|error| field_error("turret mount width", error))?,
                size.get::<u32>(2)
                    .map_err(|error| field_error("turret mount height", error))?,
            ),
        };
        out.push(TurretMount::new(turret, origin, size));
    }
    Ok(out)
}

/// Reads a splash `bands` list: an array of `{ radius, fraction }` pairs, innermost
/// first, where the fraction is a decimal string.
fn splash_bands(splash: &Table) -> crate::Result<Vec<(u32, FixedU64)>> {
    let bands: Vec<Table> = required(splash, "bands")?;
    let mut out = Vec::with_capacity(bands.len());
    for band in bands {
        let radius = band
            .get::<u32>(1)
            .map_err(|error| field_error("splash band radius", error))?;
        let fraction = band
            .get::<String>(2)
            .map_err(|error| field_error("splash band fraction", error))?;
        out.push((radius, content::fixed(&fraction)?));
    }
    Ok(out)
}

/// Reads the flat `stats = { name = value }` table: each key is a registered stat
/// name (built-in, or content-declared with `define_entity_stat`), each value a
/// non-negative integer or a decimal string. An unknown name is rejected — a
/// custom stat must be declared before it is set.
fn parse_stats(
    stats: &Table,
    registry: &ContentRegistry,
) -> crate::Result<Vec<(EntityStatId, FixedU64)>> {
    let mut out = Vec::new();
    for pair in stats.pairs::<String, Value>() {
        let (name, value) = pair.map_err(|error| field_error("stats", error))?;
        let stat = registry
            .entity_stat(&name)
            .ok_or_else(|| ScriptError::ContentError(format!("stat '{name}' is not defined")))?;
        out.push((stat, stat_value(&name, value)?));
    }
    Ok(out)
}

/// Reads one stat value: a non-negative integer, or a decimal string for a
/// fractional value (floats are rejected at the determinism boundary).
fn stat_value(name: &str, value: Value) -> crate::Result<FixedU64> {
    fixed_value(&format!("stat '{name}'"), &value)
}

/// Reads `location.size`: an integer `n` means an `n×n` footprint; a `{w, h}`
/// array gives the two dimensions.
fn cell_size(location: &Table) -> crate::Result<CellSize> {
    match required::<Value>(location, "size")? {
        Value::Integer(side) => {
            let side = dimension(side)?;
            Ok(CellSize::new(side, side))
        }
        Value::Table(size) => {
            let width = index(&size, 1)?;
            let height = index(&size, 2)?;
            Ok(CellSize::new(width, height))
        }
        other => Err(ScriptError::ContentError(format!(
            "size must be an integer or {{width, height}}, got {}",
            other.type_name()
        ))),
    }
}

/// Reads a `{kind = amount}` map as `(kind, amount)` pairs.
fn pairs<V: mlua::FromLua>(table: &Table, field: &str) -> crate::Result<Vec<(String, V)>> {
    let mut entries = Vec::new();
    for pair in table.clone().pairs::<String, V>() {
        entries.push(pair.map_err(|error| field_error(field, error))?);
    }
    Ok(entries)
}

/// Reads a `resource_carrier` map: each kind maps to `{capacity, time,
/// presence, drain, banking, sources}`. `drain` defaults to the capacity,
/// `banking` to `carried`, and `sources` to any source of the kind.
fn harvest_kinds(carrier: &Table) -> crate::Result<Vec<(String, HarvestData)>> {
    let mut carries = Vec::new();
    for pair in carrier.clone().pairs::<String, Table>() {
        let (kind, data) = pair.map_err(|error| field_error("resource_carrier", error))?;
        let capacity = required::<u32>(&data, "capacity")?;
        let banking = match optional::<String>(&data, "banking")? {
            Some(name) => content::banking(&name)?,
            None => Banking::Carried,
        };
        let harvest = HarvestData::new(
            capacity,
            optional::<u32>(&data, "drain")?.unwrap_or(capacity),
            required::<u32>(&data, "time")?,
            work_presence(&required::<Value>(&data, "presence")?)?,
            banking,
            match optional::<Value>(&data, "sources")? {
                Some(sources) => parse_kinds(&sources, "harvest sources")?,
                None => Kinds::Any,
            },
        );
        carries.push((kind, harvest));
    }
    Ok(carries)
}

/// Reads a `berths` map: each group name maps to `{ points, slots }` — the
/// `{x, y}` points about the footprint, in cells from its anchor as whole
/// numbers or decimal strings of either sign, and how many workers sit at
/// once, defaulting to one per point.
fn berth_groups(berths: &Table) -> crate::Result<Vec<(String, BerthGroup)>> {
    let mut groups = Vec::new();
    for (name, group) in pairs::<Table>(berths, "berths")? {
        let points = required::<Vec<Vec<Value>>>(&group, "points")?
            .into_iter()
            .map(|point| match point.as_slice() {
                [x, y] => Ok(FixedVec2::new(
                    signed_fixed_value("berth x", x)?,
                    signed_fixed_value("berth y", y)?,
                )),
                _ => Err(ScriptError::ContentError(format!(
                    "berth group '{name}' points must be {{x, y}} pairs"
                ))),
            })
            .collect::<crate::Result<Vec<_>>>()?;
        let slots = optional::<usize>(&group, "slots")?.unwrap_or(points.len());
        groups.push((name, BerthGroup::new(points, slots)));
    }
    Ok(groups)
}

/// Reads a fixed-point value of either sign: a whole number, or a decimal
/// string (floats are rejected at the determinism boundary).
fn signed_fixed_value(what: &str, value: &Value) -> crate::Result<FixedI64> {
    match value {
        Value::Integer(n) => Ok(FixedI64::from_num(*n)),
        Value::String(s) => content::signed_fixed(&s.to_string_lossy()),
        other => Err(ScriptError::ContentError(format!(
            "{what} must be an integer or a decimal string, got {}",
            other.type_name()
        ))),
    }
}

/// Reads a non-negative fixed-point value: a whole number, or a decimal string
/// (floats are rejected at the determinism boundary).
fn fixed_value(what: &str, value: &Value) -> crate::Result<FixedU64> {
    match value {
        Value::Integer(n) if *n >= 0 => Ok(FixedU64::from_num(*n)),
        Value::String(s) => content::fixed(&s.to_string_lossy()),
        other => Err(ScriptError::ContentError(format!(
            "{what} must be a non-negative integer or a decimal string, got {}",
            other.type_name()
        ))),
    }
}

/// Reads a setting written either as one of `options`' keywords or as a table
/// `read` converts, naming both forms when it is neither.
fn keyword_or_table<T: Clone>(
    what: &str,
    value: &Value,
    options: &[(&str, T)],
    tables: &[&str],
    read: impl FnOnce(&Table) -> crate::Result<T>,
) -> crate::Result<T> {
    match value {
        Value::Table(table) => read(table),
        Value::String(name) => {
            let name = name.to_string_lossy();
            options
                .iter()
                .find(|(keyword, _)| *keyword == name)
                .map(|(_, option)| option.clone())
                .ok_or_else(|| {
                    keyword_or_table_error(what, options, tables, &content::quoted(&name))
                })
        }
        other => Err(keyword_or_table_error(what, options, tables, &found(other))),
    }
}

/// The error for a value that is neither a keyword of `options` nor one of the
/// `tables` forms.
fn keyword_or_table_error<T>(
    what: &str,
    options: &[(&str, T)],
    tables: &[&str],
    found: &str,
) -> ScriptError {
    let keywords: Vec<String> = options
        .iter()
        .map(|(name, _)| content::quoted(name))
        .collect();
    let mut expected: Vec<&str> = keywords.iter().map(String::as_str).collect();
    expected.extend_from_slice(tables);
    content::unexpected(what, &expected, found)
}

/// Reads a work presence: `{ hidden = { crew } }` or `{ present = { crew } }`
/// for one that stands or hides on its own, `{ attached = { berths, stance } }`
/// for one that sits in a job's berths.
fn work_presence(value: &Value) -> crate::Result<WorkPresence> {
    match value {
        Value::Table(table) => work_presence_table(table),
        other => Err(content::unexpected(
            "work presence",
            &[
                "a { hidden = ... } table",
                "a { present = ... } table",
                "an { attached = ... } table",
            ],
            &found(other),
        )),
    }
}

/// Reads the table form of a presence: exactly one of `hidden`, `present` or
/// `attached`.
fn work_presence_table(table: &Table) -> crate::Result<WorkPresence> {
    let hidden = optional::<Table>(table, "hidden")?;
    let present = optional::<Table>(table, "present")?;
    let attached = optional::<Table>(table, "attached")?;
    match (hidden, present, attached) {
        (Some(hidden), None, None) => Ok(WorkPresence::Hidden {
            crew: crew_limit(&hidden)?,
        }),
        (None, Some(present), None) => Ok(WorkPresence::Present {
            crew: crew_limit(&present)?,
        }),
        (None, None, Some(attached)) => Ok(WorkPresence::Attached(attachment(&attached)?)),
        _ => Err(ScriptError::ContentError(
            "a work presence table must have exactly one of 'hidden', 'present' or 'attached'"
                .to_string(),
        )),
    }
}

/// Reads the `crew` of a presence: how many workers may work one job at once,
/// as a count or `"any"`.
fn crew_limit(table: &Table) -> crate::Result<CrewLimit> {
    match required::<Value>(table, "crew")? {
        Value::String(any) if any == "any" => Ok(CrewLimit::Unlimited),
        Value::Integer(at_once) if at_once >= 0 => Ok(CrewLimit::limit(at_once as usize)),
        other => Err(content::unexpected(
            "crew limit",
            &["a count", &content::quoted("any")],
            &found(&other),
        )),
    }
}

/// Reads the berth group name and the stance of an `attached` presence.
fn attachment(attached: &Table) -> crate::Result<Attachment> {
    let berths = required::<String>(attached, "berths")?;
    let stance = berth_stance(&required::<Value>(attached, "stance")?)?;
    Ok(Attachment::new(berths, stance))
}

/// Reads an orphan fate: the keyword `"perish"` or `"linger"`, or a
/// `{ linger = ... }` table saying how the broodlings linger.
fn orphan_fate(value: &Value) -> crate::Result<OrphanFate> {
    keyword_or_table(
        "orphan fate",
        value,
        &[
            ("perish", OrphanFate::Perish),
            ("linger", OrphanFate::Linger(Lingering::Stay)),
        ],
        &["a { linger = { reseat = { distance = ... } } } table"],
        |table| {
            Ok(OrphanFate::Linger(lingering(&required::<Table>(
                table, "linger",
            )?)?))
        },
    )
}

/// Reads how set-down broodlings linger from a `{ reseat = { distance = ... } }`
/// table.
fn lingering(table: &Table) -> crate::Result<Lingering> {
    let reseat = required::<Table>(table, "reseat")?;
    Ok(Lingering::Reseat {
        distance: required::<u32>(&reseat, "distance")?,
    })
}

/// Reads a berth stance: `"still"`, `{ circling = { speed, dwell } }`,
/// `{ roaming = { speed, dwell } }`, or `{ orbit = { radius, period } }`.
fn berth_stance(value: &Value) -> crate::Result<BerthStance> {
    keyword_or_table(
        "berth stance",
        value,
        &[("still", BerthStance::Still)],
        &[
            "a { circling = ... } table",
            "a { roaming = ... } table",
            "an { orbit = ... } table",
        ],
        moving_berth_stance,
    )
}

/// Reads the table of a stance that keeps a worker on the move: exactly one of
/// `circling`, `roaming` or `orbit`.
fn moving_berth_stance(table: &Table) -> crate::Result<BerthStance> {
    let circling = optional::<Table>(table, "circling")?;
    let roaming = optional::<Table>(table, "roaming")?;
    let orbit = optional::<Table>(table, "orbit")?;
    match (circling, roaming, orbit) {
        (Some(circling), None, None) => {
            let (speed, dwell) = berth_hopping(&circling)?;
            Ok(BerthStance::Circling { speed, dwell })
        }
        (None, Some(roaming), None) => {
            let (speed, dwell) = berth_hopping(&roaming)?;
            Ok(BerthStance::Roaming { speed, dwell })
        }
        (None, None, Some(orbit)) => Ok(BerthStance::Orbit {
            radius: fixed_value("orbit radius", &required::<Value>(&orbit, "radius")?)?,
            period: required::<u32>(&orbit, "period")?,
        }),
        _ => Err(ScriptError::ContentError(
            "a berth stance table must have exactly one of 'circling', 'roaming' or 'orbit'"
                .to_string(),
        )),
    }
}

/// Reads the `speed` and `dwell` of a stance that moves between berths.
fn berth_hopping(table: &Table) -> crate::Result<(FixedU64, u32)> {
    Ok((
        fixed_value("berth-to-berth speed", &required::<Value>(table, "speed")?)?,
        required::<u32>(table, "dwell")?,
    ))
}

/// Reads a builder attendance: a keyword for one with no presence of its own,
/// or the table form of a [`work_presence`] for one that declares where it
/// stands.
fn builder_attendance(value: &Value) -> crate::Result<BuilderAttendance> {
    keyword_or_table(
        "builder attendance",
        value,
        &[
            ("unattended", BuilderAttendance::Unattended),
            ("consumed", BuilderAttendance::Consumed),
        ],
        &[
            "a { hidden = ... } table",
            "a { present = ... } table",
            "an { attached = ... } table",
        ],
        |table| Ok(BuilderAttendance::Crew(work_presence_table(table)?)),
    )
}

/// A required table field.
fn required<T: mlua::FromLua>(table: &Table, field: &str) -> crate::Result<T> {
    let value: T = table
        .get(field)
        .map_err(|error| field_error(field, error))?;
    Ok(value)
}

/// An optional table field, `None` when absent.
fn optional<T: mlua::FromLua>(table: &Table, field: &str) -> crate::Result<Option<T>> {
    let value: Option<T> = table
        .get(field)
        .map_err(|error| field_error(field, error))?;
    Ok(value)
}

/// A required boolean field.
///
/// Read through `Option` rather than as a plain `bool`, because Lua's `nil`
/// converts to `false` — a direct read cannot tell an absent flag from one that
/// was deliberately set to `false`.
fn required_flag(table: &Table, field: &str) -> crate::Result<bool> {
    optional::<bool>(table, field)?
        .ok_or_else(|| ScriptError::ContentError(format!("field '{field}' is required")))
}

/// A required positional (array) element.
fn index(table: &Table, position: i64) -> crate::Result<u32> {
    let value: i64 = table
        .get(position)
        .map_err(|error| ScriptError::ContentError(format!("element {position}: {error}")))?;
    dimension(value)
}

/// Narrows a Lua integer to a `u32` footprint dimension.
fn dimension(value: i64) -> crate::Result<u32> {
    u32::try_from(value)
        .map_err(|_| ScriptError::ContentError(format!("size {value} out of range")))
}

fn field_error(field: &str, error: mlua::Error) -> ScriptError {
    ScriptError::ContentError(format!("field '{field}': {error}"))
}

/// Reads one skill: `{ caster, cooldown, ... }` — the `caster` arm decides the
/// remaining fields. An entity cast reads `{ cost?, target, range?, effect }`;
/// a player cast reads `{ price?, effect }` and takes no target (the cast lands
/// on the casting player). A missing `cost` block (a player cast's `price`) is a free skill, and a
/// missing `range` a cast that lands from wherever the caster stands.
fn parse_skill(table: &Table, registry: &ContentRegistry) -> crate::Result<SkillDef> {
    let cooldown = required::<u32>(table, "cooldown")?;
    let caster = match required::<String>(table, "caster")?.as_str() {
        "entity" => SkillCaster::Entity {
            costs: match optional::<Table>(table, "cost")? {
                Some(cost) => parse_costs(&cost)?,
                None => Vec::new(),
            },
            target: parse_cast_target(&required::<Value>(table, "target")?)?,
            reach: parse_reach(table, registry)?,
            casting: parse_casting(table, registry)?,
            effect: parse_entity_effect(&required::<Table>(table, "effect")?, registry)?,
        },
        "player" => {
            if optional::<Value>(table, "target")?.is_some() {
                return Err(ScriptError::ContentError(
                    "a player-cast skill takes no target: the cast lands on the casting player"
                        .to_string(),
                ));
            }
            SkillCaster::Player {
                price: match optional::<Table>(table, "price")? {
                    Some(price) => pairs::<u32>(&price, "price")?.into_iter().collect(),
                    None => Price::new(),
                },
                effect: parse_player_effect(&required::<Table>(table, "effect")?, registry)?,
            }
        }
        other => {
            return Err(content::unexpected(
                "skill caster",
                &["'entity'", "'player'"],
                &content::quoted(other),
            ));
        }
    };
    let requires = parse_requires(table, registry)?;
    Ok(SkillDef {
        cooldown,
        caster,
        requires,
    })
}

/// Reads a research definition: a `price` table of resource amounts, the
/// research `time` in ticks, an optional player `buff` applied on completion,
/// and optional `requires` entries. The buff name resolves in the player-buff
/// registry.
fn parse_research(table: &Table, registry: &ContentRegistry) -> crate::Result<ResearchDef> {
    let price: Price = match optional::<Table>(table, "price")? {
        Some(price) => pairs::<u32>(&price, "price")?.into_iter().collect(),
        None => Price::new(),
    };
    let time = required::<u32>(table, "time")?;
    let buff = match optional::<String>(table, "buff")? {
        Some(name) => Some(registry.player_buff(&name).ok_or_else(|| {
            ScriptError::ContentError(format!("player buff '{name}' is not defined"))
        })?),
        None => None,
    };
    let requires = parse_requires(table, registry)?;
    Ok(ResearchDef::new(price, time, buff, requires))
}

/// Reads a `cost` block — a skill's, a change of form's or an upkeep's: any of
/// `resources` (a table of amounts), `energy`, and `health` (decimal strings),
/// each present entry one cost the entity pays.
fn parse_costs(cost: &Table) -> crate::Result<Vec<Cost>> {
    let mut costs = Vec::new();
    if let Some(resources) = optional::<Table>(cost, "resources")? {
        costs.push(Cost::Resources(
            pairs::<u32>(&resources, "resources")?.into_iter().collect(),
        ));
    }
    if let Some(energy) = optional::<String>(cost, "energy")? {
        costs.push(Cost::Energy(content::fixed(&energy)?));
    }
    if let Some(health) = optional::<String>(cost, "health")? {
        costs.push(Cost::Health(content::fixed(&health)?));
    }
    if costs.is_empty() {
        return Err(ScriptError::ContentError(
            "skill cost must name at least one of resources, energy, or health".to_string(),
        ));
    }
    Ok(costs)
}

/// Reads a number declared as a count of `unit`s or as `{ stat = ... }` naming
/// a registered entity stat.
fn parse_quantity(
    what: &str,
    unit: &str,
    value: &Value,
    registry: &ContentRegistry,
) -> crate::Result<Quantity> {
    match value {
        Value::Integer(count) => Ok(Quantity::Constant(u32::try_from(*count).map_err(|_| {
            ScriptError::ContentError(format!(
                "{what} {count} must be a non-negative {unit} count"
            ))
        })?)),
        Value::Table(named) => {
            let name = required::<String>(named, "stat")?;
            let stat = registry.entity_stat(&name).ok_or_else(|| {
                ScriptError::ContentError(format!("{what} stat '{name}' is not defined"))
            })?;
            Ok(Quantity::Stat(stat))
        }
        other => Err(content::unexpected(
            what,
            &[&format!("a {unit} count"), "a { stat = ... } table"],
            &found(other),
        )),
    }
}

/// Reads the `morphs` list: each entry names the destination type and the
/// terms — an optional `via` form worn while the change runs, `time` (a tick
/// count, or `{ stat = ... }` naming a registered entity stat), `placement`,
/// `cancel`, an optional `interrupted` defaulting to `reverts`, an optional
/// `reason` defaulting to `change`, an optional `cost` block shaped like a skill cost,
/// and an optional `requires` list.
fn parse_morphs(
    morphs: Vec<Table>,
    registry: &ContentRegistry,
) -> crate::Result<Vec<MorphTransition>> {
    let mut transitions = Vec::with_capacity(morphs.len());
    for entry in morphs {
        let into = required::<String>(&entry, "into")?;
        let via = optional::<String>(&entry, "via")?;
        let time = parse_quantity(
            "morph time",
            "tick",
            &required::<Value>(&entry, "time")?,
            registry,
        )?;
        let placement = content::morph_placement(&required::<String>(&entry, "placement")?)?;
        let cancel = content::morph_cancel(&required::<String>(&entry, "cancel")?)?;
        let interrupted = match optional::<String>(&entry, "interrupted")? {
            Some(name) => content::morph_interrupted(&name)?,
            None => MorphInterrupted::Reverts,
        };
        let reason = match optional::<String>(&entry, "reason")? {
            Some(name) => content::morph_reason(&name)?,
            None => MorphReason::Change,
        };
        let costs = match optional::<Table>(&entry, "cost")? {
            Some(cost) => parse_costs(&cost)?,
            None => Vec::new(),
        };
        let requires = parse_requires(&entry, registry)?;
        transitions.push(MorphTransition::new(
            into,
            via.as_deref(),
            time,
            placement,
            cancel,
            interrupted,
            reason,
            costs,
            requires,
        ));
    }
    Ok(transitions)
}

/// Reads how long a cast holds its caster: `cast = { point = ..., period = ... }`
/// in ticks or as `{ stat = ... }` tables, or nothing at all for a cast that
/// lands at once. An omitted `period` is the point itself — no recovery.
fn parse_casting(table: &Table, registry: &ContentRegistry) -> crate::Result<Casting> {
    let Some(cast) = optional::<Table>(table, "cast")? else {
        return Ok(Casting::Instant);
    };
    let point = parse_quantity(
        "cast point",
        "tick",
        &required::<Value>(&cast, "point")?,
        registry,
    )?;
    let period = match optional::<Value>(&cast, "period")? {
        Some(period) => parse_quantity("cast period", "tick", &period, registry)?,
        None => point,
    };
    Ok(Casting::Delayed { point, period })
}

/// Reads how close a cast is made from: a cell count, a `{ stat = ... }` table
/// naming a registered entity stat, or nothing at all for a cast that lands
/// from wherever the caster stands.
fn parse_reach(table: &Table, registry: &ContentRegistry) -> crate::Result<Reach> {
    match optional::<Value>(table, "range")? {
        None => Ok(Reach::Wherever),
        Some(range) => Ok(Reach::Within(parse_quantity(
            "cast range",
            "cell",
            &range,
            registry,
        )?)),
    }
}

/// Reads an entity cast's effect: exactly one of `apply_buff`, `remove_buff`,
/// `damage`, `heal`, `field`, `watch`, `summon`. Buff names resolve in the
/// entity-buff registry, field names in the field registry, and a summoned type
/// in the entity registry.
fn parse_entity_effect(
    table: &Table,
    registry: &ContentRegistry,
) -> crate::Result<EntityCastEffect> {
    if let Some(name) = optional::<String>(table, "apply_buff")? {
        let id = registry.entity_buff(&name).ok_or_else(|| {
            ScriptError::ContentError(format!("entity buff '{name}' is not defined"))
        })?;
        Ok(EntityCastEffect::ApplyBuff(id))
    } else if let Some(name) = optional::<String>(table, "remove_buff")? {
        let id = registry.entity_buff(&name).ok_or_else(|| {
            ScriptError::ContentError(format!("entity buff '{name}' is not defined"))
        })?;
        Ok(EntityCastEffect::RemoveBuff(id))
    } else if let Some(amount) = optional::<String>(table, "damage")? {
        Ok(EntityCastEffect::Damage(content::fixed(&amount)?))
    } else if let Some(amount) = optional::<String>(table, "heal")? {
        Ok(EntityCastEffect::Heal(content::fixed(&amount)?))
    } else if let Some(field) = optional::<Table>(table, "field")? {
        Ok(EntityCastEffect::Field {
            field: field_id(&field, registry)?,
            radius: required::<u32>(&field, "radius")?,
            action: content::field_action(&required::<String>(&field, "action")?)?,
        })
    } else if let Some(watch) = optional::<Table>(table, "watch")? {
        Ok(EntityCastEffect::Watch {
            radius: required::<u32>(&watch, "radius")?,
            duration: required::<u32>(&watch, "duration")?,
            detection: parse_detection(&watch)?,
        })
    } else if let Some(summon) = optional::<Table>(table, "summon")? {
        let name = required::<String>(&summon, "entity")?;
        let entity_type = registry.type_id(&name).ok_or_else(|| {
            ScriptError::ContentError(format!("entity type '{name}' is not defined"))
        })?;
        Ok(EntityCastEffect::Summon {
            entity_type,
            count: required::<u32>(&summon, "count")?,
        })
    } else {
        Err(ScriptError::ContentError(
            "skill effect must be one of apply_buff, remove_buff, damage, heal, field, watch, or summon"
                .to_string(),
        ))
    }
}

/// Reads a field definition: the `layer` its cells lie on — `"anywhere"`
/// or the mask they must be passable on — its
/// `decay` — `"instant"`, `"never"`, or a `{ cycle = ticks }` table — its
/// `vision` — `"dark"` unless declared, or `"watched"` to put covered cells in
/// sight of whoever covers them — and its `detection`, the layer mask of what
/// it reveals, blind unless declared (a watch's `detection` reads the same).
fn parse_field(table: &Table) -> crate::Result<FieldDef> {
    let layer = parse_field_layer(required::<Value>(table, "layer")?)?;
    let decay = parse_field_decay(required::<Value>(table, "decay")?)?;
    let vision = match optional::<String>(table, "vision")? {
        Some(name) => content::field_vision(&name)?,
        None => FieldVision::Dark,
    };
    Ok(FieldDef::new(layer, decay, vision, parse_detection(table)?))
}

/// Reads where a field lies: `"anywhere"`, or the layer mask its cells must
/// be passable on.
fn parse_field_layer(value: Value) -> crate::Result<FieldLayer> {
    match &value {
        Value::String(name) if name == "anywhere" => Ok(FieldLayer::Anywhere),
        Value::Integer(layers) => Ok(FieldLayer::Passable(LayerMask::from(
            u32::try_from(*layers).map_err(|_| {
                content::unexpected("field layer", &["a layer mask"], &layers.to_string())
            })?,
        ))),
        other => Err(content::unexpected(
            "field layer",
            &["'anywhere'", "a layer mask"],
            &found(other),
        )),
    }
}

/// Reads what a cover detects: its `detection` layer mask, or blind when none
/// is declared.
fn parse_detection(table: &Table) -> crate::Result<Detection> {
    match optional::<u32>(table, "detection")? {
        None => Ok(Detection::Blind),
        Some(layers) => Ok(Detection::Reveals(LayerMask::from(layers))),
    }
}

/// Reads a field's decay: `"instant"`, `"never"`, or `{ cycle = ticks }`.
fn parse_field_decay(value: Value) -> crate::Result<FieldDecay> {
    match &value {
        Value::String(name) if name == "instant" => Ok(FieldDecay::Instant),
        Value::String(name) if name == "never" => Ok(FieldDecay::Never),
        Value::Table(gradual) => Ok(FieldDecay::Gradual {
            cycle: required::<u32>(gradual, "cycle")?,
        }),
        other => Err(content::unexpected(
            "field decay",
            &["'instant'", "'never'", "a { cycle = ... } table"],
            &found(other),
        )),
    }
}

/// Reads a source's growth: `"instant"` or `{ cycle = ticks, initial_radius =
/// cells }`.
fn parse_field_growth(value: Value) -> crate::Result<FieldGrowth> {
    match &value {
        Value::String(name) if name == "instant" => Ok(FieldGrowth::Instant),
        Value::Table(gradual) => Ok(FieldGrowth::Gradual {
            cycle: required::<u32>(gradual, "cycle")?,
            initial_radius: required::<u32>(gradual, "initial_radius")?,
        }),
        other => Err(content::unexpected(
            "field growth",
            &["'instant'", "a { cycle = ..., initial_radius = ... } table"],
            &found(other),
        )),
    }
}

/// Reads what a source projects while idle, declared under `what`: `"full"`,
/// `"nothing"`, or `{ held = cells }`.
fn parse_emission(what: &str, value: Value) -> crate::Result<Emission> {
    keyword_or_table(
        what,
        &value,
        &[("full", Emission::Full), ("nothing", Emission::Nothing)],
        &["a { held = cells } table"],
        |held| Ok(Emission::Held(required::<u32>(held, "held")?)),
    )
}

/// Reads one effect a buff or a field has on an entity: `"disable"`,
/// `"conceal"`, or `{ modifiers = { ... } }`.
fn parse_entity_effect_kind(
    value: Value,
    registry: &ContentRegistry,
) -> crate::Result<EntityEffect> {
    match &value {
        Value::String(name) if name == "disable" => Ok(EntityEffect::Disable),
        Value::String(name) if name == "conceal" => Ok(EntityEffect::Conceal),
        Value::Table(table) => Ok(EntityEffect::Modifiers(parse_entity_modifiers(
            &required::<Vec<Table>>(table, "modifiers")?,
            registry,
        )?)),
        other => Err(content::unexpected(
            "entity effect",
            &["'disable'", "'conceal'", "a { modifiers = ... } table"],
            &found(other),
        )),
    }
}

/// Reads how long a buff lasts: `"forever"`, `{ ticks = n }`, or
/// `{ upkeep = { cost = <cost table>, period = ticks } }` — the cost table a
/// skill's `cost` takes, paid once every `period` ticks.
fn parse_lasting(value: Value) -> crate::Result<Lasting> {
    match &value {
        Value::String(name) if name == "forever" => Ok(Lasting::Forever),
        Value::Table(table) => match (
            optional::<u32>(table, "ticks")?,
            optional::<Table>(table, "upkeep")?,
        ) {
            (Some(ticks), None) => Ok(Lasting::For(ticks)),
            (None, Some(upkeep)) => Ok(Lasting::Upkeep {
                costs: parse_costs(&required::<Table>(&upkeep, "cost")?)?,
                period: required::<u32>(&upkeep, "period")?,
            }),
            (Some(_), Some(_)) | (None, None) => Err(ScriptError::ContentError(
                "a lasting table names exactly one of ticks or upkeep".to_string(),
            )),
        },
        other => Err(content::unexpected(
            "lasting",
            &[
                "'forever'",
                "a { ticks = ... } table",
                "a { upkeep = ... } table",
            ],
            &found(other),
        )),
    }
}

/// A Lua value as an error message names it: a string by its quoted text,
/// anything else by its type.
fn found(value: &Value) -> String {
    match value {
        Value::String(text) => content::quoted(&text.to_string_lossy()),
        other => other.type_name().to_string(),
    }
}

/// Resolves the `field` entry of `table` in the field registry.
fn field_id(table: &Table, registry: &ContentRegistry) -> crate::Result<FieldId> {
    field_by_name(&required::<String>(table, "field")?, registry)
}

/// Resolves a field by the name content gave it.
fn field_by_name(name: &str, registry: &ContentRegistry) -> crate::Result<FieldId> {
    registry
        .field(name)
        .ok_or_else(|| ScriptError::ContentError(format!("field '{name}' is not defined")))
}

/// Reads the `field_sources` list: each entry names a `field`, a `radius`, a
/// `growth` — `"instant"` or `{ cycle = ticks, initial_radius = cells }` — and
/// what it projects `while_constructing` and `while_disabled`: `"full"`,
/// `"nothing"`, or `{ held = cells }`.
fn parse_field_sources(
    sources: &[Table],
    registry: &ContentRegistry,
) -> crate::Result<Vec<FieldSourceDef>> {
    sources
        .iter()
        .map(|entry| {
            let growth = parse_field_growth(required::<Value>(entry, "growth")?)?;
            Ok(FieldSourceDef::new(
                field_id(entry, registry)?,
                required::<u32>(entry, "radius")?,
                growth,
                parse_emission(
                    "while_constructing",
                    required::<Value>(entry, "while_constructing")?,
                )?,
                parse_emission(
                    "while_disabled",
                    required::<Value>(entry, "while_disabled")?,
                )?,
            ))
        })
        .collect()
}

/// Reads the `on_stand` list: each entry is one act, named by its one verb
/// key — `field = { field, radius, action }` covers or clears a field around
/// the footprint.
fn parse_on_stand(acts: &[Table], registry: &ContentRegistry) -> crate::Result<Vec<StandingAct>> {
    acts.iter()
        .map(|entry| {
            if let Some(field) = optional::<Table>(entry, "field")? {
                Ok(StandingAct::Field {
                    field: field_id(&field, registry)?,
                    radius: required::<u32>(&field, "radius")?,
                    action: content::field_action(&required::<String>(&field, "action")?)?,
                })
            } else {
                Err(ScriptError::ContentError(
                    "standing act must carry a field = { ... } table".to_string(),
                ))
            }
        })
        .collect()
}

/// Reads the `docks` list: each entry the cell offset `at` its annex stands
/// on and the annex types it `accepts`.
fn parse_docks(docks: Vec<Table>) -> crate::Result<Vec<(CellPos, Kinds)>> {
    let mut out = Vec::with_capacity(docks.len());
    for dock in docks {
        let at = required::<Table>(&dock, "at")?;
        let at = CellPos::new(
            at.get::<u32>(1)
                .map_err(|error| field_error("dock x", error))?,
            at.get::<u32>(2)
                .map_err(|error| field_error("dock y", error))?,
        );
        out.push((
            at,
            parse_kinds(&required::<Value>(&dock, "accepts")?, "dock accepts")?,
        ));
    }
    Ok(out)
}

/// Reads an `annex` table: what it does `alone`, and who may `claim` it.
fn parse_annex(table: &Table) -> crate::Result<(AloneConduct, AnnexClaim)> {
    let alone = parse_annex_alone(required::<Value>(table, "alone")?)?;
    let claim = content::annex_claim(&required::<String>(table, "claim")?)?;
    Ok((alone, claim))
}

/// Reads what an annex with no primary does: `"razed"`, or a table naming its
/// `work` and its `life`.
fn parse_annex_alone(value: Value) -> crate::Result<AloneConduct> {
    match &value {
        Value::String(name) if name == "razed" => Ok(AloneConduct::Razed),
        Value::Table(standing) => Ok(AloneConduct::Standing {
            work: content::annex_work(&required::<String>(standing, "work")?)?,
            life: parse_annex_life(required::<Value>(standing, "life")?)?,
        }),
        other => Err(content::unexpected(
            "what an annex does alone",
            &[
                &content::quoted("razed"),
                "a { work = ..., life = ... } table",
            ],
            &found(other),
        )),
    }
}

/// Reads what time does to an annex with no primary: `"endures"`, or
/// `{ fades = health per tick }`.
fn parse_annex_life(value: Value) -> crate::Result<AnnexLife> {
    match &value {
        Value::String(name) if name == "endures" => Ok(AnnexLife::Endures),
        Value::Table(fading) => Ok(AnnexLife::Fades {
            per_tick: content::fixed(&required::<String>(fading, "fades")?)?,
        }),
        other => Err(content::unexpected(
            "an annex life",
            &[&content::quoted("endures"), "a { fades = ... } table"],
            &found(other),
        )),
    }
}

/// Reads a `requires` list: each entry a table naming exactly one kind —
/// `{ entity_type = "armory" }`, `{ tag = "workshop" }`,
/// `{ research = "smithing" }` or `{ annexed = "tech_lab" }`.
fn parse_requires(table: &Table, registry: &ContentRegistry) -> crate::Result<Vec<Requirement>> {
    let Some(entries) = optional::<Vec<Value>>(table, "requires")? else {
        return Ok(Vec::new());
    };
    entries
        .into_iter()
        .map(|entry| match &entry {
            Value::Table(entry) => parse_requirement(entry, registry),
            other => Err(content::unexpected(
                "a requirement",
                &[
                    "an { entity_type = ... } table",
                    "a { tag = ... } table",
                    "a { research = ... } table",
                    "an { annexed = ... } table",
                ],
                &found(other),
            )),
        })
        .collect()
}

/// Reads one requirement entry, which names exactly one kind.
fn parse_requirement(entry: &Table, registry: &ContentRegistry) -> crate::Result<Requirement> {
    // The shape is judged before any name is resolved, so an entry naming two
    // kinds reads as the shape error it is rather than as a failed lookup.
    match (
        optional::<String>(entry, "entity_type")?,
        optional::<String>(entry, "tag")?,
        optional::<String>(entry, "research")?,
        optional::<String>(entry, "annexed")?,
    ) {
        (Some(name), None, None, None) => Ok(Requirement::EntityType(name)),
        (None, Some(name), None, None) => Ok(Requirement::Tag(name)),
        (None, None, Some(name), None) => {
            Ok(Requirement::Research(research_by_name(&name, registry)?))
        }
        (None, None, None, Some(name)) => Ok(Requirement::Annexed(name)),
        // Every other shape names none of the four kinds, or more than one.
        _ => Err(ScriptError::ContentError(
            "a requirement must name exactly one of entity_type, tag, research, or annexed"
                .to_string(),
        )),
    }
}

/// Resolves a research by the name content gave it.
fn research_by_name(name: &str, registry: &ContentRegistry) -> crate::Result<ResearchId> {
    registry
        .research(name)
        .ok_or_else(|| ScriptError::ContentError(format!("research '{name}' is not defined")))
}

/// Reads the `field_placement` list: each entry either `requires` a field
/// (with `of` and `coverage`) or `forbids` one.
fn parse_field_placement(
    rules: &[Table],
    registry: &ContentRegistry,
) -> crate::Result<Vec<FieldPlacement>> {
    rules
        .iter()
        .map(|entry| {
            let resolve = |name: String| field_by_name(&name, registry);
            match (
                optional::<String>(entry, "requires")?,
                optional::<String>(entry, "forbids")?,
            ) {
                (Some(name), None) => Ok(FieldPlacement::Requires {
                    field: resolve(name)?,
                    of: content::affiliation(&required::<String>(entry, "of")?)?,
                    coverage: content::field_coverage(&required::<String>(entry, "coverage")?)?,
                }),
                (None, Some(name)) => Ok(FieldPlacement::Forbids {
                    field: resolve(name)?,
                }),
                (Some(_), Some(_)) | (None, None) => Err(ScriptError::ContentError(
                    "a field placement rule names exactly one of requires or forbids".to_string(),
                )),
            }
        })
        .collect()
}

/// Reads the `field_effects` list: each entry names a `field` and `of`, and
/// exactly one of `inside` or `outside` holding an entity effect —
/// `{ modifiers = {...} }`, `"disable"`, or `"conceal"`.
fn parse_field_effects(
    effects: &[Table],
    registry: &ContentRegistry,
) -> crate::Result<Vec<FieldEffect>> {
    effects
        .iter()
        .map(|entry| {
            let (side, value) = match (
                optional::<Value>(entry, "inside")?,
                optional::<Value>(entry, "outside")?,
            ) {
                (Some(value), None) => (FieldSide::Inside, value),
                (None, Some(value)) => (FieldSide::Outside, value),
                (Some(_), Some(_)) | (None, None) => {
                    return Err(ScriptError::ContentError(
                        "a field effect names exactly one of inside or outside".to_string(),
                    ));
                }
            };
            let kind = parse_entity_effect_kind(value, registry)?;
            Ok(FieldEffect::new(
                field_id(entry, registry)?,
                content::affiliation(&required::<String>(entry, "of")?)?,
                side,
                content::field_coverage(&required::<String>(entry, "coverage")?)?,
                kind,
            ))
        })
        .collect()
}

/// Reads a player cast's effect: exactly one of `apply_buff` and
/// `remove_buff`. Buff names resolve in the player-buff registry.
fn parse_player_effect(
    table: &Table,
    registry: &ContentRegistry,
) -> crate::Result<PlayerCastEffect> {
    if let Some(name) = optional::<String>(table, "apply_buff")? {
        let id = registry.player_buff(&name).ok_or_else(|| {
            ScriptError::ContentError(format!("player buff '{name}' is not defined"))
        })?;
        Ok(PlayerCastEffect::ApplyBuff(id))
    } else if let Some(name) = optional::<String>(table, "remove_buff")? {
        let id = registry.player_buff(&name).ok_or_else(|| {
            ScriptError::ContentError(format!("player buff '{name}' is not defined"))
        })?;
        Ok(PlayerCastEffect::RemoveBuff(id))
    } else {
        Err(ScriptError::ContentError(
            "player-cast skill effect must be one of apply_buff or remove_buff".to_string(),
        ))
    }
}

/// Reads an entity buff definition: `{ effects?, lasting, stack, interrupted_by? }`.
/// Each effect is read as a field effect's is; `interrupted_by` lists interruptions.
fn parse_entity_buff(table: &Table, registry: &ContentRegistry) -> crate::Result<EntityBuffDef> {
    let effects = match optional::<Vec<Value>>(table, "effects")? {
        Some(values) => values
            .into_iter()
            .map(|value| parse_entity_effect_kind(value, registry))
            .collect::<crate::Result<Vec<_>>>()?,
        None => Vec::new(),
    };
    let interrupted_by = match optional::<Vec<String>>(table, "interrupted_by")? {
        Some(names) => names
            .iter()
            .map(|name| content::interruption(name))
            .collect::<crate::Result<Vec<_>>>()?,
        None => Vec::new(),
    };
    Ok(EntityBuffDef {
        effects,
        lasting: parse_lasting(required::<Value>(table, "lasting")?)?,
        stack_rule: content::stack_rule(&required::<String>(table, "stack")?)?,
        interrupted_by,
    })
}

/// Reads a player buff definition: `{ duration?, stack, player_modifiers?,
/// entity_modifiers? }` — at least one modifier list must be present.
fn parse_player_buff(table: &Table, registry: &ContentRegistry) -> crate::Result<PlayerBuffDef> {
    let player_modifiers = match optional::<Vec<Table>>(table, "player_modifiers")? {
        Some(modifiers) => parse_player_modifiers(&modifiers, registry)?,
        None => Vec::new(),
    };
    let entity_modifiers = match optional::<Vec<Table>>(table, "entity_modifiers")? {
        Some(modifiers) => parse_entity_modifiers(&modifiers, registry)?,
        None => Vec::new(),
    };
    if player_modifiers.is_empty() && entity_modifiers.is_empty() {
        return Err(ScriptError::ContentError(
            "player buff must declare player_modifiers or entity_modifiers".to_string(),
        ));
    }
    Ok(PlayerBuffDef {
        player_modifiers,
        entity_modifiers,
        duration: optional::<u32>(table, "duration")?,
        stack_rule: content::stack_rule(&required::<String>(table, "stack")?)?,
    })
}

fn parse_entity_modifiers(
    modifiers: &[Table],
    registry: &ContentRegistry,
) -> crate::Result<Vec<EntityModifier>> {
    modifiers
        .iter()
        .map(|modifier| parse_entity_modifier(modifier, registry))
        .collect()
}

fn parse_player_modifiers(
    modifiers: &[Table],
    registry: &ContentRegistry,
) -> crate::Result<Vec<PlayerModifier>> {
    modifiers
        .iter()
        .map(|modifier| parse_player_modifier(modifier, registry))
        .collect()
}

/// Reads one entity modifier: `{ entity_stat, op, value }`.
fn parse_entity_modifier(
    table: &Table,
    registry: &ContentRegistry,
) -> crate::Result<EntityModifier> {
    if optional::<Value>(table, "player_stat")?.is_some() {
        return Err(ScriptError::ContentError(
            "this modifier list holds entity modifiers; expected entity_stat, found player_stat"
                .to_string(),
        ));
    }
    let name = required::<String>(table, "entity_stat")?;
    let stat = registry
        .entity_stat(&name)
        .ok_or_else(|| ScriptError::ContentError(format!("entity stat '{name}' is not defined")))?;
    let (op, magnitude) = parse_modifier_op_value(table)?;
    Ok(EntityModifier {
        stat,
        op,
        magnitude,
    })
}

/// Reads one player modifier: `{ player_stat, op, value }`.
fn parse_player_modifier(
    table: &Table,
    registry: &ContentRegistry,
) -> crate::Result<PlayerModifier> {
    if optional::<Value>(table, "entity_stat")?.is_some() {
        return Err(ScriptError::ContentError(
            "this modifier list holds player modifiers; expected player_stat, found entity_stat"
                .to_string(),
        ));
    }
    let name = required::<String>(table, "player_stat")?;
    let stat = registry
        .player_stat(&name)
        .ok_or_else(|| ScriptError::ContentError(format!("player stat '{name}' is not defined")))?;
    let (op, magnitude) = parse_modifier_op_value(table)?;
    Ok(PlayerModifier {
        stat,
        op,
        magnitude,
    })
}

/// Reads a modifier's `op` and `value` fields, shared by both modifier kinds.
fn parse_modifier_op_value(table: &Table) -> crate::Result<(ModifierOp, FixedI64)> {
    let op = content::modifier_op(&required::<String>(table, "op")?)?;
    let value = FixedI64::from_str(&required::<String>(table, "value")?)
        .map_err(|error| ScriptError::ContentError(format!("invalid modifier value: {error}")))?;
    Ok((op, value))
}
