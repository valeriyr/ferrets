//! The lobby screen: configure slots, pick a topology, and start the game.
//!
//! For a local game the [`LobbyConfig`] is the source of truth, edited directly.
//! For a network game the host coordinates: the [`LobbyLink`] holds the
//! [`LobbyHost`]/[`LobbyClient`], the authoritative state is mirrored into the
//! config each frame for display, and edits go through the host.

use std::net::SocketAddr;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use ferrets_bevy::{install_game_resources, install_network_session};
use ferrets_network::lobby::client::{LobbyClient, PollOutcome};
use ferrets_network::message::control::Occupant;
use ferrets_network::session::NetSession;
use ferrets_network::topology::Topology;
use ferrets_network::{bootstrap, lobby::host::LobbyHost};
use ferrets_simulation::session::{
    GameSession,
    ai_hosting::AiHosting,
    player_slot::{PlayerId, PlayerSlot},
    player_type::PlayerType,
};

use crate::map::START_POINTS;
use crate::states::{GameState, LobbyMode};

/// Player-slot capacity, one per map start point.
const SLOTS: usize = START_POINTS.len();
/// The TCP port the host binds and clients dial.
const TCP_PORT: u16 = 4000;
/// The UDP port used for gameplay in a mesh game (per machine).
const UDP_PORT: u16 = 4001;

const NORMAL: Color = Color::srgb(0.20, 0.20, 0.24);
const HOVERED: Color = Color::srgb(0.30, 0.30, 0.38);

//
// ─── Resources ────────────────────────────────────────────────────────────────
//

/// A race a slot can play.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Race {
    Human,
    Orc,
}

impl Race {
    fn id(self) -> &'static str {
        match self {
            Race::Human => "human",
            Race::Orc => "orc",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Race::Human => "Humans",
            Race::Orc => "Orcs",
        }
    }

    fn toggled(self) -> Race {
        match self {
            Race::Human => Race::Orc,
            Race::Orc => Race::Human,
        }
    }

    fn from_id(id: Option<&str>) -> Race {
        match id {
            Some("orc") => Race::Orc,
            _ => Race::Human,
        }
    }
}

/// What occupies a slot, from this node's point of view.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// Empty, waiting for a human (network only).
    Open,
    /// A human (the local player, or a connected remote one).
    Human,
    /// A local AI.
    Ai,
    /// Disabled.
    Closed,
}

impl SlotKind {
    fn label(self) -> &'static str {
        match self {
            SlotKind::Open => "Open",
            SlotKind::Human => "Human",
            SlotKind::Ai => "AI",
            SlotKind::Closed => "Closed",
        }
    }
}

/// One slot's configuration as shown in the lobby.
#[derive(Clone, Copy)]
pub struct SlotView {
    pub kind: SlotKind,
    pub race: Race,
}

/// The lobby's working configuration.
#[derive(Resource)]
pub struct LobbyConfig {
    pub slots: Vec<SlotView>,
    pub topology: Topology,
    pub ai_hosting: AiHosting,
    /// Which slot the local human controls (local games).
    pub local_slot: PlayerId,
    /// The host address a client dials: `ip` or `ip:port` (the port defaults
    /// to [`TCP_PORT`] when omitted).
    pub host_addr: String,
    /// A line of status/help shown under the title.
    pub status: String,
}

impl LobbyConfig {
    fn for_mode(mode: LobbyMode) -> Self {
        let slots = (0..SLOTS)
            .map(|i| SlotView {
                kind: match (mode, i) {
                    (LobbyMode::Local, 0) | (LobbyMode::Host, 0) => SlotKind::Human,
                    (LobbyMode::Local, _) => SlotKind::Ai,
                    (LobbyMode::Host, _) => SlotKind::Open,
                    (LobbyMode::Client, _) => SlotKind::Open,
                },
                race: if i % 2 == 0 { Race::Human } else { Race::Orc },
            })
            .collect();
        Self {
            slots,
            topology: Topology::HostStar,
            ai_hosting: AiHosting::HostOnly,
            local_slot: 0,
            host_addr: "127.0.0.1".to_string(),
            status: String::new(),
        }
    }
}

/// The live network lobby handle (network modes only).
pub enum LobbyLink {
    Host(LobbyHost),
    Client(LobbyClient),
}

/// A connection attempt running off the main thread — dialing blocks on DNS,
/// the TCP connect, and the host's id assignment, and a silently-dropped dial
/// can stall for the OS timeout, so none of it may run on the UI thread.
pub struct PendingConnect {
    /// The field contents the attempt dialed; a mismatch means the user kept
    /// typing and the result is stale.
    addr: String,
    result: Receiver<ferrets_network::Result<LobbyClient>>,
}

/// Set by the Start button; consumed by the exclusive `start_game` system.
#[derive(Resource)]
struct StartRequested;

//
// ─── UI components ────────────────────────────────────────────────────────────
//

#[derive(Component)]
pub struct LobbyRoot;

#[derive(Component, Clone, Copy)]
pub enum LobbyButton {
    Kind(u8),
    Race(u8),
    Claim(u8),
    Topology,
    AiHosting,
    Back,
    Start,
}

#[derive(Component)]
pub struct StatusText;

#[derive(Component)]
pub struct TopologyText;

#[derive(Component)]
pub struct AiHostingText;

#[derive(Component)]
pub struct AddrText;

#[derive(Component)]
pub struct SlotText(u8);

//
// ─── Setup / teardown ──────────────────────────────────────────────────────────
//

/// Builds the lobby config and (for network modes) opens the connection.
pub fn enter_lobby(mut commands: Commands, mode: Res<LobbyMode>) {
    let mut config = LobbyConfig::for_mode(*mode);
    match *mode {
        LobbyMode::Host => match open_host(config.topology) {
            Ok(host) => {
                config.status =
                    format!("hosting on port {TCP_PORT} - clients dial this machine's ip");
                commands.queue(move |world: &mut World| {
                    world.insert_non_send_resource(LobbyLink::Host(host));
                });
            }
            Err(error) => {
                config.status = format!("host failed: {error}");
            }
        },
        LobbyMode::Client => {} // The client auto-connects while in the lobby.
        LobbyMode::Local => {}
    }
    commands.insert_resource(config);
}

/// Tears the lobby down, dropping any live connection.
pub fn exit_lobby(mut commands: Commands, roots: Query<Entity, With<LobbyRoot>>) {
    for entity in &roots {
        commands.entity(entity).despawn();
    }
    commands.queue(|world: &mut World| {
        world.remove_non_send_resource::<LobbyLink>();
        world.remove_non_send_resource::<PendingConnect>();
    });
    commands.remove_resource::<StartRequested>();
}

fn open_host(topology: Topology) -> ferrets_network::Result<LobbyHost> {
    // The demo defaults to host-only AI; the lobby button toggles it.
    bootstrap::open_lobby(
        ("0.0.0.0", TCP_PORT),
        topology,
        AiHosting::HostOnly,
        SLOTS,
        Race::Human.id(),
    )
}

//
// ─── Live network sync ─────────────────────────────────────────────────────────
//

/// Polls the network lobby and mirrors its authoritative state into the config so
/// the view reflects connections and host edits live. A client that receives the
/// host's start signal requests the start itself.
pub fn poll_lobby_link(
    mut commands: Commands,
    link: Option<NonSendMut<LobbyLink>>,
    mut config: ResMut<LobbyConfig>,
    mut next: ResMut<NextState<GameState>>,
) {
    let Some(mut link) = link else { return };
    match &mut *link {
        LobbyLink::Host(host) => {
            // Process joins/requests (re-broadcasting on change), then always
            // mirror the authoritative state so the host's own edits show too.
            let _ = host.poll();
            mirror(
                &mut config,
                host.slots(),
                host.topology(),
                host.ai_hosting(),
            );
        }
        LobbyLink::Client(client) => match client.poll() {
            PollOutcome::Waiting { changed } => {
                if changed && !client.slots().is_empty() {
                    mirror(
                        &mut config,
                        client.slots(),
                        client.topology(),
                        client.ai_hosting(),
                    );
                    config.status = "connected".to_string();
                }
                if let Some(slot) = client.local_player() {
                    config.local_slot = slot;
                }
                if client.started().is_some() {
                    config.status = "starting...".to_string();
                    commands.insert_resource(StartRequested);
                }
            }
            // The host refused this client (e.g. a build mismatch) or is gone; the
            // lobby can't continue. Back to the menu (OnExit(Lobby) drops the link).
            PollOutcome::Rejected(reason) => {
                eprintln!("host refused the connection: {reason}");
                next.set(GameState::Menu);
            }
            PollOutcome::HostLost => next.set(GameState::Menu),
        },
    }
}

fn mirror(
    config: &mut LobbyConfig,
    slots: &[ferrets_network::message::control::SlotInfo],
    topology: Topology,
    ai_hosting: AiHosting,
) {
    config.slots = slots
        .iter()
        .map(|info| SlotView {
            kind: match info.occupant {
                Occupant::Open => SlotKind::Open,
                Occupant::Human { .. } => SlotKind::Human,
                Occupant::Ai => SlotKind::Ai,
                Occupant::Closed => SlotKind::Closed,
            },
            race: Race::from_id(info.race.as_deref()),
        })
        .collect();
    config.topology = topology;
    config.ai_hosting = ai_hosting;
}

//
// ─── Input ────────────────────────────────────────────────────────────────────
//

/// Captures typing into the host-address field (client mode, before connecting):
/// addresses, host names, and an optional `:port`.
pub fn lobby_addr_input(
    mode: Res<LobbyMode>,
    link: Option<NonSend<LobbyLink>>,
    mut input: MessageReader<KeyboardInput>,
    mut config: ResMut<LobbyConfig>,
) {
    if *mode != LobbyMode::Client || link.is_some() {
        return;
    }
    for event in input.read() {
        if !event.state.is_pressed() {
            continue;
        }
        match &event.logical_key {
            Key::Backspace => {
                config.host_addr.pop();
            }
            Key::Character(typed) => {
                config.host_addr.extend(
                    typed
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '-')),
                );
            }
            _ => {}
        }
    }
}

/// The socket address `input` dials: as typed when it carries a port, with
/// [`TCP_PORT`] appended when it does not.
pub fn dial_addr(input: &str) -> String {
    if input.contains(':') {
        input.to_string()
    } else {
        format!("{input}:{TCP_PORT}")
    }
}

/// Handles every lobby button.
pub fn lobby_buttons(
    mut commands: Commands,
    mode: Res<LobbyMode>,
    link: Option<NonSendMut<LobbyLink>>,
    mut config: ResMut<LobbyConfig>,
    mut next: ResMut<NextState<GameState>>,
    mut buttons: Query<(&Interaction, &LobbyButton, &mut BackgroundColor), Changed<Interaction>>,
) {
    let mut link = link;
    for (interaction, button, mut color) in &mut buttons {
        if *interaction == Interaction::Hovered {
            *color = BackgroundColor(HOVERED);
            continue;
        }
        if *interaction == Interaction::None {
            *color = BackgroundColor(NORMAL);
            continue;
        }
        // Pressed.
        match button {
            LobbyButton::Kind(slot) => cycle_kind(&mode, link.as_deref_mut(), &mut config, *slot),
            LobbyButton::Race(slot) => toggle_race(&mode, link.as_deref_mut(), &mut config, *slot),
            LobbyButton::Claim(slot) => {
                if *mode == LobbyMode::Local {
                    claim_local(&mut config, *slot);
                }
            }
            LobbyButton::Topology => toggle_topology(link.as_deref_mut(), &mut config),
            LobbyButton::AiHosting => toggle_ai_hosting(link.as_deref_mut(), &mut config),
            LobbyButton::Back => next.set(GameState::Menu),
            LobbyButton::Start => {
                if can_start(&mode, link.as_deref()) {
                    commands.insert_resource(StartRequested);
                }
            }
        }
    }
}

fn cycle_kind(mode: &LobbyMode, link: Option<&mut LobbyLink>, config: &mut LobbyConfig, slot: u8) {
    match mode {
        // The local human's slot is fixed; the others toggle AI ↔ Closed.
        LobbyMode::Local => {
            if slot == config.local_slot {
                return;
            }
            let kind = &mut config.slots[slot as usize].kind;
            *kind = match kind {
                SlotKind::Ai => SlotKind::Closed,
                _ => SlotKind::Ai,
            };
        }
        // The host cycles an empty slot Open → AI → Closed; a slot a client
        // occupies is left alone.
        LobbyMode::Host => {
            if let Some(LobbyLink::Host(host)) = link {
                let occupant = match config.slots[slot as usize].kind {
                    SlotKind::Open => Occupant::Ai,
                    SlotKind::Ai => Occupant::Closed,
                    SlotKind::Closed => Occupant::Open,
                    SlotKind::Human => return,
                };
                let _ = host.set_occupant(slot, occupant);
            }
        }
        // Clients do not edit slots.
        LobbyMode::Client => {}
    }
}

fn toggle_race(mode: &LobbyMode, link: Option<&mut LobbyLink>, config: &mut LobbyConfig, slot: u8) {
    let race = config.slots[slot as usize].race.toggled();
    match (mode, link) {
        (LobbyMode::Host, Some(LobbyLink::Host(host))) => {
            let _ = host.set_race(slot, race.id());
        }
        (LobbyMode::Client, Some(LobbyLink::Client(client))) => {
            if client.local_player() == Some(slot) {
                let _ = client.request_race(race.id());
            }
        }
        (LobbyMode::Local, _) => config.slots[slot as usize].race = race,
        _ => {}
    }
}

fn claim_local(config: &mut LobbyConfig, slot: u8) {
    let previous = config.local_slot;
    // The slot you leave becomes an AI opponent (the default for a non-local slot).
    config.slots[previous as usize].kind = SlotKind::Ai;
    config.slots[slot as usize].kind = SlotKind::Human;
    config.local_slot = slot;
}

fn toggle_topology(link: Option<&mut LobbyLink>, config: &mut LobbyConfig) {
    // Only the host chooses the topology; a client just mirrors it.
    let Some(LobbyLink::Host(host)) = link else {
        return;
    };
    let next = match config.topology {
        Topology::HostStar => Topology::Mesh,
        Topology::Mesh => Topology::HostStar,
    };
    config.topology = next;
    let _ = host.set_topology(next);
}

fn toggle_ai_hosting(link: Option<&mut LobbyLink>, config: &mut LobbyConfig) {
    // Only the host chooses the AI hosting mode; a client just mirrors it.
    let Some(LobbyLink::Host(host)) = link else {
        return;
    };
    let next = match config.ai_hosting {
        AiHosting::HostOnly => AiHosting::Replicated,
        AiHosting::Replicated => AiHosting::HostOnly,
    };
    config.ai_hosting = next;
    let _ = host.set_ai_hosting(next);
}

/// Retries connecting at most this often (in frames) while a client is not yet
/// connected, so a not-yet-running host or an edited address is picked up without
/// hammering a blocking connect every frame.
const CONNECT_RETRY_FRAMES: u32 = 60;

/// Connects a client to the host automatically while it is in the lobby and not
/// yet linked: spawns a dial on a worker thread, polls its result, and redials
/// on a cooldown — or as soon as the typed address changes. An abandoned
/// attempt's thread dies on its own once the OS gives up on the dial.
pub fn auto_connect_client(
    mode: Res<LobbyMode>,
    link: Option<NonSend<LobbyLink>>,
    pending: Option<NonSend<PendingConnect>>,
    mut config: ResMut<LobbyConfig>,
    mut commands: Commands,
    mut cooldown: Local<u32>,
) {
    if *mode != LobbyMode::Client || link.is_some() {
        return;
    }

    if let Some(pending) = pending {
        if pending.addr != config.host_addr {
            // Typed past the attempt: abandon it and redial next frame.
            *cooldown = 0;
            commands.queue(|world: &mut World| {
                world.remove_non_send_resource::<PendingConnect>();
            });
            return;
        }
        match pending.result.try_recv() {
            Ok(Ok(mut client)) => {
                commands.queue(|world: &mut World| {
                    world.remove_non_send_resource::<PendingConnect>();
                });
                // Announce this build, the offered mesh port, and a race.
                if let Err(error) = client.join(Some(UDP_PORT), Some(Race::Human.id())) {
                    config.status = format!("join failed: {error}");
                    return;
                }
                config.status = "connected".to_string();
                commands.queue(move |world: &mut World| {
                    world.insert_non_send_resource(LobbyLink::Client(client));
                });
            }
            Ok(Err(error)) => {
                config.status = format!("connect failed: {error}");
                commands.queue(|world: &mut World| {
                    world.remove_non_send_resource::<PendingConnect>();
                });
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                commands.queue(|world: &mut World| {
                    world.remove_non_send_resource::<PendingConnect>();
                });
            }
        }
        return;
    }

    if *cooldown > 0 {
        *cooldown -= 1;
        return;
    }
    *cooldown = CONNECT_RETRY_FRAMES;

    let field = config.host_addr.clone();
    let addr = dial_addr(&field);
    config.status = format!("connecting to {addr}...");
    let (sender, result) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(bootstrap::join_lobby(addr.as_str()));
    });
    commands.queue(move |world: &mut World| {
        world.insert_non_send_resource(PendingConnect {
            addr: field,
            result,
        });
    });
}

/// Only the host (or a local game) may start, and a client must be connected.
fn can_start(mode: &LobbyMode, link: Option<&LobbyLink>) -> bool {
    match mode {
        LobbyMode::Local => true,
        LobbyMode::Host => matches!(link, Some(LobbyLink::Host(_))),
        LobbyMode::Client => false,
    }
}

//
// ─── UI ────────────────────────────────────────────────────────────────────────
//

/// Builds the lobby UI for the current mode.
pub fn setup_lobby(mut commands: Commands, mode: Res<LobbyMode>) {
    let is_client = *mode == LobbyMode::Client;
    let title = match *mode {
        LobbyMode::Local => "Local Game",
        LobbyMode::Host => "Create Network Game",
        LobbyMode::Client => "Connect To Network Game",
    };

    let root = commands
        .spawn((
            LobbyRoot,
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: px(10),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        ))
        .id();

    commands.entity(root).with_children(|parent| {
        parent.spawn((
            Text::new(title),
            TextFont {
                font_size: 34.0,
                ..default()
            },
            TextColor(Color::srgb(0.95, 0.95, 0.9)),
        ));
        parent.spawn((
            StatusText,
            Text::new(String::new()),
            TextFont {
                font_size: 16.0,
                ..default()
            },
            TextColor(Color::srgb(0.8, 0.8, 0.6)),
        ));

        if !matches!(*mode, LobbyMode::Local) {
            let is_host = matches!(*mode, LobbyMode::Host);
            parent.spawn(row_node()).with_children(|row| {
                label(row, "Topology:");
                // Only the host chooses it; a client just sees the choice.
                if is_host {
                    spawn_button(row, "Toggle", LobbyButton::Topology);
                }
                row.spawn((
                    TopologyText,
                    Text::new("Host-star"),
                    text_font(),
                    TextColor(Color::srgb(0.9, 0.9, 0.95)),
                ));
            });
            // In a local game the modes are equivalent, so the choice only
            // appears for network games.
            parent.spawn(row_node()).with_children(|row| {
                label(row, "AI runs on:");
                if is_host {
                    spawn_button(row, "Toggle", LobbyButton::AiHosting);
                }
                row.spawn((
                    AiHostingText,
                    Text::new("Host"),
                    text_font(),
                    TextColor(Color::srgb(0.9, 0.9, 0.95)),
                ));
            });
        }

        for slot in 0..SLOTS as u8 {
            parent.spawn(row_node()).with_children(|row| {
                row.spawn((
                    SlotText(slot),
                    Text::new(String::new()),
                    text_font(),
                    TextColor(Color::srgb(0.9, 0.9, 0.95)),
                ));
                // A client cannot change slot kinds (host-authoritative); it can
                // still pick its own race.
                if !is_client {
                    spawn_button(row, "Kind", LobbyButton::Kind(slot));
                }
                spawn_button(row, "Race", LobbyButton::Race(slot));
                if matches!(*mode, LobbyMode::Local) {
                    spawn_button(row, "Play here", LobbyButton::Claim(slot));
                }
            });
        }

        if is_client {
            parent.spawn(row_node()).with_children(|row| {
                label(row, "Host address (auto-connecting):");
                row.spawn((
                    AddrText,
                    Text::new(String::new()),
                    text_font(),
                    TextColor(Color::srgb(0.9, 0.95, 0.9)),
                ));
            });
        }

        parent.spawn(row_node()).with_children(|row| {
            spawn_button(row, "Back", LobbyButton::Back);
            // The host starts the game for everyone; a client auto-starts when the
            // host's start signal arrives, so it has no Start button.
            if !is_client {
                spawn_button(row, "Start", LobbyButton::Start);
            }
        });
    });
}

fn row_node() -> impl Bundle {
    Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: px(10),
        ..default()
    }
}

fn text_font() -> TextFont {
    TextFont {
        font_size: 18.0,
        ..default()
    }
}

fn label(parent: &mut ChildSpawnerCommands, text: &str) {
    parent.spawn((
        Text::new(text.to_string()),
        text_font(),
        TextColor(Color::srgb(0.7, 0.7, 0.75)),
    ));
}

fn spawn_button(parent: &mut ChildSpawnerCommands, text: &str, tag: LobbyButton) {
    parent
        .spawn((
            tag,
            Button,
            Node {
                padding: UiRect::axes(px(12), px(6)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(NORMAL),
        ))
        .with_children(|button| {
            button.spawn((Text::new(text.to_string()), text_font()));
        });
}

/// Refreshes the lobby text each frame so live changes (joins, edits) show.
pub fn update_lobby_view(
    config: Res<LobbyConfig>,
    mode: Res<LobbyMode>,
    mut slots: Query<
        (&SlotText, &mut Text),
        (
            Without<TopologyText>,
            Without<AiHostingText>,
            Without<StatusText>,
            Without<AddrText>,
        ),
    >,
    mut topology: Query<
        &mut Text,
        (
            With<TopologyText>,
            Without<AiHostingText>,
            Without<StatusText>,
            Without<AddrText>,
        ),
    >,
    mut ai_hosting: Query<&mut Text, (With<AiHostingText>, Without<StatusText>, Without<AddrText>)>,
    mut status: Query<&mut Text, (With<StatusText>, Without<AddrText>)>,
    mut addr: Query<&mut Text, With<AddrText>>,
    mut buttons: Query<(&LobbyButton, &mut Node)>,
) {
    // A client may only set its own slot's race, so hide every other Race button.
    for (button, mut node) in &mut buttons {
        if let LobbyButton::Race(slot) = button {
            let show = *mode != LobbyMode::Client || *slot == config.local_slot;
            node.display = if show { Display::Flex } else { Display::None };
        }
    }

    for (slot, mut text) in &mut slots {
        let view = config.slots[slot.0 as usize];
        // `local_slot` is this node's own slot in every mode (host = 0, client =
        // its assigned slot, local = the claimed slot).
        let you = if slot.0 == config.local_slot {
            " (you)"
        } else {
            ""
        };
        *text = Text::new(format!(
            "Slot {}: {} - {}{you}",
            slot.0,
            view.kind.label(),
            view.race.label(),
        ));
    }
    if let Ok(mut text) = topology.single_mut() {
        *text = Text::new(match config.topology {
            Topology::HostStar => "Host-star",
            Topology::Mesh => "Mesh",
        });
    }
    if let Ok(mut text) = ai_hosting.single_mut() {
        *text = Text::new(match config.ai_hosting {
            AiHosting::HostOnly => "Host",
            AiHosting::Replicated => "All peers",
        });
    }
    if let Ok(mut text) = status.single_mut() {
        *text = Text::new(config.status.clone());
    }
    if let Ok(mut text) = addr.single_mut() {
        *text = Text::new(config.host_addr.clone());
    }
}

//
// ─── Start ───────────────────────────────────────────────────────────────────
//

/// Builds the session (and network session) from the locked lobby and enters the
/// game. Exclusive so it can insert the `NonSend` network session and resize the
/// per-player resources.
pub fn start_game(world: &mut World) {
    if world.remove_resource::<StartRequested>().is_none() {
        return;
    }

    let mode = *world.resource::<LobbyMode>();
    let slots = player_slots(world.resource::<LobbyConfig>());
    // Mirrored from the host's authoritative state, so identical on every node.
    let ai_hosting = world.resource::<LobbyConfig>().ai_hosting;

    let (local_player, net) = match mode {
        LobbyMode::Local => (world.resource::<LobbyConfig>().local_slot, None),
        LobbyMode::Host => {
            let Some(LobbyLink::Host(host)) = world.remove_non_send_resource::<LobbyLink>() else {
                return;
            };
            match NetSession::start_host(host, udp_bind()) {
                Ok(net) => (0, Some(net)),
                Err(error) => {
                    eprintln!("failed to start host: {error}");
                    return;
                }
            }
        }
        LobbyMode::Client => {
            let Some(LobbyLink::Client(client)) = world.remove_non_send_resource::<LobbyLink>()
            else {
                return;
            };
            let local = client.local_player().unwrap_or(0);
            match NetSession::start_client(client, udp_bind()) {
                Ok(net) => (local, Some(net)),
                Err(error) => {
                    eprintln!("failed to start client: {error}");
                    return;
                }
            }
        }
    };

    let player_count = slots.len();
    {
        let mut session = world.resource_mut::<GameSession>();
        session.configure(local_player, slots, ai_hosting);
    }
    install_game_resources(world, player_count);
    if let Some(net) = net {
        install_network_session(world, net);
    }
    // After the network session, so AI ownership resolves against it.
    crate::ai::install_demo_ai(world);
    world
        .resource_mut::<NextState<GameState>>()
        .set(GameState::InGame);
}

fn player_slots(config: &LobbyConfig) -> Vec<PlayerSlot> {
    config
        .slots
        .iter()
        .enumerate()
        .map(|(i, view)| {
            let id = i as PlayerId;
            match view.kind {
                SlotKind::Human => {
                    PlayerSlot::occupied(id, PlayerType::Human, Some(view.race.id()))
                }
                SlotKind::Ai => PlayerSlot::occupied(id, PlayerType::Ai, Some(view.race.id())),
                SlotKind::Open | SlotKind::Closed => PlayerSlot::free(id),
            }
        })
        .collect()
}

fn udp_bind() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], UDP_PORT))
}
