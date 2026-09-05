<div align="center">

<img src="docs/title.svg" alt="ferrets" width="180" />

**A deterministic RTS engine that ferrets out every last desync.**

[![CI](https://github.com/valeriyr/ferrets/actions/workflows/ci.yml/badge.svg?branch=main&event=push)](https://github.com/valeriyr/ferrets/actions/workflows/ci.yml)
[![Rust 1.94+](https://img.shields.io/badge/rust-1.94%2B-orange?logo=rust)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

</div>

## What is ferrets?

**ferrets** is a real-time strategy game engine built in Rust on top of [Bevy](https://bevy.org/). The simulation is fully deterministic — given the same player inputs, every machine computes exactly the same game state. That one property is what the whole engine is organized around: multiplayer only has to exchange player commands, and a replay is just the recorded command stream played back through the same simulation.

## How it works

**Deterministic simulation** — The game world advances on a fixed tick using fixed-point math instead of floats, so results are identical across machines and platforms. Units, orders, combat, construction, harvesting, fog of war, stats, buffs, and skills all live inside this deterministic core, and state checksums catch any divergence early.

**Lockstep networking** — Players exchange only their commands, never game state. Sessions run over TCP or UDP in either a peer-to-peer mesh or a host-relayed topology, with frame redundancy and drop handling to keep the lockstep moving.

**Scripted content and AI** — Game content — units, buildings, terrain, stats, buffs, skills, projectiles — is defined in Lua and loaded through a scripting seam that keeps the simulation independent of the scripting runtime. AI players are Lua scripts too: they observe an integer-only view of the world and issue the same commands a human player would.

**Pathfinding** — Movement runs on a deterministic grid-based pathfinder over a navigation grid seeded from terrain and building occupancy.

**Replays** — Every session is streamed to a replay file as it runs, so a recording survives a crash. Playback feeds the recorded commands back through the simulation, and embedded checksums verify the replay stays in sync while it plays.

**Rendering** — The simulation core knows nothing about graphics. `ferrets-bevy-plugin` drives it from a Bevy app, and `ferrets-demo` is a playable demo game that exercises every engine feature.

## Build & test

```bash
cargo build
cargo test
```

Run the demo game:

```bash
cargo run --bin demo
```

## Workspace

```text
crates/
├── ferrets-bevy-plugin  Bevy integration for the ferrets simulation
├── ferrets-content      Content vocabulary and registry for the game engine
├── ferrets-geometry     Cell-grid geometry and the projection distance metrics
├── ferrets-math         Fixed-point scalars, vectors, rectangles and angles
├── ferrets-network      Lockstep P2P networking for deterministic multiplayer
├── ferrets-pathfinder   Deterministic RTS pathfinding
├── ferrets-physics      Contact resolution for continuous-model unit bodies
├── ferrets-replay       Replay recording and deterministic playback
├── ferrets-script       Lua scripting runtime and game content loading
├── ferrets-simulation   Deterministic RTS simulation core
└── ferrets-steam        Steam platform integration — achievements and P2P transport
ferrets-demo             Playable demo game built on the engine
```

## License

ferrets is free and open source. All code in this repository is dual-licensed under your choice of:

- [MIT License](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project shall be dual-licensed as above, without any additional terms or conditions.
