use tauri::{Emitter, Manager};
use std::sync::Mutex;

fn ts() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() % 86_400_000;
    format!("{:02}:{:02}:{:02}.{:03}", ms / 3_600_000, ms % 3_600_000 / 60_000, ms % 60_000 / 1_000, ms % 1_000)
}

struct OverlayPos(Mutex<(i32, i32)>);
struct PoeSession(Mutex<(String, String)>); // (cookie_name, cookie_value)

#[cfg(target_os = "linux")]
struct CtrlCDevice(Mutex<Option<evdev::uinput::VirtualDevice>>);


#[derive(serde::Serialize, Clone)]
struct ParsedMod {
    text: String,
    value: f64,
    stat_id: Option<String>,
    mod_group: String, // "explicit" | "implicit" | "corrupted" | "enchant"
}

#[derive(serde::Deserialize, Clone)]
struct StatEntry {
    id: String,
    text: String,
}

struct StatCache(Mutex<std::collections::HashMap<String, Vec<StatEntry>>>);

#[derive(serde::Serialize, Clone)]
struct BaseStat {
    id: String,
    label: String,
    value: f64,
}

#[derive(serde::Serialize, Clone)]
struct ParsedItem {
    name: String,
    base_type: String,
    rarity: String,
    item_level: u32,
    influence: String,
    game_mode: String,
    item_class: String,
    base_stats: Vec<BaseStat>,
    corrupted: bool,
    mods: Vec<ParsedMod>,
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[cfg_attr(not(target_os = "linux"), tauri::command)]
#[cfg_attr(target_os = "linux", tauri::command)]
fn hide_overlay(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.clone().run_on_main_thread(move || {
            #[cfg(target_os = "linux")]
            {
                use gtk_layer_shell::{KeyboardMode, LayerShell};
                if let Ok(gtk_win) = w.gtk_window() {
                    // Set None while still mapped so KWin returns focus to PoE2
                    // BEFORE the surface is unmapped. Setting it after hide is a no-op.
                    gtk_win.set_keyboard_mode(KeyboardMode::None);
                }
            }
            let _ = w.hide();
        });
    }
}

#[tauri::command]
fn set_overlay_keyboard(app: tauri::AppHandle, enabled: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.clone().run_on_main_thread(move || {
            #[cfg(target_os = "linux")]
            {
                use gtk_layer_shell::{KeyboardMode, LayerShell};
                if let Ok(gtk_win) = w.gtk_window() {
                    let mode = if enabled { KeyboardMode::OnDemand } else { KeyboardMode::None };
                    gtk_win.set_keyboard_mode(mode);
                }
            }
        });
    }
}

#[tauri::command]
fn move_overlay(app: tauri::AppHandle, state: tauri::State<OverlayPos>, dx: i32, dy: i32) {
    let new_pos = {
        let mut p = state.0.lock().unwrap();
        p.0 = (p.0 + dx).max(0);
        p.1 = (p.1 + dy).max(0);
        *p
    };
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.clone().run_on_main_thread(move || {
            #[cfg(target_os = "linux")]
            {
                use gtk_layer_shell::{Edge, LayerShell};
                if let Ok(gtk_win) = w.gtk_window() {
                    gtk_win.set_layer_shell_margin(Edge::Left, new_pos.0);
                    gtk_win.set_layer_shell_margin(Edge::Top, new_pos.1);
                }
            }
        });
    }
}

#[tauri::command]
fn save_overlay_position(state: tauri::State<OverlayPos>, app: tauri::AppHandle) {
    let (x, y) = *state.0.lock().unwrap();
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(
            dir.join("position.json"),
            format!("{{\"x\":{},\"y\":{}}}", x, y),
        );
    }
}

const KEYRING_SERVICE: &str = "io.github.5h4rkbyt3.exilewatch";
const KEYRING_USER:    &str = "poe_session";

fn save_session_config(app: &tauri::AppHandle, name: &str, value: &str) {
    // Store the token value in the OS keyring (KWallet on KDE).
    // Only fall back to config.json if the keyring is unavailable.
    let keyring_ok = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .and_then(|e| e.set_password(value))
        .map_err(|e| eprintln!("[EW] keyring write failed: {e} — falling back to config.json"))
        .is_ok();

    if let Ok(dir) = app.path().app_data_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let n = serde_json::to_string(name).unwrap_or_default();
        // Only write the value to disk if the keyring is unavailable
        let json = if keyring_ok {
            format!("{{\"cookie_name\":{n}}}")
        } else {
            let v = serde_json::to_string(value).unwrap_or_default();
            format!("{{\"cookie_name\":{n},\"cookie_value\":{v}}}")
        };
        let _ = std::fs::write(dir.join("config.json"), json);
    }
}

#[tauri::command]
fn get_session_id(state: tauri::State<PoeSession>) -> String {
    state.0.lock().unwrap().1.clone()
}

#[tauri::command]
fn set_session_id(state: tauri::State<PoeSession>, app: tauri::AppHandle, id: String) {
    if id.is_empty() {
        // Clear: wipe in-memory state and remove from keyring
        *state.0.lock().unwrap() = Default::default();
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
            let _ = entry.delete_password();
        }
    } else {
        // Manual entry assumes the legacy POESESSID format
        *state.0.lock().unwrap() = ("POESESSID".to_string(), id.clone());
        save_session_config(&app, "POESESSID", &id);
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct League {
    id: String,
    text: String,
}

#[tauri::command]
async fn fetch_leagues(
    game_mode: String,
    session: tauri::State<'_, PoeSession>,
) -> Result<Vec<League>, String> {
    let url = if game_mode == "poe2" {
        "https://www.pathofexile.com/api/trade2/data/leagues"
    } else {
        "https://www.pathofexile.com/api/trade/data/leagues"
    };
    let (cookie_name, cookie_value) = session.0.lock().unwrap().clone();

    #[derive(serde::Deserialize)]
    struct Resp { result: Vec<League> }

    let client = reqwest::Client::builder()
        .user_agent("ExileWatch/0.1")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let mut req = client.get(url);
    if !cookie_value.is_empty() {
        req = req.header("Cookie", format!("{cookie_name}={cookie_value}"));
    }

    req.send()
        .await
        .map_err(|e| format!("network: {e}"))?
        .json::<Resp>()
        .await
        .map_err(|e| format!("parse: {e}"))
        .map(|r| r.result)
}

// ── Clipboard + item parsing ──────────────────────────────────────────────────

fn read_wl_paste_raw() -> String {
    let Ok(out) = std::process::Command::new("wl-paste").arg("--no-newline").output() else {
        return String::new();
    };
    if out.status.success() {
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    } else {
        String::new()
    }
}

// Read X11 CLIPBOARD selection directly from the owning process (PoE2/Wine).
// Unlike wl-paste, this bypasses KWin's Wayland clipboard cache — so when PoE2
// updates its clipboard content WITHOUT releasing X11 selection ownership, we
// still see the fresh content on every call.
fn read_x11_clipboard() -> String {
    let Ok(out) = std::process::Command::new("xclip")
        .args(["-selection", "clipboard", "-o"])
        .output() else {
        return String::new();
    };
    if out.status.success() {
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    } else {
        String::new()
    }
}


fn collect_numbers(s: &str) -> Vec<f64> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        let signed = (c == '+' || c == '-')
            && i + 1 < b.len()
            && b[i + 1].is_ascii_digit();
        if c.is_ascii_digit() || signed {
            let start = i;
            i += 1;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            if let Ok(n) = s[start..i].parse::<f64>() {
                out.push(n);
            }
        } else {
            i += 1;
        }
    }
    out
}

fn numbers_to_hash(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        let signed = (c == '+' || c == '-')
            && i + 1 < b.len()
            && b[i + 1].is_ascii_digit();
        if signed {
            out.push(c);
            out.push('#');
            i += 2;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
        } else if c.is_ascii_digit() {
            out.push('#');
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

// Strip trailing qualifier annotations like "(augmented)", "(crafted)", "(fractured)".
// These only appear in PoE clipboard text, never in GGG stat text.
fn strip_qualifier(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_suffix(')') {
        if let Some(pos) = rest.rfind('(') {
            let inner = &rest[pos + 1..];
            if !inner.is_empty() && inner.chars().all(|c| c.is_alphabetic() || c == ' ') {
                return rest[..pos].trim_end();
            }
        }
    }
    s
}

// Strip inline roll-range annotations before numbers are hashed.
// Handles both "(10-15)" (PoE1) and "(-17-+17)" (PoE2 signed ranges).
// Pattern: '(' [-+]? digits '-' [-+]? digits ')'
// Must run on raw clipboard text (before numbers_to_hash) because it matches digits.
fn strip_roll_ranges(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '(' {
            let start = i;
            let mut j = i + 1;
            // Optional sign on first number
            if j < chars.len() && (chars[j] == '-' || chars[j] == '+') { j += 1; }
            let n1 = j;
            while j < chars.len() && chars[j].is_ascii_digit() { j += 1; }
            if j > n1 && j < chars.len() && chars[j] == '-' {
                j += 1;
                // Optional sign on second number
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') { j += 1; }
                let n2 = j;
                while j < chars.len() && chars[j].is_ascii_digit() { j += 1; }
                if j > n2 && j < chars.len() && chars[j] == ')' {
                    i = j + 1; // skip the range entirely
                    continue;
                }
            }
            // Not a range — emit the '(' and resume from char after it
            out.push('(');
            i = start + 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn find_stat<'a>(needle: &str, entries: &'a [StatEntry]) -> Option<&'a StatEntry> {
    entries.iter().find(|e| e.text.trim().to_lowercase() == needle)
}

fn match_stat_id(mod_text: &str, entries: &[StatEntry]) -> Option<String> {
    let needle = mod_text.trim().to_lowercase();

    // 1. Exact match
    if let Some(e) = find_stat(&needle, entries) { return Some(e.id.clone()); }

    // 2. PoE2: "+# to X"  →  "+# total X"  (life, mana, energy shield…)
    if let Some(rest) = needle.strip_prefix("+# to ") {
        let candidate = format!("+# total {rest}");
        if let Some(e) = find_stat(&candidate, entries) { return Some(e.id.clone()); }
    }

    // 3. PoE2: "+#% to X"  →  "+#% total to X"  (resistances)
    if let Some(rest) = needle.strip_prefix("+#% to ") {
        let candidate = format!("+#% total to {rest}");
        if let Some(e) = find_stat(&candidate, entries) { return Some(e.id.clone()); }
    }

    // 4. PoE2: "#% to X"  →  "#% total to X"
    if let Some(rest) = needle.strip_prefix("#% to ") {
        let candidate = format!("#% total to {rest}");
        if let Some(e) = find_stat(&candidate, entries) { return Some(e.id.clone()); }
    }

    // 5. GGG omits leading '+' on some stats (e.g. "+# to Spirit" → "# to Spirit")
    if let Some(rest) = needle.strip_prefix('+') {
        if let Some(e) = find_stat(rest, entries) { return Some(e.id.clone()); }
    }

    None
}

async fn load_stats(game_mode: &str, cookie_name: &str, cookie_value: &str) -> Result<Vec<StatEntry>, String> {
    let url = if game_mode == "poe2" {
        "https://www.pathofexile.com/api/trade2/data/stats"
    } else {
        "https://www.pathofexile.com/api/trade/data/stats"
    };

    #[derive(serde::Deserialize)]
    struct Group { entries: Vec<StatEntry> }
    #[derive(serde::Deserialize)]
    struct Resp { result: Vec<Group> }

    let client = reqwest::Client::builder()
        .user_agent("ExileWatch/0.1")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let mut req = client.get(url);
    if !cookie_value.is_empty() {
        req = req.header("Cookie", format!("{cookie_name}={cookie_value}"));
    }

    req.send()
        .await
        .map_err(|e| format!("network: {e}"))?
        .json::<Resp>()
        .await
        .map_err(|e| format!("parse: {e}"))
        .map(|r| r.result.into_iter().flat_map(|g| g.entries).collect())
}

fn is_adds_range_mod(line: &str) -> bool {
    // "Adds 14 to 28 Lightning Damage" — the token after "to" is a bare number.
    // "+14 to Strength" — the token after "to" is a word, so NOT a range mod.
    // This prevents misidentifying PoE2 mods like "+14(10-15) to Strength" as ranges.
    let Some(pos) = line.find(" to ") else { return false };
    let after = line[pos + 4..].trim_start();
    after.starts_with(|c: char| c.is_ascii_digit())
}

fn parse_mod_line(line: &str) -> Option<ParsedMod> {
    // Normalise before collecting numbers: strip trailing qualifiers like
    // "(augmented)" and inline roll ranges like "(10-15)" while digits are still digits.
    let line = strip_qualifier(line);
    let stripped = strip_roll_ranges(line);
    let line = stripped.trim();

    let nums = collect_numbers(line);
    if nums.is_empty() {
        return None;
    }
    // Two-value range mods like "Adds 14 to 28 Lightning Damage" → average.
    // Single-value mods like "+14 to Strength" → the roll.
    let value = if nums.len() >= 2 && is_adds_range_mod(line) {
        (nums[0].abs() + nums[1].abs()) / 2.0
    } else {
        nums[0].abs()
    };
    Some(ParsedMod { text: numbers_to_hash(line), value, stat_id: None, mod_group: String::new() })
}

fn parse_damage_range(s: &str) -> Option<(f64, f64)> {
    let (a, b) = s.trim().split_once('-')?;
    let a = a.trim().parse::<f64>().ok()?;
    let b = b.trim().parse::<f64>().ok()?;
    Some((a, b))
}

fn round1(v: f64) -> f64 { (v * 10.0).round() / 10.0 }

fn parse_base_stats(sections: &[&str], il_idx: usize) -> Vec<BaseStat> {
    let mut phys = (0.0f64, 0.0f64);
    let mut elem = (0.0f64, 0.0f64);
    let mut aps: Option<f64> = None;
    let mut crit: Option<f64> = None;
    let mut ar: Option<f64> = None;
    let mut ev: Option<f64> = None;
    let mut es: Option<f64> = None;
    let mut ward: Option<f64> = None;
    let mut block: Option<f64> = None;

    for section in sections.iter().take(il_idx).skip(1) {
        for raw_line in section.trim().lines() {
            let line = strip_qualifier(raw_line.trim()).trim();
            if line.starts_with("Quality:") || line.starts_with("Requires:") || line.starts_with("Sockets:") {
                continue;
            }
            if let Some(v) = line.strip_prefix("Physical Damage: ") {
                if let Some(r) = parse_damage_range(v) { phys = r; }
            } else if let Some(v) = line.strip_prefix("Fire Damage: ")
                    .or_else(|| line.strip_prefix("Cold Damage: "))
                    .or_else(|| line.strip_prefix("Lightning Damage: "))
                    .or_else(|| line.strip_prefix("Chaos Damage: ")) {
                if let Some(r) = parse_damage_range(v) { elem.0 += r.0; elem.1 += r.1; }
            } else if let Some(v) = line.strip_prefix("Attacks per Second: ") {
                aps = v.trim().parse().ok();
            } else if let Some(v) = line.strip_prefix("Critical Hit Chance: ") {
                crit = v.trim().trim_end_matches('%').trim().parse().ok();
            } else if let Some(v) = line.strip_prefix("Armour: ") {
                ar = v.trim().replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("Evasion Rating: ") {
                ev = v.trim().replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("Energy Shield: ") {
                es = v.trim().replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("Ward: ") {
                ward = v.trim().replace(',', "").parse().ok();
            } else if let Some(v) = line.strip_prefix("Block Chance: ") {
                block = v.trim().trim_end_matches('%').trim().parse().ok();
            }
        }
    }

    let mut stats: Vec<BaseStat> = Vec::new();

    if let Some(a) = aps {
        let pdps = (phys.0 + phys.1) / 2.0 * a;
        let edps = (elem.0 + elem.1) / 2.0 * a;
        let dps = pdps + edps;
        if dps  > 0.0 { stats.push(BaseStat { id: "dps".into(),  label: "DPS".into(),           value: round1(dps) }); }
        if pdps > 0.0 { stats.push(BaseStat { id: "pdps".into(), label: "PDPS".into(),          value: round1(pdps) }); }
        if edps > 0.0 { stats.push(BaseStat { id: "edps".into(), label: "EDPS".into(),          value: round1(edps) }); }
        if let Some(c) = crit { stats.push(BaseStat { id: "crit".into(), label: "Crit %".into(), value: c }); }
        stats.push(BaseStat { id: "aps".into(), label: "APS".into(), value: a });
    }

    if let Some(v) = ar    { stats.push(BaseStat { id: "ar".into(),    label: "Armour".into(),        value: v }); }
    if let Some(v) = ev    { stats.push(BaseStat { id: "ev".into(),    label: "Evasion".into(),        value: v }); }
    if let Some(v) = es    { stats.push(BaseStat { id: "es".into(),    label: "Energy Shield".into(),  value: v }); }
    if let Some(v) = ward  { stats.push(BaseStat { id: "ward".into(),  label: "Ward".into(),           value: v }); }
    if let Some(v) = block { stats.push(BaseStat { id: "block".into(), label: "Block %".into(),        value: v }); }

    stats
}

fn item_class_to_category(class: &str) -> Option<&'static str> {
    match class {
        "Helmets"                             => Some("armour.head"),
        "Body Armours"                        => Some("armour.chest"),
        "Gloves"                              => Some("armour.gloves"),
        "Boots"                               => Some("armour.boots"),
        "Belts"                               => Some("armour.belt"),
        "Amulets"                             => Some("accessory.amulet"),
        "Rings"                               => Some("accessory.ring"),
        "Spears"                              => Some("weapon.spear"),
        "Bows"                                => Some("weapon.bow"),
        "Crossbows"                           => Some("weapon.crossbow"),
        "Quarterstaves"                       => Some("weapon.quarterstaff"),
        "Shields"                             => Some("armour.shield"),
        "Wands"                               => Some("weapon.wand"),
        "Sceptres"                            => Some("weapon.sceptre"),
        "Flails"                              => Some("weapon.flail"),
        "Foci"                                => Some("armour.focus"),
        "Quivers"                             => Some("armour.quiver"),
        "One Hand Swords" | "Swords"          => Some("weapon.onesword"),
        "Two Hand Swords"                     => Some("weapon.twosword"),
        "One Hand Axes" | "Axes"              => Some("weapon.oneaxe"),
        "Two Hand Axes"                       => Some("weapon.twoaxe"),
        "One Hand Maces" | "Maces" | "Clubs" => Some("weapon.onemace"),
        "Two Hand Maces"                      => Some("weapon.twomace"),
        "Trinkets"                            => Some("accessory.trinket"),
        _                                     => None,
    }
}

const INFLUENCE_TAGS: &[&str] = &[
    "Hunter Item", "Shaper Item", "Elder Item",
    "Crusader Item", "Redeemer Item", "Warlord Item",
];
const SKIP_TAGS: &[&str] = &[
    "Hunter Item", "Shaper Item", "Elder Item", "Crusader Item",
    "Redeemer Item", "Warlord Item", "Synthesised Item", "Fractured Item",
    "Corrupted", "Mirrored", "Unidentified",
];

fn parse_poe_item(text: &str) -> Option<ParsedItem> {
    if !text.contains("Rarity:") || !text.contains("Item Level:") {
        return None;
    }

    let sections: Vec<&str> = text.split("--------").collect();
    if sections.len() < 3 {
        return None;
    }

    // Rarity
    let rarity = sections[0].lines()
        .find_map(|l| l.strip_prefix("Rarity: "))
        .map(str::trim)?;

    let rarity_key = match rarity {
        "Normal" => "normal",
        "Magic"  => "magic",
        "Rare"   => "rare",
        "Unique" => "unique",
        _ => return None,
    };

    // Name + base type live in sections[0], after the "Item Class:" and "Rarity:" lines.
    // sections[1] onwards is requirements, item level, mods, etc.
    let name_lines: Vec<&str> = sections[0].trim().lines()
        .filter(|l| {
            let l = l.trim();
            !l.is_empty() && !l.starts_with("Item Class:") && !l.starts_with("Rarity:")
        })
        .collect();

    let (name, base_type) = match rarity_key {
        "rare" | "unique" if name_lines.len() >= 2 =>
            (name_lines[0].trim().to_string(), name_lines[1].trim().to_string()),
        _ if !name_lines.is_empty() =>
            (name_lines[0].trim().to_string(), name_lines[0].trim().to_string()),
        _ => return None,
    };

    // Item level
    let item_level: u32 = text.lines()
        .find_map(|l| l.strip_prefix("Item Level: "))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);

    // Influence from last section
    let influence = sections.last()
        .and_then(|last| {
            INFLUENCE_TAGS.iter()
                .find(|&&tag| last.trim().lines().any(|l| l.trim() == tag))
                .map(|&tag| tag.split_whitespace().next().unwrap_or("").to_string())
        })
        .unwrap_or_default();

    let item_class = sections[0].lines()
        .find_map(|l| l.strip_prefix("Item Class: "))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    // Sections after "Item Level:" section contain mods
    let il_idx = sections.iter().position(|s| s.contains("Item Level: "))?;

    let base_stats = parse_base_stats(&sections, il_idx);

    let corrupted = text.lines().any(|l| l.trim() == "Corrupted");

    let mut mods = Vec::new();
    for section in &sections[il_idx + 1..] {
        let trimmed = section.trim();
        if trimmed.is_empty() { continue; }
        // Skip sections that are entirely skip-tags (e.g. the "Corrupted" section)
        if trimmed.lines().all(|l| {
            let l = l.trim();
            l.is_empty() || SKIP_TAGS.iter().any(|&t| l == t)
        }) { continue; }
        // Track mod source from PoE2 { } descriptor headers
        let mut current_group = "explicit";
        for line in trimmed.lines() {
            let line = line.trim();
            if line.is_empty() { continue; }
            if line.starts_with('(') && line.ends_with(')') { continue; }
            if line.starts_with('{') {
                current_group = if line.contains("Corruption Enhancement") { "corrupted" }
                    else if line.contains("Implicit")    { "implicit" }
                    else if line.contains("Enchantment") { "enchant"  }
                    else                                  { "explicit" };
                continue;
            }
            if SKIP_TAGS.iter().any(|&t| line == t) { continue; }
            if let Some(mut m) = parse_mod_line(line) {
                m.mod_group = current_group.to_string();
                mods.push(m);
            }
        }
    }

    if mods.is_empty() { return None; }

    Some(ParsedItem {
        name, base_type, item_class, base_stats,
        rarity: rarity_key.to_string(),
        item_level, influence, corrupted,
        game_mode: detect_poe_game().to_string(),
        mods,
    })
}

#[cfg(target_os = "linux")]
fn create_ctrl_c_device() -> Option<evdev::uinput::VirtualDevice> {
    use evdev::{AttributeSet, Key, uinput::VirtualDeviceBuilder};
    let mut keys = AttributeSet::<Key>::new();
    keys.insert(Key::KEY_LEFTCTRL);
    keys.insert(Key::KEY_C);
    VirtualDeviceBuilder::new().ok()?
        .name("exilewatch-hotkey")
        .with_keys(&keys).ok()?
        .build().ok()
}

fn detect_poe_game() -> &'static str {
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let n = entry.file_name();
            let name = n.to_str().unwrap_or("");
            if name.parse::<u32>().is_err() { continue; }
            let Ok(cmd) = std::fs::read_to_string(format!("/proc/{}/cmdline", name)) else { continue };
            // Steam/Proton path: "Path of Exile 2\PathOfExileSteam.exe"
            // Native path: "PathOfExile2"
            if cmd.contains("Path of Exile 2") || cmd.contains("PathOfExile2") { return "poe2"; }
            if cmd.contains("PathOfExile") || cmd.contains("Path of Exile") { return "poe1"; }
        }
    }
    "poe1"
}

// ── Layer-shell setup ─────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn monitor_size_at_cursor() -> Option<(i32, i32)> {
    use gdk::prelude::*;
    let display = gdk::Display::default()?;
    let seat = display.default_seat()?;
    let pointer = seat.pointer()?;
    let (_screen, px, py) = pointer.position();
    let monitor = display.monitor_at_point(px, py)?;
    let geo = monitor.geometry();
    Some((geo.width(), geo.height()))
}

#[cfg(target_os = "linux")]
fn init_layer_shell(
    window: &tauri::WebviewWindow<impl tauri::Runtime>,
    saved_x: i32,
    saved_y: i32,
) -> (i32, i32) {
    use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
    let Ok(gtk_win) = window.gtk_window() else { return (saved_x, saved_y) };

    gtk_win.init_layer_shell();
    gtk_win.set_layer(Layer::Overlay);
    gtk_win.set_keyboard_mode(KeyboardMode::None);
    gtk_win.set_exclusive_zone(-1);
    gtk_win.set_anchor(Edge::Top, true);
    gtk_win.set_anchor(Edge::Left, true);

    const W: i32 = 500;
    const H: i32 = 680;
    let (x, y) = if saved_x > 0 || saved_y > 0 {
        (saved_x, saved_y)
    } else {
        let (mw, mh) = monitor_size_at_cursor().unwrap_or((3440, 1440));
        (((mw - W) / 2).max(0), ((mh - H) / 2).max(0))
    };

    gtk_win.set_layer_shell_margin(Edge::Left, x);
    gtk_win.set_layer_shell_margin(Edge::Top, y);
    (x, y)
}

// ── evdev listener ────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn start_evdev_listener(handle: tauri::AppHandle) {
    use evdev::{InputEventKind, Key};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let keyboards: Vec<_> = evdev::enumerate()
        .filter_map(|(_, d)| {
            let supported = d.supported_keys()?;
            if supported.contains(Key::KEY_D) && supported.contains(Key::KEY_LEFTALT) {
                Some(d)
            } else {
                None
            }
        })
        .collect();

    eprintln!("[ExileWatch] evdev: monitoring {} keyboard(s)", keyboards.len());

    // Shared flag: prevents multiple keyboard devices from firing on_alt_d
    // simultaneously for the same physical keypress.
    let in_flight = Arc::new(AtomicBool::new(false));

    for device in keyboards {
        let handle = handle.clone();
        let in_flight = in_flight.clone();
        tauri::async_runtime::spawn(async move {
            let mut stream = match device.into_event_stream() {
                Ok(s) => s,
                Err(e) => { eprintln!("[ExileWatch] evdev stream: {e}"); return; }
            };
            let mut alt_held = false;
            let mut pending_search = false;
            loop {
                let Ok(event) = stream.next_event().await else { break };
                let InputEventKind::Key(key) = event.kind() else { continue };
                match key {
                    Key::KEY_LEFTALT | Key::KEY_RIGHTALT => {
                        let was = alt_held;
                        alt_held = event.value() != 0;
                        if was && !alt_held {
                            if pending_search {
                                pending_search = false;
                                eprintln!("[EW {}] Alt released — firing on_alt_d", ts());
                                // Only the first keyboard to reach this wins;
                                // others see in_flight=true and skip.
                                if !in_flight.swap(true, Ordering::SeqCst) {
                                    let h = handle.clone();
                                    let ifl = in_flight.clone();
                                    tauri::async_runtime::spawn(async move {
                                        on_alt_d(&h).await;
                                        ifl.store(false, Ordering::SeqCst);
                                    });
                                } else {
                                    eprintln!("[EW {}] Alt released — skipped (in_flight)", ts());
                                }
                            } else {
                                if let Some(w) = handle.get_webview_window("main") {
                                    let _ = w.emit("alt-released", ());
                                }
                            }
                        }
                    }
                    Key::KEY_D if event.value() == 1 && alt_held => {
                        if let Some(w) = handle.get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) {
                                eprintln!("[EW {}] Alt+D — hiding overlay", ts());
                                let _ = w.clone().run_on_main_thread(move || {
                                    use gtk_layer_shell::{KeyboardMode, LayerShell};
                                    if let Ok(gtk_win) = w.gtk_window() {
                                        // Release keyboard grab while surface is still
                                        // mapped so KWin returns focus to PoE2.
                                        gtk_win.set_keyboard_mode(KeyboardMode::None);
                                    }
                                    let _ = w.hide();
                                });
                            } else {
                                eprintln!("[EW {}] Alt+D — queuing search", ts());
                                pending_search = true;
                            }
                        }
                    }
                    Key::KEY_ESC if event.value() == 1 => {
                        if let Some(w) = handle.get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) {
                                eprintln!("[EW {}] Escape — hiding overlay", ts());
                                let _ = w.emit("escape-pressed", ());
                            }
                        }
                    }
                    _ => {}
                }
            }
        });
    }
}

#[cfg(target_os = "linux")]
async fn on_alt_d(handle: &tauri::AppHandle) {
    let Some(window) = handle.get_webview_window("main") else { return };

    // 1. Snapshot BEFORE showing overlay — overlay is still hidden, PoE2 has focus.
    eprintln!("[EW {}] on_alt_d: taking snapshot (overlay hidden)", ts());
    let snapshot = tauri::async_runtime::spawn_blocking(read_wl_paste_raw)
        .await.unwrap_or_default();
    eprintln!("[EW {}] snapshot done: {} bytes", ts(), snapshot.len());

    // 2. Inject Ctrl+C while overlay is still hidden.
    eprintln!("[EW {}] injecting Ctrl+C", ts());
    {
        use evdev::{EventType, InputEvent, Key};
        let state = handle.state::<CtrlCDevice>();
        let mut lock = state.0.lock().unwrap();
        if let Some(dev) = lock.as_mut() {
            let ev = |k: Key, v: i32| InputEvent::new(EventType::KEY, k.0, v);
            match dev.emit(&[
                ev(Key::KEY_LEFTCTRL, 1), ev(Key::KEY_C, 1),
                ev(Key::KEY_C, 0), ev(Key::KEY_LEFTCTRL, 0),
            ]) {
                Ok(_)  => eprintln!("[EW {}] Ctrl+C emitted", ts()),
                Err(e) => eprintln!("[EW {}] Ctrl+C emit error: {e}", ts()),
            }
        }
    }

    // 3. Show the overlay — Ctrl+C is already in flight.
    eprintln!("[EW {}] showing overlay", ts());
    let win = window.clone();
    let _ = window.clone().run_on_main_thread(move || {
        // Stay at KeyboardMode::None — prevents WebKit from grabbing focus and
        // intercepting the uinput Ctrl+C that's already in flight to PoE2.
        // The frontend switches to OnDemand via set_overlay_keyboard when the
        // user explicitly clicks a number input.
        let _ = win.show();
        let _ = win.emit("overlay-shown", ());
        let _ = win.emit("search-started", ());
    });

    // 5. Poll wl-paste for PoE item content.
    //    - Non-PoE clipboard changes (game UI text, currency names, etc.) are logged
    //      and skipped — we update the local baseline and keep polling.
    //    - Same-item case: content never changes → fall back to original snapshot,
    //      but ONLY if the snapshot itself looks like a PoE item.
    //    - If a non-item change was seen, we know the cursor wasn't on an item →
    //      return empty even if snapshot was a PoE item.
    eprintln!("[EW {}] starting poll (40 × 25ms = 1s max)", ts());
    let original_snapshot = snapshot.clone();
    let text = tauri::async_runtime::spawn_blocking(move || {
        let mut baseline = snapshot;
        let mut saw_non_item = false;
        for attempt in 0..40 {
            let t0 = std::time::Instant::now();
            std::thread::sleep(std::time::Duration::from_millis(25));
            // xclip reads from PoE2's X11 selection owner directly.
            // wl-paste would return KWin's stale Wayland cache when PoE2
            // reuses the same X11 selection owner window across copies.
            let content = read_x11_clipboard();
            let elapsed = t0.elapsed().as_millis();
            let tag = if content == baseline { "same" }
                      else if content.is_empty() { "empty" }
                      else if content.contains("Rarity:") { "PoE?" }
                      else { "non-item" };
            eprintln!("[EW {}] poll #{:02}: {}ms  {} bytes  {}", ts(), attempt, elapsed, content.len(), tag);

            if !content.is_empty() && content != baseline {
                if content.contains("Rarity:") && content.contains("Item Level:") {
                    eprintln!("[EW {}] PoE item detected at attempt {}", ts(), attempt);
                    return content;
                }
                // Non-PoE clipboard change (UI text, skill names, etc.).
                // Log the actual text so we can see what PoE2 is copying.
                let preview = &content[..content.len().min(120)];
                eprintln!("[EW {}] non-item change at attempt {} — {:?}", ts(), attempt, preview);
                saw_non_item = true;
                baseline = content; // update baseline, keep polling
            }
        }
        // Same-item case: only fall back to original snapshot if it was a PoE item
        // AND we never saw non-item content (which would mean cursor wasn't on an item).
        if !saw_non_item
            && original_snapshot.contains("Rarity:")
            && original_snapshot.contains("Item Level:")
        {
            eprintln!("[EW {}] unchanged — same-item fallback ({} bytes)", ts(), original_snapshot.len());
            original_snapshot
        } else {
            eprintln!("[EW {}] timeout — no PoE item found", ts());
            String::new()
        }
    }).await.unwrap_or_default();
    eprintln!("[EW {}] poll done: {} bytes", ts(), text.len());

    if text.is_empty() {
        eprintln!("[EW {}] no content — hiding overlay", ts());
        let _ = window.emit("search-failed", ());
        return;
    }

    let Some(item) = parse_poe_item(&text) else {
        eprintln!("[EW {}] not a PoE item — hiding overlay", ts());
        let _ = window.emit("search-failed", ());
        return;
    };

    eprintln!("[EW {}] parsed: \"{}\" rarity={} ilvl={} {} mods  game={}", ts(), item.name, item.rarity, item.item_level, item.mods.len(), item.game_mode);

    // Resolve stat IDs — fetch on first use per game mode, then serve from cache.
    let stat_entries: Option<Vec<StatEntry>> = {
        let cache = handle.state::<StatCache>();
        let cached = cache.0.lock().unwrap().get(&item.game_mode).cloned();
        if cached.is_some() {
            cached
        } else {
            let (cname, cval) = handle.state::<PoeSession>().0.lock().unwrap().clone();
            match load_stats(&item.game_mode, &cname, &cval).await {
                Ok(entries) => {
                    eprintln!("[EW {}] cached {} stat entries for {}", ts(), entries.len(), item.game_mode);
                    cache.0.lock().unwrap().insert(item.game_mode.clone(), entries.clone());
                    Some(entries)
                }
                Err(e) => { eprintln!("[EW {}] stat load failed: {e}", ts()); None }
            }
        }
    };

    let item = if let Some(ref entries) = stat_entries {
        let matched: usize = item.mods.iter().filter(|m| match_stat_id(&m.text, entries).is_some()).count();
        eprintln!("[EW {}] stat match: {}/{} mods resolved", ts(), matched, item.mods.len());
        ParsedItem {
            mods: item.mods.into_iter().map(|m| {
                let sid = match_stat_id(&m.text, entries);
                ParsedMod { stat_id: sid, ..m }
            }).collect(),
            ..item
        }

    } else {
        item
    };

    eprintln!("[EW {}] emitting item-data", ts());
    let _ = window.emit("item-data", &item);
}

// ── Startup helpers ───────────────────────────────────────────────────────────

fn load_saved_position(app: &tauri::App) -> (i32, i32) {
    let Ok(dir) = app.path().app_data_dir() else { return (0, 0) };
    let Ok(json) = std::fs::read_to_string(dir.join("position.json")) else { return (0, 0) };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) else { return (0, 0) };
    (v["x"].as_i64().unwrap_or(0) as i32, v["y"].as_i64().unwrap_or(0) as i32)
}

// ── Firefox session detection ─────────────────────────────────────────────────

// Scan a single Firefox base directory for its default profile.
#[cfg(target_os = "linux")]
fn find_profile_in(ff_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let ini = std::fs::read_to_string(ff_dir.join("profiles.ini")).unwrap_or_default();
    let mut cur_path  = String::new();
    let mut cur_rel   = false;
    let mut cur_def   = false;
    let mut in_prof   = false;
    let mut fallback: Option<std::path::PathBuf> = None;

    let resolve = |path: &str, rel: bool| -> std::path::PathBuf {
        if rel { ff_dir.join(path) } else { std::path::PathBuf::from(path) }
    };

    let mut commit = |path: &str, rel: bool, def: bool, fb: &mut Option<std::path::PathBuf>| -> Option<std::path::PathBuf> {
        if path.is_empty() { return None; }
        let full = resolve(path, rel);
        if !full.join("cookies.sqlite").exists() { return None; }
        if fb.is_none() { *fb = Some(full.clone()); }
        if def { Some(full) } else { None }
    };

    for line in ini.lines().map(str::trim) {
        if line.starts_with('[') {
            if in_prof {
                if let Some(p) = commit(&cur_path, cur_rel, cur_def, &mut fallback) {
                    return Some(p);
                }
            }
            cur_path.clear(); cur_rel = false; cur_def = false;
            in_prof = line.starts_with("[Profile");
        } else if in_prof {
            if let Some(v) = line.strip_prefix("Path=") { cur_path = v.to_string(); }
            else if line == "IsRelative=1" { cur_rel = true; }
            else if line == "Default=1"   { cur_def = true; }
        }
    }
    if in_prof {
        if let Some(p) = commit(&cur_path, cur_rel, cur_def, &mut fallback) {
            return Some(p);
        }
    }
    fallback.or_else(|| {
        std::fs::read_dir(ff_dir).ok()?.flatten()
            .filter(|e| e.path().is_dir() && e.path().join("cookies.sqlite").exists())
            .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
            .map(|e| e.path())
    })
}

#[cfg(target_os = "linux")]
fn find_firefox_profile() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let home = std::path::Path::new(&home);
    // Native → snap → flatpak
    for base in [
        home.join(".mozilla/firefox"),
        home.join("snap/firefox/common/.mozilla/firefox"),
        home.join(".var/app/org.mozilla.firefox/.mozilla/firefox"),
    ] {
        if base.is_dir() {
            if let Some(p) = find_profile_in(&base) { return Some(p); }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn read_firefox_session() -> Result<(String, String), String> {
    let profile = find_firefox_profile().ok_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("Firefox profile not found. Checked:\n  {home}/.mozilla/firefox\n  {home}/snap/firefox/...\n  {home}/.var/app/org.mozilla.firefox/...")
    })?;

    // Firefox holds a write lock on cookies.sqlite while running.
    // Copy the file (+ WAL/SHM) to a private temp path so we can read without contention.
    let pid     = std::process::id();
    let tmp     = std::env::temp_dir().join(format!("ew_cookies_{pid}.sqlite"));
    let tmp_wal = std::env::temp_dir().join(format!("ew_cookies_{pid}.sqlite-wal"));
    let tmp_shm = std::env::temp_dir().join(format!("ew_cookies_{pid}.sqlite-shm"));

    struct Cleanup(Vec<std::path::PathBuf>);
    impl Drop for Cleanup {
        fn drop(&mut self) { for p in &self.0 { let _ = std::fs::remove_file(p); } }
    }
    let _cleanup = Cleanup(vec![tmp.clone(), tmp_wal.clone(), tmp_shm.clone()]);

    std::fs::copy(profile.join("cookies.sqlite"), &tmp)
        .map_err(|e| format!("Cannot copy cookies.sqlite: {e}"))?;
    for (ext, dst) in [("sqlite-wal", &tmp_wal), ("sqlite-shm", &tmp_shm)] {
        let s = profile.join(format!("cookies.{ext}"));
        if s.exists() { let _ = std::fs::copy(&s, dst); }
    }

    let conn = rusqlite::Connection::open_with_flags(
        &tmp,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ).map_err(|e| format!("Cannot open cookie database copy: {e}"))?;

    // Priority order:
    //   1. POETOKEN @ pathofexile.com  (current unified auth token, used by trade API)
    //   2. POESESSID @ pathofexile.com (legacy session, main site)
    //   3. POESESSID @ pathofexile2.com (PoE2 site session)
    if let Ok((n, v)) = conn.query_row(
        "SELECT name, value FROM moz_cookies \
         WHERE (host LIKE '%pathofexile.com' OR host LIKE '%pathofexile2.com') \
           AND name IN ('POETOKEN', 'POESESSID') \
         ORDER BY \
           CASE name WHEN 'POETOKEN' THEN 1 ELSE 2 END, \
           CASE WHEN host LIKE '%pathofexile.com' THEN 1 ELSE 2 END, \
           expiry DESC \
         LIMIT 1",
        [],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    ) {
        return Ok((n, v));
    }

    // Nothing matched — include diagnostic (names+hosts only, no values)
    let found: Vec<String> = conn.prepare(
        "SELECT name, host FROM moz_cookies WHERE host LIKE '%pathofexile%'"
    ).ok()
     .and_then(|mut s| s.query_map([], |r| {
         Ok(format!("{}@{}", r.get::<_,String>(0)?, r.get::<_,String>(1)?))
     }).ok().map(|rows| rows.flatten().collect()))
     .unwrap_or_default();

    if found.is_empty() {
        Err("No pathofexile.com cookies found — log in at pathofexile.com in Firefox first".to_string())
    } else {
        Err(format!("No usable token found. Cookies present: {}", found.join(", ")))
    }
}

#[derive(serde::Deserialize)]
struct StatFilter {
    stat_id: String,
    min: Option<f64>,
    max: Option<f64>,
}

#[derive(serde::Serialize)]
struct TradeResult {
    price: f64,
    currency: String,
    seller: String,
    status: String,
}

#[derive(serde::Serialize)]
struct TradeSearchResponse {
    results: Vec<TradeResult>,
    trade_url: String,
}

#[tauri::command]
async fn trade_search(
    league: String,
    game_mode: String,
    currency: String,
    item_name: String,       // non-empty only for unique items
    item_type: String,       // non-empty when user enables base-type filter
    corrupted: Option<bool>, // Some(true/false) to filter, None for any
    filters: Vec<StatFilter>,
    base_filters: Vec<StatFilter>, // stat_id is "dps"/"ar"/"ilvl" etc.
    session: tauri::State<'_, PoeSession>,
) -> Result<TradeSearchResponse, String> {
    let (cookie_name, cookie_value) = session.0.lock().unwrap().clone();

    let (base, site_base) = if game_mode == "poe2" {
        ("https://www.pathofexile.com/api/trade2", "https://www.pathofexile.com/trade2")
    } else {
        ("https://www.pathofexile.com/api/trade", "https://www.pathofexile.com/trade")
    };

    let client = reqwest::Client::builder()
        .user_agent("ExileWatch/0.1")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let auth_header = if cookie_value.is_empty() {
        None
    } else {
        Some(format!("{cookie_name}={cookie_value}"))
    };

    // Build stat filters — skip any mod without a resolved stat_id
    let stat_filters: Vec<serde_json::Value> = filters.iter()
        .filter(|f| !f.stat_id.is_empty())
        .map(|f| {
            let mut entry = serde_json::json!({ "id": f.stat_id });
            let mut val = serde_json::Map::new();
            if let Some(min) = f.min { val.insert("min".into(), serde_json::json!(min)); }
            if let Some(max) = f.max { val.insert("max".into(), serde_json::json!(max)); }
            if !val.is_empty() { entry["value"] = serde_json::Value::Object(val); }
            entry
        })
        .collect();

    // Route base_filters into weapon_filters / armour_filters / misc_filters
    const WEAPON_IDS: &[&str] = &["dps", "pdps", "edps", "crit", "aps"];
    const ARMOUR_IDS: &[&str] = &["ar", "ev", "es", "ward", "block"];

    let mut weapon_block = serde_json::Map::new();
    let mut armour_block = serde_json::Map::new();
    let mut ilvl_min: Option<f64> = None;

    for f in &base_filters {
        let id = f.stat_id.as_str();
        if id == "ilvl" {
            ilvl_min = f.min;
            continue;
        }
        let mut val = serde_json::Map::new();
        if let Some(min) = f.min { val.insert("min".into(), serde_json::json!(min)); }
        if let Some(max) = f.max { val.insert("max".into(), serde_json::json!(max)); }
        if val.is_empty() { continue; }
        if WEAPON_IDS.contains(&id) {
            weapon_block.insert(id.to_string(), serde_json::Value::Object(val));
        } else if ARMOUR_IDS.contains(&id) {
            armour_block.insert(id.to_string(), serde_json::Value::Object(val));
        }
    }

    let mut query_filters = serde_json::json!({
        "trade_filters": {
            "filters": { "price": { "option": currency } }
        }
    });
    if game_mode == "poe2" {
        // PoE2 API unifies weapon + defence stats under a single "equipment_filters" group
        let mut equip_block = weapon_block.clone();
        equip_block.extend(armour_block.clone());
        if !equip_block.is_empty() {
            query_filters["equipment_filters"] = serde_json::json!({ "filters": equip_block });
        }
    } else {
        if !weapon_block.is_empty() {
            query_filters["weapon_filters"] = serde_json::json!({ "filters": weapon_block });
        }
        if !armour_block.is_empty() {
            query_filters["armour_filters"] = serde_json::json!({ "filters": armour_block });
        }
    }
    {
        let mut misc = serde_json::Map::new();
        if let Some(min) = ilvl_min {
            misc.insert("ilvl".into(), serde_json::json!({ "min": min }));
        }
        if let Some(corr) = corrupted {
            misc.insert("corrupted".into(), serde_json::json!({
                "option": if corr { "true" } else { "false" }
            }));
        }
        if !misc.is_empty() {
            query_filters["misc_filters"] = serde_json::json!({ "filters": misc });
        }
    }

    let mut query = serde_json::json!({
        "filters": query_filters,
        "stats": [{ "type": "and", "filters": stat_filters }]
    });
    if !item_name.is_empty() {
        query["name"] = serde_json::json!(item_name);
    }
    if !item_type.is_empty() {
        query["type"] = serde_json::json!(item_type);
    }

    let body = serde_json::json!({ "query": query, "sort": { "price": "asc" } });
    let body_str = body.to_string();
    eprintln!("[EW] trade body: {}", &body_str[..body_str.len().min(1000)]);

    // POST search
    #[derive(serde::Deserialize)]
    struct SearchResp { result: Vec<String>, id: String }

    async fn check_response(resp: reqwest::Response) -> Result<reqwest::Response, String> {
        let status = resp.status();
        if status.is_success() { return Ok(resp); }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let secs = resp.headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(60);
            return Err(format!("Rate limited — try again in {secs}s"));
        }
        let err_body = resp.text().await.unwrap_or_default();
        eprintln!("[EW] API error {}: {}", status, &err_body[..err_body.len().min(500)]);
        Err(format!("API error (HTTP {}) — {}", status.as_u16(), &err_body[..err_body.len().min(300)]))
    }

    let mut req = client.post(format!("{base}/search/{league}"))
        .header("Content-Type", "application/json");
    if let Some(ref h) = auth_header { req = req.header("Cookie", h); }

    let search_resp = check_response(
        req.body(body_str).send().await.map_err(|e| format!("search request: {e}"))?
    ).await?;
    let search: SearchResp = search_resp.json().await
        .map_err(|e| format!("search parse: {e}"))?;

    let trade_url = format!("{site_base}/search/{league}/{}", search.id);

    if search.result.is_empty() {
        return Ok(TradeSearchResponse { results: vec![], trade_url });
    }

    // GET fetch (first 10 results)
    let ids = search.result[..search.result.len().min(10)].join(",");
    let fetch_url = format!("{base}/fetch/{ids}?query={}", search.id);

    #[derive(serde::Deserialize)]
    struct Price { amount: f64, currency: String }
    #[derive(serde::Deserialize)]
    struct Account { name: String, online: Option<serde_json::Value> }
    #[derive(serde::Deserialize)]
    struct Listing { price: Option<Price>, account: Account }
    #[derive(serde::Deserialize)]
    struct FetchEntry { listing: Listing }
    #[derive(serde::Deserialize)]
    struct FetchResp { result: Vec<Option<FetchEntry>> }

    let mut req = client.get(&fetch_url);
    if let Some(ref h) = auth_header { req = req.header("Cookie", h); }

    let fetch_text = check_response(
        req.send().await.map_err(|e| format!("fetch request: {e}"))?
    ).await?.text().await.map_err(|e| format!("fetch body: {e}"))?;
    let fetched: FetchResp = serde_json::from_str(&fetch_text)
        .map_err(|e| format!("fetch parse: {e}\nbody: {}", &fetch_text[..fetch_text.len().min(500)]))?;

    let results = fetched.result.into_iter().flatten()
        .filter_map(|entry| {
            let price = entry.listing.price?;
            // online can be: null → offline, false → offline, {status:"afk"} → afk, {status:"online"} → online
            let status = match entry.listing.account.online.as_ref() {
                Some(serde_json::Value::Object(obj))
                    if obj.get("status").and_then(|s| s.as_str()) == Some("afk") => "afk",
                Some(serde_json::Value::Object(_)) => "online",
                _ => "offline",
            }.to_string();
            Some(TradeResult {
                price: price.amount,
                currency: price.currency,
                seller: entry.listing.account.name,
                status,
            })
        })
        .collect();

    Ok(TradeSearchResponse { results, trade_url })
}

#[tauri::command]
async fn read_browser_session(
    state: tauri::State<'_, PoeSession>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        let (name, value) = tauri::async_runtime::spawn_blocking(read_firefox_session)
            .await
            .map_err(|e| e.to_string())??;
        *state.0.lock().unwrap() = (name.clone(), value.clone());
        save_session_config(&app, &name, &value);
        Ok(value)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("Browser auto-detection is not yet supported on this platform".to_string())
    }
}

fn load_saved_config(app: &tauri::App) -> (String, String) {
    let Ok(dir) = app.path().app_data_dir() else { return Default::default() };

    // Read cookie_name (not secret) from config.json
    let cookie_name = std::fs::read_to_string(dir.join("config.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v["cookie_name"].as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "POESESSID".to_string());

    // Try keyring first
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        if let Ok(val) = entry.get_password() {
            if !val.is_empty() {
                return (cookie_name, val);
            }
        }
    }

    // Migrate: if config.json still has a plaintext value, move it to keyring
    let migrated = std::fs::read_to_string(dir.join("config.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| {
            // Support both new (cookie_value) and legacy (poesessid) key names
            let name = v["cookie_name"].as_str().map(|s| s.to_string())
                .unwrap_or_else(|| "POESESSID".to_string());
            let val = v["cookie_value"].as_str()
                .or_else(|| v["poesessid"].as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            val.map(|v| (name, v))
        });

    if let Some((name, val)) = migrated {
        // Migrate plaintext value into keyring and scrub from disk
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
            if entry.set_password(&val).is_ok() {
                let n = serde_json::to_string(&name).unwrap_or_default();
                let _ = std::fs::write(dir.join("config.json"), format!("{{\"cookie_name\":{n}}}"));
                eprintln!("[EW] migrated session token from config.json to keyring");
            }
        }
        return (name, val);
    }

    Default::default()
}

// Used only on non-Linux (global shortcut path)
#[cfg(not(target_os = "linux"))]
fn toggle_overlay(handle: &tauri::AppHandle) {
    let Some(window) = handle.get_webview_window("main") else { return };
    let _ = window.clone().run_on_main_thread(move || {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.eval(
                "var b=document.body;b.style.display='none';b.offsetHeight;\
                 b.style.display='';document.documentElement.style.background='#0a0a0d';\
                 b.style.background='#0a0a0d';"
            );
            let _ = window.emit("overlay-shown", ());
        }
    });
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    let ctrl_c_dev = {
        let dev = create_ctrl_c_device();
        if dev.is_none() {
            eprintln!("[ExileWatch] WARNING: uinput device creation failed — Ctrl+C injection disabled. Make sure you are in the 'input' group.");
        } else {
            eprintln!("[ExileWatch] uinput Ctrl+C device created");
        }
        CtrlCDevice(Mutex::new(dev))
    };

    let builder = tauri::Builder::default()
        .manage(OverlayPos(Mutex::new((0, 0))));

    #[cfg(target_os = "linux")]
    let builder = builder.manage(ctrl_c_dev);

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let saved = load_saved_position(app);
            let (sname, svalue) = load_saved_config(app);
            app.manage(PoeSession(Mutex::new((sname, svalue))));
            app.manage(StatCache(Mutex::new(std::collections::HashMap::new())));

            if let Some(window) = app.get_webview_window("main") {
                #[cfg(target_os = "linux")]
                {
                    let (x, y) = init_layer_shell(&window, saved.0, saved.1);
                    *app.state::<OverlayPos>().0.lock().unwrap() = (x, y);
                }
                #[cfg(not(target_os = "linux"))]
                let _ = window.set_always_on_top(true);
            }

            #[cfg(target_os = "linux")]
            start_evdev_listener(app.handle().clone());

            #[cfg(not(target_os = "linux"))]
            {
                use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
                let handle = app.handle().clone();
                let _ = app.handle().global_shortcut().on_shortcut(
                    "Alt+D",
                    move |_app, _shortcut, event| {
                        if event.state() == ShortcutState::Pressed {
                            toggle_overlay(&handle);
                        }
                    },
                );
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            hide_overlay,
            set_overlay_keyboard,
            move_overlay,
            save_overlay_position,
            get_session_id,
            set_session_id,
            read_browser_session,
            fetch_leagues,
            trade_search,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
