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
}

#[derive(serde::Serialize, Clone)]
struct ParsedItem {
    name: String,
    base_type: String,
    rarity: String,
    item_level: u32,
    influence: String,
    game_mode: String,
    mods: Vec<ParsedMod>,
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
fn hide_overlay(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
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

fn is_adds_range_mod(line: &str) -> bool {
    // "Adds 14 to 28 Lightning Damage" — the token after "to" is a bare number.
    // "+14 to Strength" — the token after "to" is a word, so NOT a range mod.
    // This prevents misidentifying PoE2 mods like "+14(10-15) to Strength" as ranges.
    let Some(pos) = line.find(" to ") else { return false };
    let after = line[pos + 4..].trim_start();
    after.starts_with(|c: char| c.is_ascii_digit())
}

fn parse_mod_line(line: &str) -> Option<ParsedMod> {
    let nums = collect_numbers(line);
    if nums.is_empty() {
        return None;
    }
    // Two-value range mods like "Adds 14 to 28 Lightning Damage" → average.
    // Single-value mods like "+14(10-15) to Strength" → first number (the roll).
    let value = if nums.len() >= 2 && is_adds_range_mod(line) {
        (nums[0].abs() + nums[1].abs()) / 2.0
    } else {
        nums[0].abs()
    };
    Some(ParsedMod { text: numbers_to_hash(line), value })
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

    // Sections after "Item Level:" section contain mods
    let il_idx = sections.iter().position(|s| s.contains("Item Level: "))?;
    let mut mods = Vec::new();
    for section in &sections[il_idx + 1..] {
        let trimmed = section.trim();
        if trimmed.is_empty() { continue; }
        // Skip sections that are entirely skip-tags
        if trimmed.lines().all(|l| {
            let l = l.trim();
            l.is_empty() || SKIP_TAGS.iter().any(|&t| l == t)
        }) { continue; }
        for line in trimmed.lines() {
            let line = line.trim();
            if line.is_empty() { continue; }
            if line.starts_with('(') && line.ends_with(')') { continue; }
            if line.starts_with('{') { continue; } // PoE2 modifier descriptor lines
            if SKIP_TAGS.iter().any(|&t| line == t) { continue; }
            if let Some(m) = parse_mod_line(line) {
                mods.push(m);
            }
        }
    }

    if mods.is_empty() { return None; }

    Some(ParsedItem {
        name, base_type,
        rarity: rarity_key.to_string(),
        item_level, influence,
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
    //    Showing the overlay first lets WebKit briefly grab focus and intercept Ctrl+C
    //    (copying the page URL into clipboard instead of PoE2 copying the item).
    eprintln!("[EW {}] injecting Ctrl+C (overlay hidden)", ts());
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
                Ok(_) => eprintln!("[EW {}] Ctrl+C emitted", ts()),
                Err(e) => eprintln!("[EW {}] Ctrl+C emit error: {e}", ts()),
            }
        } else {
            eprintln!("[EW {}] no uinput device — skipping Ctrl+C injection", ts());
        }
    }

    // 3. Now show the overlay in loading state — Ctrl+C is already in flight.
    eprintln!("[EW {}] showing overlay", ts());
    let win = window.clone();
    let _ = window.clone().run_on_main_thread(move || {
        let _ = win.show();
        let _ = win.emit("overlay-shown", ());
        let _ = win.emit("search-started", ());
    });

    // 4. Poll wl-paste for PoE item content.
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
            move_overlay,
            save_overlay_position,
            get_session_id,
            set_session_id,
            read_browser_session,
            fetch_leagues,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
