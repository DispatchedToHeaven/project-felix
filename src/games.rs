//! Board games at an NPC's table, on the server: chess and Russian draughts, against the NPC or
//! another player.
//!
//! A game belongs to an NPC: it waits where it is, a table appears in front of it (synced to
//! Horizon clients as the `projectfelix:game` attachment, so everyone sees the position), and the
//! players play on a board screen. The server checks every move; the NPC thinks on a thread of its
//! own and moves at a level tick, at least a second later. The game ends by the rules or a
//! resignation; the table stays a few seconds for everyone to see the end.
//!
//! Client to server: `game_move` (NPC id, from, to, promotion letter or 0), `game_resign` (NPC id).
//! Server to client: `game_view` (JSON, the position as the player sees it).
use crate::checkers;
use crate::chess::{self, Color, Kind, Outcome};
use crate::companion::{self, Mode, ORDERS};
use crate::sync::{ACTION, Action as Play};
use mod_api::{
    AttachmentCodec, AttachmentError, AttachmentType, Compound, EntityRef, Holder, IPayloadContext, IPayloadHandler,
    PayloadBytes, PayloadId, PlayerRef, Registrar, ServerHost, Tag,
};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

pub const MOVE: &str = "projectfelix:game_move";
pub const RESIGN: &str = "projectfelix:game_resign";
pub const VIEW: &str = "projectfelix:game_view";

/// Ticks the table stays after a game ends; ticks a game waits for a second player.
const LINGER: i64 = 100;
const WAIT_FOR_PLAYER: i64 = 20 * 120;
/// Ticks the NPC takes to move, at least.
const THINK: i64 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameKind {
    Chess,
    Checkers,
}

impl GameKind {
    pub fn name(self) -> &'static str {
        match self {
            GameKind::Chess => "chess",
            GameKind::Checkers => "checkers",
        }
    }
    pub fn parse(s: &str) -> Option<GameKind> {
        match s {
            "chess" => Some(GameKind::Chess),
            "checkers" | "draughts" => Some(GameKind::Checkers),
            _ => None,
        }
    }
}

/// What everyone sees of a game: the table in front of the NPC.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub kind: String,
    /// 64 letters, a1 first: chess FEN letters, or `w b W B` for draughts; `.` empty.
    pub board: String,
    /// The way the NPC faces (degrees): towards the player with white.
    pub yaw: f32,
}

pub static TABLE: AttachmentType<Option<Table>> = AttachmentType {
    namespace: "projectfelix",
    name: "game",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |t| {
            let mut c = Compound::new();
            if let Some(t) = t {
                c.insert("kind".into(), Tag::String(t.kind.as_str().into()));
                c.insert("board".into(), Tag::String(t.board.as_str().into()));
                c.insert("yaw".into(), Tag::Float(t.yaw.to_bits()));
            }
            Ok(c)
        },
        decode: |c: &Compound| {
            let s = |k: &str| match c.get(&k.into()) {
                Some(Tag::String(v)) => Some(v.to_string_lossy()),
                _ => None,
            };
            let (Some(kind), Some(board)) = (s("kind"), s("board")) else { return Ok(None) };
            let yaw = match c.get(&"yaw".into()) {
                Some(Tag::Float(b)) => f32::from_bits(*b),
                Some(_) => return Err(AttachmentError::Decode),
                None => 0.0,
            };
            Ok(Some(Table { kind, board, yaw }))
        },
    }),
    copy_on_death: false,
    sync: true,
};

#[derive(Clone)]
enum Board {
    Chess(chess::Position),
    Checkers(checkers::Position),
}

impl Board {
    fn side(&self) -> Color {
        match self {
            Board::Chess(p) => p.side,
            Board::Checkers(p) => p.side,
        }
    }
    fn letters(&self) -> String {
        match self {
            Board::Chess(p) => p.board.iter().map(|x| x.map(|p| p.letter()).unwrap_or('.')).collect(),
            Board::Checkers(p) => p.board_text(),
        }
    }
    fn key(&self) -> String {
        match self {
            Board::Chess(p) => p.key(),
            Board::Checkers(p) => p.key(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Seat {
    Npc,
    Player(PlayerRef),
    Empty,
}

struct Game {
    npc: u32,
    kind: GameKind,
    board: Board,
    history: Vec<String>,
    white: Seat,
    black: Seat,
    last: Option<(u8, u8)>,
    result: Option<String>,
    /// Game time: when it began, the last move, when it ended.
    started: i64,
    moved: i64,
    ended: Option<i64>,
    restore: Mode,
    yaw: f32,
    thinking: Option<Arc<Mutex<Option<NpcMove>>>>,
}

#[derive(Clone)]
enum NpcMove {
    Chess(chess::Move),
    Checkers(checkers::Move),
    None,
}

static GAMES: LazyLock<Mutex<HashMap<u32, Game>>> = LazyLock::new(Default::default);

impl Game {
    fn seat(&self, color: Color) -> Seat {
        if color == Color::White { self.white } else { self.black }
    }
    fn color_of(&self, player: PlayerRef) -> Option<Color> {
        let uuid = player.uuid();
        let is = |s: Seat| matches!(s, Seat::Player(p) if p.uuid() == uuid);
        if is(self.white) {
            Some(Color::White)
        } else if is(self.black) {
            Some(Color::Black)
        } else {
            None
        }
    }
    fn players(&self) -> Vec<PlayerRef> {
        [self.white, self.black].into_iter().filter_map(|s| if let Seat::Player(p) = s { Some(p) } else { None }).collect()
    }
}

/// What a dialog asks about the game at an NPC's table, for `player`.
pub fn state_for(npc: u32, player: PlayerRef) -> (&'static str, bool) {
    let Ok(games) = GAMES.lock() else { return ("none", false) };
    match games.get(&npc) {
        None => ("none", false),
        Some(g) => {
            let state = if g.ended.is_some() {
                "over"
            } else if g.black == Seat::Empty {
                "waiting"
            } else {
                "playing"
            };
            (state, g.color_of(player).is_some())
        }
    }
}

struct OnServer(fn(&mut dyn ServerHost, PlayerRef, &[u8]));

impl IPayloadHandler for OnServer {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        let Ok(player) = context.player() else { return Ok(()) };
        let Ok(host) = context.server_host() else { return Ok(()) };
        (self.0)(host, player, &payload.0);
        Ok(())
    }
}

pub fn register(r: &mut dyn Registrar, view: Arc<dyn IPayloadHandler>) {
    let mut handlers = r.payload_handlers();
    let mut reg = handlers.registrar("1");
    reg.play_to_server(PayloadId(MOVE.into()), Arc::new(OnServer(on_move)));
    reg.play_to_server(PayloadId(RESIGN.into()), Arc::new(OnServer(on_resign)));
    reg.play_to_client(PayloadId(VIEW.into()), Some(view));
}

fn now_of(host: &dyn mod_api::Host, npc: EntityRef) -> i64 {
    npc.level(host).and_then(|l| l.game_time(host)).unwrap_or(0)
}

/// Sets up a game at `npc`'s table: `player` takes white; black is the NPC, or the next player
/// who joins.
pub fn start(host: &mut dyn ServerHost, npc: EntityRef, kind: GameKind, player: PlayerRef, against_npc: bool) -> Result<(), String> {
    if GAMES.lock().map_err(|_| "busy")?.contains_key(&npc.id()) {
        return Err("a game is already set up here".into());
    }
    let now = now_of(&*host, npc);
    let at = npc.position(&*host).ok_or("no NPC")?;
    let yaw = player.entity().position(&*host).map(|p| crate::scene::yaw_towards(at, p)).unwrap_or(0.0);
    let restore = ORDERS.get(host, Holder::Entity(npc.id())).map(|o| o.mode).unwrap_or_default();
    companion::set_orders(host, npc, Mode::Wait);
    let board = match kind {
        GameKind::Chess => Board::Chess(chess::Position::start()),
        GameKind::Checkers => Board::Checkers(checkers::Position::start()),
    };
    let game = Game {
        npc: npc.id(),
        kind,
        history: Vec::new(),
        board,
        white: Seat::Player(player),
        black: if against_npc { Seat::Npc } else { Seat::Empty },
        last: None,
        result: None,
        started: now,
        moved: now,
        ended: None,
        restore,
        yaw,
        thinking: None,
    };
    let table = Table { kind: kind.name().into(), board: game.board.letters(), yaw };
    let _ = TABLE.set(host, Holder::Entity(npc.id()), Some(table));
    if let Ok(mut g) = GAMES.lock() {
        g.insert(npc.id(), game);
    }
    send_views(host, npc.id(), &[]);
    Ok(())
}

/// The second player sits down at black.
pub fn join(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef) -> Result<(), String> {
    {
        let mut games = GAMES.lock().map_err(|_| "busy")?;
        let g = games.get_mut(&npc.id()).ok_or("no game here")?;
        if g.black != Seat::Empty {
            return Err("both seats are taken".into());
        }
        if g.color_of(player).is_some() {
            return Err("you already play here".into());
        }
        g.black = Seat::Player(player);
        g.moved = now_of(&*host, npc);
    }
    send_views(host, npc.id(), &[]);
    Ok(())
}

/// Opens the board for `player`: to play if they sit at it, to watch if not.
pub fn open(host: &mut dyn ServerHost, npc: EntityRef, player: PlayerRef) {
    send_views(host, npc.id(), &[player]);
}

fn on_move(host: &mut dyn ServerHost, player: PlayerRef, b: &[u8]) {
    if b.len() < 7 {
        return;
    }
    let id = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let (from, to, promo) = (b[4], b[5], b[6]);
    let Some(npc) = host.entity(id) else { return };
    let now = now_of(&*host, npc);
    let ok = {
        let Ok(mut games) = GAMES.lock() else { return };
        let Some(g) = games.get_mut(&id) else { return };
        if g.ended.is_some() || g.black == Seat::Empty || g.color_of(player) != Some(g.board.side()) {
            false
        } else {
            apply_player_move(g, from, to, promo, now)
        }
    };
    if ok {
        after_move(host, npc, now);
    } else {
        send_views(host, id, &[player]);
    }
}

fn apply_player_move(g: &mut Game, from: u8, to: u8, promo: u8, now: i64) -> bool {
    let key = g.board.key();
    let next = match &g.board {
        Board::Chess(p) => {
            let promotion = chess::Piece::from_letter(promo as char).map(|p| p.kind);
            let Some(m) = p.legal_moves().into_iter().find(|m| {
                m.from == from && m.to == to && (m.promotion.is_none() || m.promotion == promotion.or(Some(Kind::Queen)))
            }) else {
                return false;
            };
            Board::Chess(p.play(m))
        }
        Board::Checkers(p) => {
            let Some(m) = p.find(from, to) else { return false };
            Board::Checkers(p.play(&m))
        }
    };
    g.history.push(key);
    g.board = next;
    g.last = Some((from, to));
    g.moved = now;
    true
}

fn outcome_text(g: &Game) -> Option<String> {
    let name = |c: Color| if c == Color::White { "White" } else { "Black" };
    match &g.board {
        Board::Chess(p) => p.outcome(&g.history).map(|o| match o {
            Outcome::Checkmate(c) => format!("Checkmate. {} wins.", name(c)),
            Outcome::Stalemate => "Stalemate: a draw.".into(),
            Outcome::FiftyMoves => "Fifty moves without a capture or a pawn move: a draw.".into(),
            Outcome::Repetition => "The same position three times: a draw.".into(),
            Outcome::InsufficientMaterial => "Nobody can mate any more: a draw.".into(),
        }),
        Board::Checkers(p) => p.outcome(&g.history).map(|o| match o {
            checkers::Outcome::Win(c) => format!("{} has no moves left. {} wins.", name(c.other()), name(c)),
            checkers::Outcome::Draw => "A draw.".into(),
        }),
    }
}

/// After any move: the table, the end of the game, the NPC's turn, everyone's views.
fn after_move(host: &mut dyn ServerHost, npc: EntityRef, now: i64) {
    let mut reaction = None;
    let (letters, finished) = {
        let Ok(mut games) = GAMES.lock() else { return };
        let Some(g) = games.get_mut(&npc.id()) else { return };
        if let Some(text) = outcome_text(g) {
            g.result = Some(text.clone());
            g.ended = Some(now);
            // The NPC cheers a win of its own and nods at a loss.
            let npc_side = if g.white == Seat::Npc { Some(Color::White) } else if g.black == Seat::Npc { Some(Color::Black) } else { None };
            if let Some(side) = npc_side {
                let won = text.contains(if side == Color::White { "White wins" } else { "Black wins" });
                reaction = Some(if won { "animation.felix.cheer" } else { "animation.felix.wave" });
            }
        } else if g.seat(g.board.side()) == Seat::Npc {
            think(g);
        }
        (g.board.letters(), g.ended.is_some())
    };
    if let Some(Some(mut t)) = TABLE.get(host, Holder::Entity(npc.id())).ok() {
        t.board = letters;
        let _ = TABLE.set(host, Holder::Entity(npc.id()), Some(t));
    }
    if let Some(anim) = reaction {
        let _ = ACTION.set(host, Holder::Entity(npc.id()), Some(Play { animation: anim.into(), start: now }));
    }
    let _ = finished;
    send_views(host, npc.id(), &[]);
}

/// Starts the NPC thinking about its move, on a thread of its own.
fn think(g: &mut Game) {
    let slot = Arc::new(Mutex::new(None));
    g.thinking = Some(slot.clone());
    let board = g.board.clone();
    let seed = g.history.len() as u64 * 7919 + g.started as u64;
    std::thread::spawn(move || {
        let m = match board {
            Board::Chess(p) => chess::best_move(&p, 3, seed, 15).map(NpcMove::Chess),
            Board::Checkers(p) => checkers::best_move(&p, 6, seed, 10).map(NpcMove::Checkers),
        };
        if let Ok(mut s) = slot.lock() {
            *s = Some(m.unwrap_or(NpcMove::None));
        }
    });
}

fn on_resign(host: &mut dyn ServerHost, player: PlayerRef, b: &[u8]) {
    let Some(id) = b.get(0..4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]])) else { return };
    let Some(npc) = host.entity(id) else { return };
    let now = now_of(&*host, npc);
    {
        let Ok(mut games) = GAMES.lock() else { return };
        let Some(g) = games.get_mut(&id) else { return };
        let Some(color) = g.color_of(player) else { return };
        if g.ended.is_some() {
            return;
        }
        let winner = if color == Color::White { "Black" } else { "White" };
        g.result = Some(if g.black == Seat::Empty { "The game was called off.".into() } else { format!("{} resigned. {winner} wins.", if color == Color::White { "White" } else { "Black" }) });
        g.ended = Some(now);
        g.thinking = None;
    }
    send_views(host, id, &[]);
}

/// The level tick: the NPC's moves once it has thought, games nobody joined, tables put away.
pub fn level_tick(host: &mut dyn ServerHost) {
    let ids: Vec<u32> = GAMES.lock().map(|g| g.keys().copied().collect()).unwrap_or_default();
    for id in ids {
        let Some(npc) = host.entity(id) else { continue };
        let now = now_of(&*host, npc);
        enum Step {
            Nothing,
            Moved,
            Finish(Mode),
            Called,
        }
        let step = {
            let Ok(mut games) = GAMES.lock() else { return };
            let Some(g) = games.get_mut(&id) else { continue };
            if let Some(end) = g.ended {
                if now - end >= LINGER {
                    let restore = g.restore;
                    games.remove(&id);
                    Step::Finish(restore)
                } else {
                    Step::Nothing
                }
            } else if g.black == Seat::Empty && now - g.started > WAIT_FOR_PLAYER {
                g.result = Some("Nobody came to play.".into());
                g.ended = Some(now);
                Step::Called
            } else if let Some(slot) = g.thinking.clone()
                && now - g.moved >= THINK
                && let Some(m) = slot.lock().ok().and_then(|mut s| s.take())
            {
                g.thinking = None;
                let key = g.board.key();
                match (m, &g.board) {
                    (NpcMove::Chess(m), Board::Chess(p)) => {
                        g.board = Board::Chess(p.play(m));
                        g.last = Some((m.from, m.to));
                    }
                    (NpcMove::Checkers(m), Board::Checkers(p)) => {
                        g.last = Some((m.from(), m.to()));
                        g.board = Board::Checkers(p.play(&m));
                    }
                    _ => {}
                }
                g.history.push(key);
                g.moved = now;
                Step::Moved
            } else {
                Step::Nothing
            }
        };
        match step {
            Step::Nothing => {}
            Step::Called => send_views(host, id, &[]),
            Step::Moved => {
                // A reach for the piece.
                let _ = ACTION.set(host, Holder::Entity(id), Some(Play { animation: "animation.felix.move_piece".into(), start: now }));
                after_move(host, npc, now);
            }
            Step::Finish(restore) => {
                let _ = TABLE.set(host, Holder::Entity(id), None);
                companion::set_orders(host, npc, restore);
            }
        }
    }
}

fn color_name(c: Color) -> &'static str {
    if c == Color::White { "white" } else { "black" }
}

/// Sends each player at the game (or `only` these, players or watchers) what they see.
fn send_views(host: &mut dyn ServerHost, id: u32, only: &[PlayerRef]) {
    let views: Vec<(PlayerRef, Vec<u8>)> = {
        let Ok(games) = GAMES.lock() else { return };
        let Some(g) = games.get(&id) else { return };
        let targets: Vec<PlayerRef> = if only.is_empty() { g.players() } else { only.to_vec() };
        targets
            .into_iter()
            .map(|p| {
                let you = g.color_of(p);
                let your_turn = g.ended.is_none() && g.black != Seat::Empty && you == Some(g.board.side());
                let moves: Vec<Vec<u8>> = if !your_turn {
                    Vec::new()
                } else {
                    match &g.board {
                        Board::Chess(pos) => pos
                            .legal_moves()
                            .into_iter()
                            .filter(|m| m.promotion.is_none() || m.promotion == Some(Kind::Queen))
                            .map(|m| vec![m.from, m.to, u8::from(m.promotion.is_some())])
                            .collect(),
                        Board::Checkers(pos) => pos.legal_moves().into_iter().map(|m| vec![m.from(), m.to(), 0]).collect(),
                    }
                };
                let seat_name = |s: Seat| match s {
                    Seat::Npc => "NPC".to_string(),
                    Seat::Player(p) => p.name(&*host).unwrap_or_else(|| "?".into()),
                    Seat::Empty => "(empty seat)".into(),
                };
                let status = if let Some(r) = &g.result {
                    r.clone()
                } else if g.black == Seat::Empty {
                    "Waiting for a second player: they talk to the NPC and take the other seat.".into()
                } else if your_turn {
                    "Your move.".into()
                } else if g.seat(g.board.side()) == Seat::Npc {
                    "The NPC is thinking...".into()
                } else {
                    format!("{} to move.", if g.board.side() == Color::White { "White" } else { "Black" })
                };
                let check = matches!(&g.board, Board::Chess(pos) if pos.in_check(pos.side));
                let view = crate::serde_json::json!({
                    "npc": id,
                    "kind": g.kind.name(),
                    "board": g.board.letters(),
                    "you": you.map(color_name).unwrap_or("watch"),
                    "turn": color_name(g.board.side()),
                    "moves": moves,
                    "last": g.last.map(|(a, b)| vec![a, b]),
                    "status": status,
                    "check": check,
                    "over": g.ended.is_some(),
                    "white": seat_name(g.white),
                    "black": seat_name(g.black),
                });
                (p, crate::serde_json::to_vec(&view).unwrap_or_default())
            })
            .collect()
    };
    for (p, bytes) in views {
        let _ = p.send_payload(host, &PayloadId(VIEW.into()), PayloadBytes(bytes));
    }
}
