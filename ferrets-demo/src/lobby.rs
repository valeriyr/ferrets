//! The lobby screen: configure slots, pick a topology, and start the game.
//!
//! For a local game the [`LobbyConfig`] is the source of truth, edited directly.
//! For a network game the host coordinates: the [`LobbyLink`] holds the
//! [`LobbyHost`]/[`LobbyClient`], the authoritative state is mirrored into the
//! config each frame for display, and edits go through the host.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use ferrets_bevy_plugin::{install_game_resources, install_network_session};
use ferrets_network::lobby::client::{LobbyClient, PollOutcome};
use ferrets_network::message::control::Occupant;
use ferrets_network::session::NetSession;
use ferrets_network::session_mode::SessionMode;
use ferrets_network::{bootstrap, lobby::host::LobbyHost};
use ferrets_simulation::session::{
    GameSession,
    ai_hosting::AiHosting,
    drop_policy::DropPolicy,
    finish_policy::FinishPolicy,
    player_slot::{PlayerId, PlayerSlot},
    player_type::PlayerType,
};

use crate::map::START_POINTS;
use crate::states::{GameState, LobbyMode};

/// Player-slot capacity, one per map start point.
const SLOTS: usize = START_POINTS.len();
/// The TCP port the host binds and clients dial.
const TCP_PORT: u16 = 4000;

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

/// An editable text field of the lobby.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LobbyField {
    /// The host address a client dials.
    Addr,
    /// The TCP port the host's lobby listens on.
    TcpPort,
    /// This node's gameplay (UDP) port for a mesh game.
    UdpPort,
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
    pub mode: SessionMode,
    /// Which slot the local human controls (local games).
    pub local_slot: PlayerId,
    /// The host address a client dials: `ip` or `ip:port` (the port defaults
    /// to [`TCP_PORT`] when omitted).
    pub host_addr: String,
    /// The TCP port the host's lobby listens on, as typed. Empty means the
    /// default ([`TCP_PORT`]); anything else must parse as a port and is used
    /// exactly (see [`parse_tcp_port`]). Editing it reopens the lobby.
    pub tcp_port: String,
    /// This node's gameplay (UDP) port for a mesh game, as typed. Empty means
    /// an ephemeral port; anything else must parse as a port and is used
    /// exactly (see [`parse_udp_port`]).
    pub udp_port: String,
    /// The field keyboard input currently edits.
    pub focused: LobbyField,
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
            mode: SessionMode::HostStar {
                ai_hosting: AiHosting::Host,
            },
            local_slot: 0,
            host_addr: "127.0.0.1".to_string(),
            tcp_port: String::new(),
            udp_port: String::new(),
            focused: match mode {
                LobbyMode::Host => LobbyField::TcpPort,
                _ => LobbyField::Addr,
            },
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

/// The TCP port the host's lobby listener is currently bound on.
#[derive(Resource)]
pub struct HostTcpPort(u16);

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
    Mode,
    AiHosting,
    Focus(LobbyField),
    Back,
    Start,
}

#[derive(Component)]
pub struct StatusText;

#[derive(Component)]
pub struct ModeText;

#[derive(Component)]
pub struct AiHostingText;

#[derive(Component)]
pub struct AddrText;

#[derive(Component)]
pub struct TcpPortText;

#[derive(Component)]
pub struct UdpPortText;

#[derive(Component)]
pub struct SlotText(u8);

//
// ─── Setup / teardown ──────────────────────────────────────────────────────────
//

/// Builds the lobby config and (for network modes) opens the connection.
pub fn enter_lobby(mut commands: Commands, mode: Res<LobbyMode>) {
    let mut config = LobbyConfig::for_mode(*mode);
    match *mode {
        LobbyMode::Host => match open_host(config.mode, TCP_PORT) {
            Ok(host) => {
                config.status =
                    format!("hosting on port {TCP_PORT} - clients dial this machine's ip");
                commands.insert_resource(HostTcpPort(TCP_PORT));
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
    commands.remove_resource::<HostTcpPort>();
    commands.remove_resource::<StartRequested>();
}

fn open_host(mode: SessionMode, tcp_port: u16) -> ferrets_network::Result<LobbyHost> {
    // The demo's game decisions beyond the lobby-editable mode: drops resolve
    // on the timeout (no wait dialog yet) and a match ends by elimination.
    bootstrap::open_lobby(
        ("0.0.0.0", tcp_port),
        mode,
        DropPolicy::Automatic,
        FinishPolicy::LastStanding,
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
            mirror(&mut config, host.slots(), host.mode());
        }
        LobbyLink::Client(client) => match client.poll() {
            PollOutcome::Waiting { changed } => {
                if changed
                    && let Some(state) = client.state()
                    && !state.slots.is_empty()
                {
                    mirror(&mut config, &state.slots, state.mode);
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
    mode: SessionMode,
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
    config.mode = mode;
}

//
// ─── Input ────────────────────────────────────────────────────────────────────
//

/// Captures typing into the focused lobby field: the host address (addresses,
/// host names, an optional `:port`) or this node's gameplay port (digits).
pub fn lobby_field_input(
    mode: Res<LobbyMode>,
    link: Option<NonSend<LobbyLink>>,
    mut input: MessageReader<KeyboardInput>,
    mut config: ResMut<LobbyConfig>,
) {
    match *mode {
        // A local game has no fields; a connected client's fields are spent.
        LobbyMode::Local => return,
        LobbyMode::Client if link.is_some() => return,
        _ => {}
    }
    for event in input.read() {
        if !event.state.is_pressed() {
            continue;
        }
        let field = match config.focused {
            LobbyField::Addr => &mut config.host_addr,
            LobbyField::TcpPort => &mut config.tcp_port,
            LobbyField::UdpPort => &mut config.udp_port,
        };
        match &event.logical_key {
            Key::Backspace => {
                field.pop();
            }
            Key::Character(typed) => match config.focused {
                LobbyField::Addr => {
                    config.host_addr.extend(
                        typed
                            .chars()
                            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '-')),
                    );
                }
                LobbyField::TcpPort => {
                    let room = 5usize.saturating_sub(config.tcp_port.len());
                    config
                        .tcp_port
                        .extend(typed.chars().filter(char::is_ascii_digit).take(room));
                }
                LobbyField::UdpPort => {
                    let room = 5usize.saturating_sub(config.udp_port.len());
                    config
                        .udp_port
                        .extend(typed.chars().filter(char::is_ascii_digit).take(room));
                }
            },
            _ => {}
        }
    }
}

/// Parses the TCP-port field: empty means the default ([`TCP_PORT`]);
/// anything else must be a valid non-zero port, used exactly as given.
pub fn parse_tcp_port(input: &str) -> Result<u16, String> {
    if input.is_empty() {
        return Ok(TCP_PORT);
    }
    match input.parse::<u16>() {
        Ok(0) | Err(_) => Err(format!("invalid tcp port '{input}'")),
        Ok(port) => Ok(port),
    }
}

/// Parses the UDP-port field: empty means "pick an ephemeral port" (`None`);
/// anything else must be a valid port number, used exactly as given.
pub fn parse_udp_port(input: &str) -> Result<Option<u16>, String> {
    if input.is_empty() {
        return Ok(None);
    }
    input
        .parse::<u16>()
        .map(Some)
        .map_err(|_| format!("invalid udp port '{input}'"))
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
            LobbyButton::Mode => toggle_mode(link.as_deref_mut(), &mut config),
            LobbyButton::AiHosting => toggle_ai_hosting(link.as_deref_mut(), &mut config),
            LobbyButton::Focus(field) => config.focused = *field,
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

fn toggle_mode(link: Option<&mut LobbyLink>, config: &mut LobbyConfig) {
    // Only the host chooses the session mode; a client just mirrors it.
    let Some(LobbyLink::Host(host)) = link else {
        return;
    };
    // Cycle the three modes, keeping the AI hosting choice where the next
    // mode has one.
    let next = match config.mode {
        SessionMode::HostStar { ai_hosting } => SessionMode::MeshHosted { ai_hosting },
        SessionMode::MeshHosted { .. } => SessionMode::MeshDecentralized,
        SessionMode::MeshDecentralized => SessionMode::HostStar {
            ai_hosting: AiHosting::Host,
        },
    };
    config.mode = next;
    let _ = host.set_mode(next);
}

fn toggle_ai_hosting(link: Option<&mut LobbyLink>, config: &mut LobbyConfig) {
    // Only the host chooses the AI hosting mode; a client just mirrors it. A
    // decentralized session has no host to compute AI, so there is nothing to
    // toggle.
    let Some(LobbyLink::Host(host)) = link else {
        return;
    };
    let next = match config.mode {
        SessionMode::HostStar { ai_hosting } => SessionMode::HostStar {
            ai_hosting: other_hosting(ai_hosting),
        },
        SessionMode::MeshHosted { ai_hosting } => SessionMode::MeshHosted {
            ai_hosting: other_hosting(ai_hosting),
        },
        SessionMode::MeshDecentralized => return,
    };
    config.mode = next;
    let _ = host.set_mode(next);
}

fn other_hosting(ai_hosting: AiHosting) -> AiHosting {
    match ai_hosting {
        AiHosting::Host => AiHosting::Replicated,
        AiHosting::Replicated => AiHosting::Host,
    }
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
                // Announce this build, the offered mesh port, and a race. An
                // invalid or occupied configured port fails here; the status
                // guides the player and the dial retries once it is fixed.
                let udp_port = match parse_udp_port(&config.udp_port) {
                    Ok(port) => port,
                    Err(message) => {
                        config.status = message;
                        return;
                    }
                };
                if let Err(error) = client.join(udp_port, Some(Race::Human.id())) {
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

/// Reopens the host's lobby listener once the edited TCP port settles on a
/// value it is not bound on. Reopening resets the lobby (connected clients
/// are dropped and reconnect by dialing the new port), so the change waits a
/// debounce, not each keystroke.
pub fn host_rebind(
    mode: Res<LobbyMode>,
    bound: Option<Res<HostTcpPort>>,
    mut config: ResMut<LobbyConfig>,
    mut commands: Commands,
    mut last: Local<Option<u16>>,
    mut cooldown: Local<u32>,
) {
    if *mode != LobbyMode::Host {
        return;
    }
    let desired = match parse_tcp_port(&config.tcp_port) {
        Ok(port) => port,
        Err(message) => {
            config.status = message;
            return;
        }
    };
    if bound.map(|bound| bound.0) == Some(desired) {
        *last = Some(desired);
        return;
    }
    if *last != Some(desired) {
        // The value just changed: restart the debounce.
        *last = Some(desired);
        *cooldown = CONNECT_RETRY_FRAMES;
        return;
    }
    if *cooldown > 0 {
        *cooldown -= 1;
        return;
    }
    *cooldown = CONNECT_RETRY_FRAMES;

    let mode = config.mode;
    commands.queue(move |world: &mut World| {
        world.remove_non_send_resource::<LobbyLink>();
        world.remove_resource::<HostTcpPort>();
        match open_host(mode, desired) {
            Ok(host) => {
                world.insert_non_send_resource(LobbyLink::Host(host));
                world.insert_resource(HostTcpPort(desired));
                world.resource_mut::<LobbyConfig>().status =
                    format!("hosting on port {desired} - clients dial this machine's ip");
            }
            Err(error) => {
                world.resource_mut::<LobbyConfig>().status = format!("host failed: {error}");
            }
        }
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
                label(row, "Mode:");
                // Only the host chooses it; a client just sees the choice.
                if is_host {
                    spawn_button(row, "Toggle", LobbyButton::Mode);
                }
                row.spawn((
                    ModeText,
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
                spawn_button(row, "Edit", LobbyButton::Focus(LobbyField::Addr));
                row.spawn((
                    AddrText,
                    Text::new(String::new()),
                    text_font(),
                    TextColor(Color::srgb(0.9, 0.95, 0.9)),
                ));
            });
        }
        if !is_client && !matches!(*mode, LobbyMode::Local) {
            parent.spawn(row_node()).with_children(|row| {
                label(row, "TCP port (lobby):");
                spawn_button(row, "Edit", LobbyButton::Focus(LobbyField::TcpPort));
                row.spawn((
                    TcpPortText,
                    Text::new(String::new()),
                    text_font(),
                    TextColor(Color::srgb(0.9, 0.95, 0.9)),
                ));
            });
        }
        if !matches!(*mode, LobbyMode::Local) {
            parent.spawn(row_node()).with_children(|row| {
                label(row, "UDP port (mesh):");
                spawn_button(row, "Edit", LobbyButton::Focus(LobbyField::UdpPort));
                row.spawn((
                    UdpPortText,
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
            Without<ModeText>,
            Without<AiHostingText>,
            Without<StatusText>,
            Without<AddrText>,
            Without<TcpPortText>,
            Without<UdpPortText>,
        ),
    >,
    mut mode_text: Query<
        &mut Text,
        (
            With<ModeText>,
            Without<AiHostingText>,
            Without<StatusText>,
            Without<AddrText>,
            Without<TcpPortText>,
            Without<UdpPortText>,
        ),
    >,
    mut ai_hosting: Query<
        &mut Text,
        (
            With<AiHostingText>,
            Without<StatusText>,
            Without<AddrText>,
            Without<TcpPortText>,
            Without<UdpPortText>,
        ),
    >,
    mut status: Query<
        &mut Text,
        (
            With<StatusText>,
            Without<AddrText>,
            Without<TcpPortText>,
            Without<UdpPortText>,
        ),
    >,
    mut addr: Query<&mut Text, (With<AddrText>, Without<TcpPortText>, Without<UdpPortText>)>,
    mut tcp_port: Query<&mut Text, (With<TcpPortText>, Without<UdpPortText>)>,
    mut udp_port: Query<&mut Text, With<UdpPortText>>,
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
    if let Ok(mut text) = mode_text.single_mut() {
        *text = Text::new(match config.mode {
            SessionMode::HostStar { .. } => "Host-star",
            SessionMode::MeshHosted { .. } => "Mesh (hosted)",
            SessionMode::MeshDecentralized => "Mesh (decentralized)",
        });
    }
    if let Ok(mut text) = ai_hosting.single_mut() {
        *text = Text::new(match config.mode.ai_hosting() {
            AiHosting::Host => "Host",
            AiHosting::Replicated => "All peers",
        });
    }
    if let Ok(mut text) = status.single_mut() {
        *text = Text::new(config.status.clone());
    }
    if let Ok(mut text) = addr.single_mut() {
        *text = Text::new(field_view(
            &config.host_addr,
            config.focused == LobbyField::Addr,
        ));
    }
    if let Ok(mut text) = tcp_port.single_mut() {
        let default = TCP_PORT.to_string();
        let value = if config.tcp_port.is_empty() {
            &default
        } else {
            &config.tcp_port
        };
        *text = Text::new(field_view(value, config.focused == LobbyField::TcpPort));
    }
    if let Ok(mut text) = udp_port.single_mut() {
        let value = if config.udp_port.is_empty() {
            "auto"
        } else {
            &config.udp_port
        };
        *text = Text::new(field_view(value, config.focused == LobbyField::UdpPort));
    }
}

/// A field's display text, with a cursor on the focused one.
fn field_view(value: &str, focused: bool) -> String {
    if focused {
        format!("{value}_")
    } else {
        value.to_string()
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

    let (local_player, choices, net) = match mode {
        LobbyMode::Local => {
            let config = world.resource::<LobbyConfig>();
            let choices = (
                config.mode.authority(),
                DropPolicy::Automatic,
                FinishPolicy::LastStanding,
            );
            (config.local_slot, choices, None)
        }
        LobbyMode::Host => {
            // Validate before consuming the lobby link, so a bad port leaves
            // the lobby alive to fix it.
            let udp_port = match parse_udp_port(&world.resource::<LobbyConfig>().udp_port) {
                Ok(port) => port,
                Err(message) => {
                    world.resource_mut::<LobbyConfig>().status = message;
                    return;
                }
            };
            let Some(LobbyLink::Host(host)) = world.remove_non_send_resource::<LobbyLink>() else {
                return;
            };
            // The choices the lobby broadcast, captured before the link is
            // consumed — identical on every node.
            let state = host.state();
            let choices = (
                state.mode.authority(),
                state.drop_policy,
                state.finish_policy,
            );
            match NetSession::start_host(host, udp_port) {
                Ok(net) => (0, choices, Some(net)),
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
            let Some(choices) = client.state().map(|state| {
                (
                    state.mode.authority(),
                    state.drop_policy,
                    state.finish_policy,
                )
            }) else {
                return;
            };
            match NetSession::start_client(client) {
                Ok(net) => (local, choices, Some(net)),
                Err(error) => {
                    eprintln!("failed to start client: {error}");
                    return;
                }
            }
        }
    };

    {
        let mut session = world.resource_mut::<GameSession>();
        let (authority, drop_policy, finish_policy) = choices;
        session.configure(local_player, slots, authority, drop_policy, finish_policy);
    }
    install_game_resources(world);
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
