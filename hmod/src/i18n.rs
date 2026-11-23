//! The mod's texts in the player's language. Every text the mod shows (menu, HUD, chat) has a key
//! in `assets/blazematica/lang/`; the game's language layers give its text (`translate`), with
//! the English file built into the mod as the fallback.
//!
//! `refresh` (each client tick, each menu frame) reads the whole table again when the game's
//! language changed; `tr!` then works anywhere, also off the game thread. Placeholders are the
//! game's: `%s` in order, `%1$s` by position, `%%` for a percent sign.

use mod_api::ClientHost;
use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::sync::{LazyLock, Mutex, RwLock};

/// The English texts, built in.
const EN_JSON: &str = include_str!("../assets/blazematica/lang/en_us.json");

/// The English texts by key without the `blazematica.` prefix (as `tr!` takes them).
static EN: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    parse(EN_JSON).into_iter().filter_map(|(k, v)| Some((k.strip_prefix(PREFIX)?.to_string(), v))).collect()
});

const PREFIX: &str = "blazematica.";

#[derive(Default)]
struct Table {
    /// The language the table is for (`refresh`), None before the first refresh.
    lang: Option<String>,
    texts: HashMap<String, String>,
    /// Item ids -> the game's name for them, in this language.
    names: HashMap<String, String>,
}

static TABLE: LazyLock<RwLock<Table>> = LazyLock::new(|| RwLock::new(Table::default()));
/// Item ids asked for whose name isn't known yet (looked up at the next refresh).
static WANTED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// Reads the texts again if the game's language changed, and looks up the item names asked for.
pub fn refresh(host: &dyn ClientHost) {
    // The game's language code, and one of our own texts (in case a resource pack changes it).
    let lang = format!(
        "{}|{}",
        host.translate("language.code").unwrap_or_default(),
        host.translate(&format!("{PREFIX}menu.done")).unwrap_or_default()
    );
    let stale = TABLE.read().map_or(true, |t| t.lang.as_deref() != Some(lang.as_str()));
    if stale {
        let texts = EN.keys().map(|k| (k.clone(), host.translate(&format!("{PREFIX}{k}")).unwrap_or_else(|| EN[k].clone()))).collect();
        if let Ok(mut t) = TABLE.write() {
            *t = Table { lang: Some(lang), texts, names: HashMap::new() };
        }
    }
    let wanted: Vec<String> = WANTED.lock().map(|mut w| w.drain().collect()).unwrap_or_default();
    if wanted.is_empty() {
        return;
    }
    let found: Vec<(String, String)> = wanted
        .into_iter()
        .map(|id| {
            let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id.as_str()));
            let name = host
                .translate(&format!("block.{ns}.{path}"))
                .or_else(|| host.translate(&format!("item.{ns}.{path}")))
                .unwrap_or_else(|| pretty(path));
            (id, name)
        })
        .collect();
    if let Ok(mut t) = TABLE.write() {
        t.names.extend(found);
    }
}

/// The text for `key` (without the `blazematica.` prefix; the key itself when no file has it).
pub fn t(key: &str) -> String {
    if let Ok(t) = TABLE.read()
        && let Some(v) = t.texts.get(key)
    {
        return v.clone();
    }
    EN.get(key).cloned().unwrap_or_else(|| key.to_string())
}

/// The text for `key` with its placeholders filled.
pub fn tf(key: &str, args: &[&dyn Display]) -> String {
    format_args_in(&t(key), args)
}

/// The text with `%s`, `%n$s` and `%%` filled from `args`.
fn format_args_in(template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = 0;
    let mut chars = template.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let rest = &template[i + 1..];
        if rest.starts_with('%') {
            out.push('%');
            chars.next();
        } else if rest.starts_with('s') {
            if let Some(a) = args.get(next) {
                out += &a.to_string();
            }
            next += 1;
            chars.next();
        } else if let Some(dollar) = rest.find("$s").filter(|&d| d > 0 && rest[..d].bytes().all(|b| b.is_ascii_digit())) {
            let n: usize = rest[..dollar].parse().unwrap_or(0);
            if let Some(a) = n.checked_sub(1).and_then(|n| args.get(n)) {
                out += &a.to_string();
            }
            for _ in 0..dollar + 2 {
                chars.next();
            }
        } else {
            out.push('%');
        }
    }
    out
}

/// `tr!("key")` or `tr!("key", a, b)`: the text in the player's language (`key` without the
/// `blazematica.` prefix).
#[macro_export]
macro_rules! tr {
    ($key:expr) => {
        $crate::i18n::t($key)
    };
    ($key:expr, $($arg:expr),+ $(,)?) => {
        $crate::i18n::tf($key, &[$(&$arg as &dyn ::std::fmt::Display),+])
    };
}

/// "ON" or "OFF" in the player's language.
pub fn on_off(on: bool) -> String {
    t(if on { "on" } else { "off" })
}

/// The game's name for an item (`minecraft:oak_planks` → "Oak Planks", in the player's language).
/// Asked for the first time, the id's own words stand in until the next refresh.
pub fn item_name(id: &str) -> String {
    if let Ok(t) = TABLE.read()
        && let Some(n) = t.names.get(id)
    {
        return n.clone();
    }
    if let Ok(mut w) = WANTED.lock() {
        w.insert(id.to_string());
    }
    pretty(id.split_once(':').map_or(id, |(_, p)| p))
}

/// `oak_planks` → `Oak Planks`.
fn pretty(path: &str) -> String {
    path.split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A flat JSON object of strings, as the lang files are.
fn parse(json: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut chars = json.chars().peekable();
    let mut strings: Vec<String> = Vec::new();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut s = String::new();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('u') => {
                        let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                        if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                            s.push(ch);
                        }
                    }
                    Some(other) => s.push(other),
                    None => {}
                },
                c => s.push(c),
            }
        }
        strings.push(s);
        if strings.len() == 2 {
            let v = strings.pop().unwrap_or_default();
            let k = strings.pop().unwrap_or_default();
            map.insert(k, v);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders() {
        assert_eq!(format_args_in("%s of %s", &[&1, &2]), "1 of 2");
        assert_eq!(format_args_in("%2$s / %1$s", &[&"a", &"b"]), "b / a");
        assert_eq!(format_args_in("%s%% done", &[&"50.0"]), "50.0% done");
        assert_eq!(format_args_in("100%", &[]), "100%");
    }

    #[test]
    fn english_file_parses() {
        assert_eq!(EN.get("setting.render").map(String::as_str), Some("Rendering"));
        assert!(EN.len() > 50);
    }
}
