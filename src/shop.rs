//! What an NPC sells and buys, and at what price for whom. No mod API types here.
//!
//! A shop is `data/<namespace>/felix_shops/<name>.json`, or `shop.json` in an NPC made in the game:
//!
//! ```json
//! { "currency": "minecraft:emerald", "restock": 10, "hours": "work",
//!   "sells": [ { "item": "minecraft:bread", "count": 3, "price": 1, "stock": 8 } ],
//!   "buys":  [ { "item": "minecraft:wheat", "count": 10, "price": 1, "stock": 16 } ] }
//! ```
//!
//! Prices are what a stranger pays (or is paid): a friend pays a fifth less and is paid a fifth
//! more, one it likes a tenth; one it dislikes pays a quarter more and gets a fifth less; one it
//! hates gets nothing at all. `stock` (none: no end) comes back every `restock` minutes. `hours`:
//! when it lives by a day, it trades only while doing that.

use crate::serde_json::{Map, Value, json};

#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub item: String,
    pub count: u32,
    pub price: u32,
    pub stock: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shop {
    pub currency: String,
    pub sells: Vec<Offer>,
    pub buys: Vec<Offer>,
    /// Minutes.
    pub restock: f32,
    pub hours: String,
}

/// How a relation stands for trading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Standing {
    Friend,
    Likes,
    Neutral,
    Dislikes,
    Hates,
}

impl Standing {
    pub fn of(relation: i32, friend_at: i32, hate_at: i32) -> Standing {
        if relation >= friend_at {
            Standing::Friend
        } else if relation >= 10 {
            Standing::Likes
        } else if relation > -10 {
            Standing::Neutral
        } else if relation > hate_at {
            Standing::Dislikes
        } else {
            Standing::Hates
        }
    }

    /// What one in this standing pays for what costs `base` (None: no trade).
    pub fn pays(self, base: u32) -> Option<u32> {
        let f = match self {
            Standing::Friend => 0.8,
            Standing::Likes => 0.9,
            Standing::Neutral => 1.0,
            Standing::Dislikes => 1.25,
            Standing::Hates => return None,
        };
        Some(((base as f32 * f).round() as u32).max(1))
    }

    /// What one in this standing is paid for what is worth `base`.
    pub fn is_paid(self, base: u32) -> Option<u32> {
        let f = match self {
            Standing::Friend => 1.2,
            Standing::Likes => 1.1,
            Standing::Neutral => 1.0,
            Standing::Dislikes => 0.8,
            Standing::Hates => return None,
        };
        Some(((base as f32 * f).round() as u32).max(1))
    }
}

fn id(x: &str) -> String {
    let x = x.trim();
    if x.contains(':') { x.to_string() } else { format!("minecraft:{x}") }
}

fn offers(v: Option<&Value>, what: &str) -> Result<Vec<Offer>, String> {
    let Some(v) = v else { return Ok(Vec::new()) };
    let list = v.as_array().ok_or(format!("{what} is not a list"))?;
    list.iter()
        .enumerate()
        .map(|(i, o)| {
            let item = o.get("item").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or(format!("{what} {}: no item", i + 1))?;
            Ok(Offer {
                item: id(item),
                count: o.get("count").and_then(Value::as_u64).unwrap_or(1).clamp(1, 64) as u32,
                price: o.get("price").and_then(Value::as_u64).unwrap_or(1).clamp(1, 64 * 9) as u32,
                stock: o.get("stock").and_then(Value::as_u64).map(|s| s.min(9999) as u32),
            })
        })
        .collect()
}

pub fn parse(bytes: &[u8]) -> Result<Shop, String> {
    let root: Value = crate::serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
    read(&root)
}

pub fn read(v: &Value) -> Result<Shop, String> {
    let m = v.as_object().ok_or("not an object")?;
    let shop = Shop {
        currency: id(m.get("currency").and_then(Value::as_str).unwrap_or("minecraft:emerald")),
        sells: offers(m.get("sells"), "sells")?,
        buys: offers(m.get("buys"), "buys")?,
        restock: m.get("restock").and_then(Value::as_f64).unwrap_or(10.0).max(0.5) as f32,
        hours: m.get("hours").and_then(Value::as_str).unwrap_or("").to_string(),
    };
    if shop.sells.is_empty() && shop.buys.is_empty() {
        return Err("it neither sells nor buys".into());
    }
    Ok(shop)
}

/// An item id in a few words: `minecraft:iron_ingot` → `iron ingot`.
pub fn short(item: &str) -> String {
    item.rsplit(':').next().unwrap_or(item).replace('_', " ")
}

// ---- the maker's rows ---------------------------------------------------------------------------------

/// One offer as the maker shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub buys: bool,
    pub item: String,
    pub count: String,
    pub price: String,
}

/// The maker's offers as a shop file (none: no shop).
pub fn rows_to_json(rows: &[Row]) -> Result<Option<Value>, String> {
    if rows.is_empty() {
        return Ok(None);
    }
    let (mut sells, mut buys) = (Vec::new(), Vec::new());
    for (n, r) in rows.iter().enumerate() {
        if r.item.trim().is_empty() {
            return Err(format!("Offer {}: which item?", n + 1));
        }
        let num = |s: &str, what: &str| -> Result<u32, String> {
            if s.trim().is_empty() {
                return Ok(1);
            }
            s.trim().parse::<u32>().ok().filter(|v| *v > 0).ok_or(format!("Offer {}: the {what} is a number", n + 1))
        };
        let o = json!({ "item": id(&r.item), "count": num(&r.count, "count")?, "price": num(&r.price, "price")? });
        if r.buys { buys.push(o) } else { sells.push(o) }
    }
    let mut m = Map::new();
    if !sells.is_empty() {
        m.insert("sells".into(), Value::Array(sells));
    }
    if !buys.is_empty() {
        m.insert("buys".into(), Value::Array(buys));
    }
    let v = Value::Object(m);
    read(&v)?;
    Ok(Some(v))
}

/// A shop file back as rows, when rows can show all of it (else None: it is kept as it is).
pub fn rows_from_json(v: &Value) -> Option<Vec<Row>> {
    let m = v.as_object()?;
    if m.keys().any(|k| k != "sells" && k != "buys") {
        return None;
    }
    let mut rows = Vec::new();
    for (key, buys) in [("sells", false), ("buys", true)] {
        for o in m.get(key).and_then(Value::as_array).cloned().unwrap_or_default() {
            let om = o.as_object()?;
            if om.keys().any(|k| !["item", "count", "price"].contains(&k.as_str())) {
                return None;
            }
            let n = |k: &str| om.get(k).and_then(Value::as_u64).map(|x| x.to_string()).unwrap_or_default();
            rows.push(Row { buys, item: om.get("item")?.as_str()?.to_string(), count: n("count"), price: n("price") });
        }
    }
    Some(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_read() {
        for text in [include_str!("../data/projectfelix/felix_shops/baker.json"), include_str!("../data/projectfelix/felix_shops/smith.json")] {
            let s = parse(text.as_bytes()).unwrap();
            assert_eq!(s.currency, "minecraft:emerald");
            assert!(!s.sells.is_empty() && !s.buys.is_empty());
        }
    }

    #[test]
    fn prices_follow_relations() {
        assert_eq!(Standing::of(50, 30, -50), Standing::Friend);
        assert_eq!(Standing::of(0, 30, -50), Standing::Neutral);
        assert_eq!(Standing::of(-30, 30, -50), Standing::Dislikes);
        assert_eq!(Standing::of(-60, 30, -50), Standing::Hates);
        assert_eq!(Standing::Friend.pays(10), Some(8));
        assert_eq!(Standing::Friend.pays(1), Some(1), "never free");
        assert_eq!(Standing::Dislikes.pays(4), Some(5));
        assert_eq!(Standing::Hates.pays(4), None);
        assert_eq!(Standing::Friend.is_paid(5), Some(6));
        assert_eq!(Standing::Dislikes.is_paid(5), Some(4));
        assert!(parse(br#"{ "sells": [] }"#).is_err());
        assert!(parse(br#"{ "sells": [ { "count": 2 } ] }"#).is_err());
        let s = parse(br#"{ "buys": [ { "item": "wheat", "count": 10 } ] }"#).unwrap();
        assert_eq!(s.buys[0].item, "minecraft:wheat");
        assert_eq!(s.buys[0].stock, None);
        assert_eq!(short("minecraft:iron_ingot"), "iron ingot");
    }

    #[test]
    fn rows_round_trip() {
        let rows = vec![
            Row { buys: false, item: "minecraft:bread".into(), count: "3".into(), price: "1".into() },
            Row { buys: true, item: "minecraft:wheat".into(), count: "10".into(), price: "2".into() },
        ];
        let v = rows_to_json(&rows).unwrap().unwrap();
        assert_eq!(rows_from_json(&v), Some(rows));
        assert_eq!(rows_to_json(&[]).unwrap(), None);
        assert!(rows_to_json(&[Row::default()]).is_err());
        assert!(rows_to_json(&[Row { item: "x".into(), price: "lots".into(), ..Row::default() }]).is_err());
        assert!(rows_from_json(&json!({ "sells": [ { "item": "a", "stock": 3 } ] })).is_none());
    }
}
