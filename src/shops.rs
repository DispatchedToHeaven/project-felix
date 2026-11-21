//! Shops at work on the server (see `shop`): an NPC's stall in its dialog, prices by how much it
//! likes the player, stock that runs out and comes back, and hours when it lives by a day.
//!
//! An NPC sells what `/felix shop` gives it, what its dialog's `shop` action names, or what its
//! file's `shop.json` says when it was made in the game.
use crate::dialog::{Action, Choice, Dialog, Node};
use crate::shop::{self, Offer, Shop, Standing};
use mod_api::{
    AddServerReloadListenersEvent, AttachmentCodec, AttachmentError, AttachmentType, Command, Compound, EntityRef,
    FileToIdConverter, Flow, Holder, Identifier, PlayerRef, RegisterCommandsEvent, ReloadApply, ReloadResourceError,
    RunModCommandEvent, ServerHost, ServerReloadContext, ServerReloadListener, Tag, Text,
};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

pub static SHOP: AttachmentType<Option<String>> = AttachmentType {
    namespace: "projectfelix",
    name: "shop",
    default: || None,
    codec: Some(AttachmentCodec {
        encode: |s| {
            let mut c = Compound::new();
            if let Some(s) = s {
                c.insert("id".into(), Tag::String(s.as_str().into()));
            }
            Ok(c)
        },
        decode: |c: &Compound| match c.get(&"id".into()) {
            Some(Tag::String(s)) => Ok(Some(s.to_string_lossy())),
            None => Ok(None),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

static LOADED: LazyLock<Mutex<HashMap<String, Arc<Shop>>>> = LazyLock::new(Default::default);
/// Shops of NPCs made in the game, by their dialog id.
static EXTRA: LazyLock<Mutex<HashMap<String, Arc<Shop>>>> = LazyLock::new(Default::default);

pub fn set_extra(dialog: &str, s: Option<Shop>) {
    if let Ok(mut e) = EXTRA.lock() {
        match s {
            Some(s) => e.insert(dialog.to_string(), Arc::new(s)),
            None => e.remove(dialog),
        };
    }
}

fn shop_named(id: &str) -> Option<Arc<Shop>> {
    EXTRA.lock().ok().and_then(|e| e.get(id).cloned()).or_else(|| LOADED.lock().ok().and_then(|l| l.get(id).cloned()))
}

struct Listener;

impl ServerReloadListener for Listener {
    fn prepare(&self, cx: &ServerReloadContext) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut next = HashMap::new();
        for id in cx.list() {
            let Some(bytes) = cx.read(id) else { continue };
            match shop::parse(bytes) {
                Ok(s) => {
                    next.insert(id.to_string(), Arc::new(s));
                }
                Err(e) => eprintln!("[projectfelix] shop {id} not loaded: {e}"),
            }
        }
        Ok(<dyn ReloadApply>::new(move || {
            if let Ok(mut live) = LOADED.lock() {
                *live = next;
            }
        }))
    }
}

pub fn add_listener(e: &mut AddServerReloadListenersEvent<'_>) {
    let (Ok(id), Ok(converter)) = (Identifier::parse("projectfelix:shops"), FileToIdConverter::json("felix_shops")) else {
        return;
    };
    if let Err(err) = e.listeners.add_listener(id, converter, Arc::new(Listener)) {
        eprintln!("[projectfelix] shops listener not added: {err:?}");
    }
}

// ---- stock and reopening ---------------------------------------------------------------------------------

#[derive(Default)]
struct State {
    /// Left of an offer: (NPC by UUID, shop, buys, index) → (left, since).
    stock: HashMap<(u128, String, bool, usize), (u32, i64)>,
    /// Stalls to show again once the trade's commands ran: (player, NPC, shop, when).
    reopen: Vec<(PlayerRef, u32, String, i64)>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

/// The server stopped: nothing of its world is kept.
pub fn clear() {
    if let Ok(mut s) = STATE.lock() {
        *s = Default::default();
    }
    if let Ok(mut s) = EXTRA.lock() {
        *s = Default::default();
    }
    if let Ok(mut s) = LAST.lock() {
        *s = Default::default();
    }
}

fn now_of(host: &dyn mod_api::Host, e: EntityRef) -> i64 {
    e.level(host).and_then(|l| l.game_time(host)).unwrap_or(0)
}

/// How many of offer `o` are left (None: no end).
/// The key an NPC's stock is kept by.
fn stock_key(host: &mut dyn ServerHost, npc: EntityRef) -> u128 {
    npc.uuid(host).unwrap_or(u128::from(npc.id()))
}

fn left(npc: u128, id: &str, s: &Shop, buys: bool, i: usize, o: &Offer, now: i64) -> Option<u32> {
    let full = o.stock?;
    let mut st = STATE.lock().ok()?;
    // What was restocked long ago is as good as new: not kept.
    if st.stock.len() > 2048 {
        st.stock.retain(|_, e| now >= e.1 && now - e.1 < 24000 * 8);
    }
    let e = st.stock.entry((npc, id.to_string(), buys, i)).or_insert((full, now));
    if now - e.1 >= (s.restock * 1200.0) as i64 || now < e.1 {
        *e = (full, now);
    }
    Some(e.0)
}

fn take_one(npc: u128, id: &str, buys: bool, i: usize) {
    if let Ok(mut st) = STATE.lock()
        && let Some(e) = st.stock.get_mut(&(npc, id.to_string(), buys, i))
    {
        e.0 = e.0.saturating_sub(1);
    }
}

/// The shop NPC `npc` keeps: the one named, else its own by command, else its file's.
fn shop_of(host: &mut dyn ServerHost, npc: EntityRef, named: &str) -> Option<(String, Arc<Shop>)> {
    let mut ids = Vec::new();
    if !named.is_empty() {
        ids.push(named.to_string());
    }
    if let Some(s) = SHOP.get(host, Holder::Entity(npc.id())).ok().flatten() {
        ids.push(s);
    }
    if let Some(d) = crate::talk::DIALOG.get(host, Holder::Entity(npc.id())).ok().flatten() {
        ids.push(d);
    }
    ids.into_iter().find_map(|id| shop_named(&id).map(|s| (id, s)))
}

fn items_of(host: &mut dyn ServerHost, p: PlayerRef, item: &str) -> u32 {
    p.inventory(&*host).unwrap_or_default().iter().filter(|s| s.item().id(&*host) == item).map(|s| s.count()).sum()
}

fn choice(text: String, actions: Vec<Action>) -> Choice {
    Choice { text, condition: None, actions, goto: None }
}

// ---- the stall ----------------------------------------------------------------------------------------------

/// The `shop` dialog action: the stall, with this player's prices.
pub fn board(host: &mut dyn ServerHost, player: PlayerRef, npc: EntityRef, named: &str) {
    let now = now_of(&*host, npc);
    let mut text = Vec::new();
    let mut options = Vec::new();
    match shop_of(host, npc, named) {
        None => text.push("I have nothing to sell, @player.".to_string()),
        Some((id, s)) => {
            let doing = crate::routines::doing(npc.id());
            let (friend_at, hate_at) = crate::heart::thresholds(npc.id());
            let standing = Standing::of(crate::heart::relation_of(npc.id(), player.uuid()), friend_at, hate_at);
            if !s.hours.is_empty() && !doing.is_empty() && doing != s.hours && doing != "event" {
                text.push(format!("The shop is closed, @player. Come back while I'm at {}.", s.hours));
            } else if standing == Standing::Hates {
                text.push("I don't trade with you, @player.".into());
            } else {
                text.push("What will it be, @player?".into());
                // What the player is to its village counts too (see `factions`).
                let (folk, folk_note) = crate::factions::price_factor(host, npc, player);
                if !folk_note.is_empty() {
                    text.push(folk_note.into());
                }
                text.push(
                    match standing {
                        Standing::Friend => "(friend's prices)",
                        Standing::Likes => "(a little off for you)",
                        Standing::Dislikes => "(dearer for you)",
                        _ => "",
                    }
                    .into(),
                );
                // At an event it sells for less.
                if let Some(e) = crate::events::name_at(host, npc).filter(|_| crate::events::price(host, npc, 100) < 100) {
                    text.push(format!("({e} prices!)"));
                }
                let money = shop::short(&s.currency);
                // An NPC with a job sells what its chest holds (see `jobs`).
                let chest = crate::jobs::chest_of(host, npc);
                // What the village has moves the prices (see `market`).
                let mut note: Option<String> = None;
                for (i, o) in s.sells.iter().enumerate() {
                    let (deal, count) = crate::faction::deal(standing.pays(o.price).unwrap_or(o.price), o.count, folk);
                    let n = match chest {
                        // Its job's goods only, and none without a chest.
                        Some(None) => Some(0),
                        Some(Some(c)) => Some((crate::jobs::amount(host, c, &o.item) / u64::from(count.max(1))).min(9999) as u32),
                        None => left(stock_key(host, npc), &id, &s, false, i, o, now),
                    };
                    if n == Some(0) {
                        continue;
                    }
                    let supply = crate::markets::supply_of(host, npc, &o.item);
                    if note.is_none() {
                        note = crate::market::note(&o.item, supply);
                    }
                    let price = crate::market::priced(crate::events::price(host, npc, deal), supply);
                    let stock = n.map(|n| format!(" ({n} left)")).unwrap_or_default();
                    options.push(choice(format!("Buy {count} {} for {price} {money}{stock}", shop::short(&o.item)), vec![Action::Trade(i as i32)]));
                }
                for (i, o) in s.buys.iter().enumerate() {
                    if left(stock_key(host, npc), &id, &s, true, i, o, now) == Some(0) {
                        continue;
                    }
                    let paid = crate::market::priced(standing.is_paid(o.price).unwrap_or(o.price), crate::markets::supply_of(host, npc, &o.item));
                    options.push(choice(format!("Sell {} {} for {paid} {money}", o.count, shop::short(&o.item)), vec![Action::Trade(-(i as i32) - 1)]));
                }
                if !s.buys.is_empty() && s.currency == crate::market::CURRENCY {
                    text.push(format!("(I have {} {money} to buy with.)", crate::markets::purse_of(host, npc)));
                }
                if let Some(n) = note {
                    text.push(n);
                }
                if options.is_empty() {
                    text.push("Sold out. Come back later.".into());
                }
            }
        }
    }
    text.retain(|t| !t.is_empty());
    options.push(choice("That's all.".into(), Vec::new()));
    let mut nodes = HashMap::new();
    nodes.insert("stall".to_string(), Node { text, options, ..Node::default() });
    // The shop it came from goes with the stall's trades.
    let named = named.to_string();
    if let Ok(mut st) = STATE.lock() {
        st.reopen.retain(|r| r.0.uuid() != player.uuid());
    }
    LAST.lock().map(|mut l| l.insert(player.uuid(), named)).ok();
    crate::talk::open(host, player, npc, Arc::new(Dialog { start: "stall".into(), nodes }), "stall");
}

/// The shop each player's stall was opened with (a trade looks it up again).
static LAST: LazyLock<Mutex<HashMap<u128, String>>> = LazyLock::new(Default::default);

/// The `trade` action: offer `n` (sells from 0, buys from -1 down).
pub fn trade(host: &mut dyn ServerHost, player: PlayerRef, npc: EntityRef, n: i32) {
    let named = LAST.lock().ok().and_then(|l| l.get(&player.uuid()).cloned()).unwrap_or_default();
    let Some((id, s)) = shop_of(host, npc, &named) else { return };
    let now = now_of(&*host, npc);
    let (buys, i) = if n >= 0 { (false, n as usize) } else { (true, (-n - 1) as usize) };
    let Some(o) = (if buys { s.buys.get(i) } else { s.sells.get(i) }).cloned() else { return };
    let (friend_at, hate_at) = crate::heart::thresholds(npc.id());
    let standing = Standing::of(crate::heart::relation_of(npc.id(), player.uuid()), friend_at, hate_at);
    let name = player.name(&*host).unwrap_or_default();
    let money = shop::short(&s.currency);
    let chest = crate::jobs::chest_of(host, npc);
    // With a chest, what it sells is what the chest holds, not stock.
    let stocked = match (chest, buys) {
        (Some(Some(_)), false) => None,
        (Some(None), false) => Some(0),
        _ => left(stock_key(host, npc), &id, &s, buys, i, &o, now),
    };
    let chest = chest.flatten();
    let ok = if stocked == Some(0) {
        crate::heart::say_aloud(host, npc, &format!("Sorry, {name}, that's gone."));
        false
    } else if buys {
        let Some(paid) = standing.is_paid(o.price) else { return };
        let paid = crate::market::priced(paid, crate::markets::supply_of(host, npc, &o.item));
        let purse = crate::markets::purse_of(host, npc);
        if s.currency == crate::market::CURRENCY && purse < paid {
            // It pays from its purse: what it earned selling, and its day's pay.
            crate::heart::say_aloud(host, npc, &format!("I can't pay {paid} {money} today, {name}: I have {purse}. Buy something, or come back tomorrow."));
            false
        } else if items_of(host, player, &o.item) < o.count {
            crate::heart::say_aloud(host, npc, &format!("You don't have {} {}, {name}.", o.count, shop::short(&o.item)));
            false
        } else {
            crate::talk::queue(format!("clear {name} {} {}", o.item, o.count));
            crate::talk::queue(format!("give {name} {} {paid}", s.currency));
            // What it buys goes in its chest.
            if let Some(c) = chest {
                crate::jobs::put_item(host, c, &o.item, o.count);
            }
            if s.currency == crate::market::CURRENCY {
                crate::markets::spend(host, npc, paid);
            }
            true
        }
    } else {
        let Some(price) = standing.pays(o.price) else { return };
        // What the player is to its village: a better or worse deal (see `factions`).
        let folk = crate::factions::price_factor(host, npc, player).0;
        let (price, count) = crate::faction::deal(price, o.count, folk);
        let price = crate::market::priced(crate::events::price(host, npc, price), crate::markets::supply_of(host, npc, &o.item));
        if items_of(host, player, &s.currency) < price {
            crate::heart::say_aloud(host, npc, &format!("That's {price} {money}, {name}. You're short."));
            false
        } else if chest.is_some_and(|c| !crate::jobs::take(host, c, &o.item, count)) {
            crate::heart::say_aloud(host, npc, &format!("Sorry, {name}, my chest is out of that."));
            false
        } else {
            crate::talk::queue(format!("clear {name} {} {price}", s.currency));
            crate::talk::queue(format!("give {name} {} {count}", o.item));
            if s.currency == crate::market::CURRENCY {
                crate::markets::earn(host, npc, price);
            }
            true
        }
    };
    if ok {
        take_one(stock_key(host, npc), &id, buys, i);
        crate::talk::queue(format!("execute at {} run playsound minecraft:entity.villager.trade neutral @a ~ ~ ~", crate::talk::uuid_text(npc.uuid(host).unwrap_or(0))));
        crate::heart::feel(host, npc, Some(player), "trade");
        crate::factions::deed_by(host, npc, player, crate::faction::Deed::Traded);
    }
    if let Ok(mut st) = STATE.lock() {
        st.reopen.push((player, npc.id(), named, now + 2));
    }
}

/// The server tick: stalls shown again after a trade (once its commands ran).
pub fn tick(host: &mut dyn ServerHost) {
    let due: Vec<(PlayerRef, u32, String)> = {
        let Ok(mut st) = STATE.lock() else { return };
        if st.reopen.is_empty() {
            return;
        }
        let Some(now) = host.level(crate::pace::level()).and_then(|l| l.game_time(&*host)) else { return };
        // Due, and its NPC in the level ticking now (one whose NPC was lost goes after a while).
        let (due, wait): (Vec<_>, Vec<_>) = std::mem::take(&mut st.reopen).into_iter().partition(|r| r.3 <= now && crate::pace::here(&*host, r.1));
        st.reopen = wait.into_iter().filter(|r| now - r.3 < 400).collect();
        due.into_iter().map(|r| (r.0, r.1, r.2)).collect()
    };
    for (p, npc, named) in due {
        if let Some(e) = host.entity(npc) {
            board(host, p, e, &named);
        }
    }
}

// ---- commands ---------------------------------------------------------------------------------------------

pub fn register_commands(e: &mut RegisterCommandsEvent<'_>) {
    e.commands.push(Command::op("felix shop <targets:entities> <shop:text>", 2));
    e.commands.push(Command::op("felix shop clear <targets:entities>", 2));
}

pub fn run_command(host: &mut dyn ServerHost, e: &mut RunModCommandEvent<'_>) -> Flow {
    let lit: Vec<&str> = e.command.literals.iter().map(String::as_str).collect();
    let set = match lit.as_slice() {
        ["felix", "shop"] => Some(crate::companion::text_argument(e, "shop").unwrap_or_default().trim().to_string()),
        ["felix", "shop", "clear"] => None,
        _ => return Flow::Continue,
    };
    let npcs: Vec<EntityRef> = crate::companion::targets(e).into_iter().filter(|t| crate::companion::is_npc(&*host, *t)).collect();
    let n = npcs.iter().filter(|t| SHOP.set(host, Holder::Entity(t.id()), set.clone()).is_ok()).count();
    let line = match &set {
        Some(id) if shop_named(id).is_some() => format!("{n} NPCs now keep the shop {id}"),
        Some(id) => format!("{n} NPCs now keep the shop {id} (no such shop is loaded yet)"),
        None => format!("{n} NPCs keep no shop by command now"),
    };
    let _ = host.commands().feedback(Text::literal(line));
    Flow::Cancel
}
