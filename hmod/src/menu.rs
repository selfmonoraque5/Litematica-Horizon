//! The Blazematica menu (M): the mod's own screen (`ClientHost::mod_screen_open`), drawn with
//! the widgets of `ui.rs` in `screen_render_post`. The game keeps running behind it.
//!
//! Tabs: Load, Placement, Save, Tool, Printer, Display, Materials, and the credits. The keys are
//! the game's key mappings now (Options > Controls), so there is no Keys tab any more.

use crate::i18n::{item_name, on_off};
use crate::runtime::{self, display_name};
use crate::settings::{self, Kind};
use crate::store;
use crate::ui::{Rect, Ui};
use crate::world::World;
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// What the Load tab shows of a file (read when it is picked).
#[derive(Clone, Debug)]
struct FileInfo {
    name: String,
    format: String,
    size: [i32; 3],
    blocks: u64,
    regions: usize,
    author: String,
}

/// What the menu keeps between frames.
#[derive(Default)]
struct Screen {
    /// The menu is to open (or close) at the next tick (that needs the game's mutable host).
    open: bool,
    close: bool,
    tab: usize,
    /// The schematics folder's files with their sizes, and when they were listed.
    files: Vec<(String, u64)>,
    files_at: Option<Instant>,
    /// The picked file, and what is known of files (None while a thread reads one).
    file: Option<String>,
    info: HashMap<String, Option<Result<FileInfo, String>>>,
    info_chan: Option<(std::sync::mpsc::Sender<(String, Result<FileInfo, String>)>, std::sync::mpsc::Receiver<(String, Result<FileInfo, String>)>)>,
    /// What the origin and corner fields were last set to (they follow the placement unless
    /// being typed in).
    origin: Option<[i32; 3]>,
    corners: [Option<[i32; 3]>; 2],
    /// The file a save would overwrite, once asked.
    overwrite: Option<String>,
}

thread_local! {
    static SCREEN: RefCell<Screen> = RefCell::new(Screen::default());
}

fn sc<R>(f: impl FnOnce(&mut Screen) -> R) -> R {
    SCREEN.with_borrow_mut(f)
}

/// Opens the menu (at the next tick). Its edit boxes are new, so they take the origin and the
/// corners again.
pub fn open(_w: &World<'_>) {
    sc(|s| {
        s.open = true;
        s.origin = None;
        s.corners = [None; 2];
    });
}

/// Whether the menu is to open now (taken).
pub fn take_open() -> bool {
    sc(|s| std::mem::take(&mut s.open))
}

/// Whether the menu is to close now (taken).
pub fn take_close() -> bool {
    sc(|s| std::mem::take(&mut s.close))
}

/// Runs a command for the screen; its first line (or the error) shows at the bottom.
fn act(w: &World<'_>, args: &[&str]) {
    let msg = match runtime::run(w, args) {
        Ok(lines) => lines.into_iter().next().unwrap_or_default(),
        Err(e) => tr!("error", e),
    };
    runtime::with(|s| s.message = msg);
}

/// A key's or a menu row's action; what it says goes to the chat.
fn action(w: &World<'_>, id: &str) {
    runtime::action(w, id);
}

fn file_info(dir: &std::path::Path, name: &str) -> Result<FileInfo, String> {
    let bytes = blazematica_core::nbt::read_input(&dir.join(name))?;
    let s = blazematica_core::formats::summary(&bytes)?;
    Ok(FileInfo {
        name: if s.name.is_empty() { name.to_string() } else { s.name },
        format: runtime::format_text(s.format),
        size: s.size,
        blocks: s.blocks,
        regions: s.regions,
        author: s.author,
    })
}

/// What is known of a file; asked for the first time, a thread starts reading it (`None` until
/// it is in).
fn file_info_cached(dir: &std::path::Path, name: &str, key: &str) -> Option<Result<FileInfo, String>> {
    let known = sc(|s| {
        if let Some((_, rx)) = &s.info_chan {
            while let Ok((k, info)) = rx.try_recv() {
                s.info.insert(k, Some(info));
            }
        }
        s.info.get(key).cloned()
    });
    if let Some(info) = known {
        return info;
    }
    let tx = sc(|s| {
        s.info.insert(key.to_string(), None);
        s.info_chan.get_or_insert_with(std::sync::mpsc::channel).0.clone()
    });
    let (name, key, dir) = (name.to_string(), key.to_string(), dir.to_path_buf());
    std::thread::spawn(move || {
        let _ = tx.send((key, file_info(&dir, &name)));
    });
    None
}

// ---------------------------------------------------------------- the settings grid

/// The settings of each part of the menu (registered settings, `settings::SPECS`).
const DISPLAY: &[&str] = &[
    "render", "through", "mode", "alpha", "radius_view", "hide_fluids", "placement_boxes", "show_extra", "show_wrong",
    "highlight_missing", "highlight_wrong", "strict", "power",
];
const HUD: &[&str] = &["hud", "materials_hud"];
const PRINTER: &[&str] = &["printer_interval", "printer_reach", "air_place"];
const EASY_PLACE: &[&str] = &["easy_place", "easy_interval", "easy_air"];
const SELECTION: &[&str] = &["tool", "tool_mode", "tool_range", "tool_target", "tool_fluids", "tool_hud"];

fn number_text(v: f64, step: f64) -> String {
    if step >= 1.0 { format!("{v:.0}") } else if step >= 0.1 { format!("{v:.1}") } else { format!("{v:.2}") }
}

/// The settings `keys` in `columns` columns of the game's widgets (toggles and choices as
/// buttons, numbers as sliders) from the top of `r`. Returns the height used.
fn settings_grid(ui: &mut Ui<'_>, w: &World<'_>, keys: &[&str], r: Rect, columns: usize) -> f32 {
    let columns = columns.max(1);
    let gap = 4.0;
    let cw = ((r[2] - gap * (columns as f32 - 1.0)) / columns as f32).floor();
    let mut changed = false;
    let mut y = r[1];
    for (i, key) in keys.iter().enumerate() {
        let Some(&(_, _, kind)) = settings::SPECS.iter().find(|(k, ..)| k == key) else { continue };
        let label = tr!(&format!("setting.{key}"));
        let col = i % columns;
        if i > 0 && col == 0 {
            y += 24.0;
        }
        let x = r[0] + col as f32 * (cw + gap);
        let b = [x, y, cw, 20.0];
        match kind {
            Kind::Toggle(_) => {
                let on = settings::get_bool(w, key);
                if ui.button(b, &format!("{label}: {}", on_off(on)), true) {
                    settings::set(w, key, if on { "false" } else { "true" });
                    changed = true;
                }
            }
            Kind::Choice(options, _) => {
                let now = settings::get_choice(w, key);
                if ui.button(b, &format!("{label}: {}", tr!(&format!("setting.{key}.{now}"))), true) {
                    let i = options.iter().position(|o| *o == now).unwrap_or(0);
                    let n = options.len();
                    let next = if ui.shift { (i + n - 1) % n } else { (i + 1) % n };
                    settings::set(w, key, options[next]);
                    changed = true;
                }
            }
            Kind::Range(min, max, step, _) => {
                let v = settings::get_number(w, key);
                let fraction = ((v - min) / (max - min)) as f32;
                if let Some(f) = ui.slider(key, b, &format!("{label}: {}", number_text(v, step)), fraction) {
                    let raw = min + f64::from(f) * (max - min);
                    let snapped = ((raw - min) / step).round() * step + min;
                    let snapped = snapped.clamp(min, max);
                    if (snapped - v).abs() > 1e-9 {
                        settings::set(w, key, &snapped.to_string());
                        changed = true;
                    }
                }
            }
        }
    }
    if changed {
        runtime::settings_changed();
        runtime::ensure_settings(w);
    }
    if keys.is_empty() { 0.0 } else { y + 24.0 - r[1] }
}

// ---------------------------------------------------------------- the screen

/// The tabs, by their lang keys (`menu.tab.<name>`).
const TABS: [&str; 7] = ["load", "placement", "save", "tool", "printer", "display", "materials"];
const CREDITS_TAB: usize = TABS.len();

/// Draws the menu (the `screen_render_post` hook, while it is open). Whether it asked to close.
pub fn draw(w: &World<'_>, mut ui: Ui<'_>) {
    runtime::ensure_settings(w);
    let width = (ui.width - 20.0).min(500.0).floor();
    let x0 = ((ui.width - width) / 2.0).floor();
    let loaded = runtime::with(|s| {
        s.placement().map(|p| if s.slots.len() > 1 { format!("{} ({}/{})", display_name(p), s.selected + 1, s.slots.len()) } else { display_name(p) })
    });
    let cx = ui.width / 2.0;
    ui.text_centered(cx, 7.0, "Blazematica", 0xFFFFFF);
    if let Some(name) = &loaded {
        let shown = ui.fit(name, width / 2.0 - 50.0);
        ui.text_right(x0 + width, 7.0, &shown, 0x55FFFF);
    }
    let tab = sc(|s| s.tab).min(CREDITS_TAB);
    let labels: Vec<String> = TABS.iter().map(|t| tr!(&format!("menu.tab.{t}"))).collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    let mut next = ui.tabs([x0, 20.0, width, 20.0], &labels, tab);
    // 1-7 switch tabs, M closes (while no edit box has the keys).
    for i in 0..TABS.len() {
        if ui.key(&format!("key.keyboard.{}", i + 1)) {
            next = Some(i);
        }
    }
    if ui.key("key.keyboard.m") {
        ui.close();
    }
    if let Some(t) = next {
        sc(|s| s.tab = t);
    }
    let body = [x0, 46.0, width, ui.height - 46.0 - 30.0];
    match tab {
        0 => tab_load(w, &mut ui, body),
        1 => tab_placement(w, &mut ui, body),
        2 => tab_save(w, &mut ui, body),
        3 => tab_tool(w, &mut ui, body),
        4 => tab_printer(w, &mut ui, body),
        5 => tab_display(w, &mut ui, body),
        6 => tab_materials(w, &mut ui, body),
        _ => tab_credits(w, &mut ui, body),
    }
    let fy = ui.height - 25.0;
    let msg = runtime::with(|s| s.message.clone());
    if !msg.is_empty() {
        ui.text_fit(x0, fy + 6.0, width - 110.0, &msg, 0xFFFF55);
    }
    let credits = tr!("menu.credits");
    let cw = (ui.text_width(&credits) + 12.0).max(50.0);
    if ui.button([x0, 2.0, cw, 16.0], &credits, tab != CREDITS_TAB) {
        sc(|s| s.tab = CREDITS_TAB);
    }
    if ui.button([x0 + width - 100.0, fy, 100.0, 20.0], &tr!("menu.done"), true) {
        ui.close();
    }
    if ui.end() {
        sc(|s| s.close = true);
    }
}

fn tab_load(w: &World<'_>, ui: &mut Ui<'_>, r: Rect) {
    let dir = store::schematics_dir(w);
    // The folder is listed again every few seconds while the tab is open.
    sc(|s| {
        if s.files_at.is_none_or(|t| t.elapsed() > Duration::from_secs(3)) {
            s.files_at = Some(Instant::now());
            s.files = dir
                .as_ref()
                .map(|dir| {
                    blazematica_core::list_schematics(dir)
                        .into_iter()
                        .map(|n| {
                            let size = std::fs::metadata(dir.join(&n)).map_or(0, |m| m.len());
                            (n, size)
                        })
                        .collect()
                })
                .unwrap_or_default();
        }
    });
    let lw = (r[2] * 0.55).floor();
    let query = ui.edit("search", [r[0] + 1.0, r[1] + 1.0, lw - 2.0, 18.0], &tr!("menu.load.search"), 64).to_lowercase();
    let files: Vec<(String, u64)> = sc(|s| s.files.iter().filter(|(n, _)| n.to_lowercase().contains(&query)).cloned().collect());
    let total = sc(|s| s.files.len());
    let list = [r[0], r[1] + 24.0, lw, r[3] - 24.0];
    ui.fill(list, 0x9000_0000);
    ui.frame(list, 0xFF50_5050);
    let row_h = 14.0;
    let content = files.len() as f32 * row_h + 4.0;
    let y0 = ui.scroll_begin("files", list, content);
    let selected = sc(|s| s.file.clone());
    let mut picked = None;
    for (i, (name, size)) in files.iter().enumerate() {
        let y = y0 + 2.0 + i as f32 * row_h;
        if y + row_h < list[1] || y > list[1] + list[3] {
            continue;
        }
        let row = [list[0] + 2.0, y, list[2] - 10.0, row_h];
        let is_selected = selected.as_deref() == Some(name.as_str());
        if is_selected {
            ui.fill(row, 0x50FF_FFFF);
        } else if ui.hovered(row) {
            ui.fill(row, 0x25FF_FFFF);
        }
        let size = runtime::human_size(*size);
        let sw = ui.text_width(&size);
        ui.text_fit(row[0] + 3.0, y + 3.0, row[2] - sw - 12.0, name, if is_selected { 0xFFFFFF } else { 0xD0D0D0 });
        ui.text_right(row[0] + row[2] - 3.0, y + 3.0, &size, 0x909090);
        if ui.clicked(row) {
            picked = Some(name.clone());
        }
    }
    ui.scroll_end("files", list, content);
    if files.is_empty() {
        let text = if total == 0 { tr!("menu.load.no_files") } else { tr!("menu.load.no_match") };
        ui.text_fit(list[0] + 5.0, list[1] + 5.0, list[2] - 10.0, &text, 0x909090);
    }
    if let Some(name) = picked {
        // A second click on the picked file loads it.
        if selected.as_deref() == Some(name.as_str()) {
            act(w, &["load", &name]);
            sc(|s| s.tab = 1);
            return;
        }
        sc(|s| s.file = Some(name));
    }

    // The picked file: what it is, and loading it.
    let rx = r[0] + lw + 8.0;
    let rw = r[2] - lw - 8.0;
    let selected = sc(|s| s.file.clone().filter(|n| s.files.iter().any(|(f, _)| f == n)));
    let mut y = r[1] + 2.0;
    match (&selected, &dir) {
        (Some(name), Ok(dir)) => {
            let size = sc(|s| s.files.iter().find(|(f, _)| f == name).map_or(0, |(_, b)| *b));
            let key = format!("{name}|{size}");
            match file_info_cached(dir, name, &key) {
                None => {
                    ui.text_wrapped(rx, y, rw, &tr!("menu.load.reading"), 0xA0A0A0);
                }
                Some(Ok(i)) => {
                    y += ui.text_wrapped(rx, y, rw, &i.name, 0xFFFFFF) + 2.0;
                    let lines = [
                        (format!("{} x {} x {}", i.size[0], i.size[1], i.size[2]), 0xE0E0E0),
                        (tr!("menu.load.blocks", i.blocks), 0xE0E0E0),
                        (tr!("menu.load.regions", i.regions), 0xA0A0A0),
                        (i.format.clone(), 0xA0A0A0),
                        (if i.author.is_empty() { String::new() } else { tr!("menu.load.by", i.author) }, 0xA0A0A0),
                    ];
                    for (text, rgb) in lines {
                        if !text.is_empty() {
                            ui.text_fit(rx, y, rw, &text, rgb);
                            y += 11.0;
                        }
                    }
                }
                Some(Err(e)) => {
                    ui.text_wrapped(rx, y, rw, &tr!("menu.load.unreadable", e), 0xFF5555);
                }
            }
            let by = r[1] + r[3] - 68.0;
            if ui.button_tip([rx, by, rw, 20.0], &tr!("menu.load.load_here"), true, &[&tr!("menu.load.load_here_tip")]) {
                act(w, &["load", name]);
                sc(|s| s.tab = 1);
            }
        }
        _ => {
            ui.text_wrapped(rx, y, rw, &tr!("menu.load.pick"), 0xA0A0A0);
        }
    }
    let by = r[1] + r[3] - 44.0;
    let half = ((rw - 4.0) / 2.0).floor();
    if ui.button_tip([rx, by, half, 20.0], &tr!("menu.load.reload"), true, &[&tr!("menu.load.reload_tip")]) {
        act(w, &["reload"]);
    }
    if ui.button([rx + half + 4.0, by, rw - half - 4.0, 20.0], &tr!("menu.open_folder"), true) {
        action(w, "folder");
    }
    if ui.button([rx, by + 24.0, rw, 20.0], &tr!("menu.load.refresh"), true) {
        sc(|s| {
            s.files_at = None;
            s.info.clear();
        });
    }
}

fn tab_placement(w: &World<'_>, ui: &mut Ui<'_>, r: Rect) {
    // Every placement: name, origin, hidden or locked.
    let rows: Vec<(String, [i32; 3], bool, bool)> =
        runtime::with(|s| s.slots.iter().map(|x| (display_name(&x.placement), x.placement.origin, x.shown, x.locked)).collect());
    if rows.is_empty() {
        ui.text(r[0], r[1] + 4.0, &tr!("menu.pl.none"), 0xA0A0A0);
        if ui.button([r[0], r[1] + 18.0, 150.0, 20.0], &tr!("menu.pl.load_one"), true) {
            sc(|s| s.tab = 0);
        }
        return;
    }
    let selected = runtime::with(|s| s.selected.min(s.slots.len() - 1));

    // Left: the list; a click selects.
    let lw = (r[2] * 0.38).floor();
    let list = [r[0], r[1], lw, r[3] - 96.0];
    ui.fill(list, 0x9000_0000);
    ui.frame(list, 0xFF50_5050);
    let row_h = 26.0;
    let content = rows.len() as f32 * row_h + 4.0;
    let y0 = ui.scroll_begin("placements", list, content);
    let mut picked = None;
    for (i, (name, origin, shown, locked)) in rows.iter().enumerate() {
        let y = y0 + 2.0 + i as f32 * row_h;
        if y + row_h < list[1] || y > list[1] + list[3] {
            continue;
        }
        let row = [list[0] + 2.0, y, list[2] - 10.0, row_h - 2.0];
        let is_selected = i == selected;
        if is_selected {
            ui.fill(row, 0x50FF_FFFF);
            ui.fill([row[0], row[1], 2.0, row[3]], if *locked { 0xFFFF_AA00 } else { 0xFF55_FFFF });
        } else if ui.hovered(row) {
            ui.fill(row, 0x25FF_FFFF);
        }
        let rgb = match (is_selected, *shown) {
            (true, _) => 0xFFFFFF,
            (false, true) => 0xD0D0D0,
            (false, false) => 0x808080,
        };
        ui.text_fit(row[0] + 6.0, y + 3.0, row[2] - 10.0, &format!("{}. {name}", i + 1), rgb);
        let mut detail = format!("{} {} {}", origin[0], origin[1], origin[2]);
        if !shown {
            detail += &tr!("menu.pl.hidden");
        }
        if *locked {
            detail += &tr!("menu.pl.locked");
        }
        ui.text_fit(row[0] + 6.0, y + 13.0, row[2] - 10.0, &detail, if *locked { 0xFFAA00 } else { 0x909090 });
        if ui.clicked(row) {
            picked = Some(i);
        }
    }
    ui.scroll_end("placements", list, content);
    if let Some(i) = picked
        && i != selected
    {
        act(w, &["select", &(i + 1).to_string()]);
    }
    let by = r[1] + r[3] - 20.0;
    settings_grid(ui, w, EASY_PLACE, [r[0], by - 72.0, lw, 0.0], 1);
    let half = ((lw - 4.0) / 2.0).floor();
    if ui.button_tip([r[0], by, half, 20.0], &tr!("menu.pl.load_more"), true, &[&tr!("menu.pl.load_more_tip")]) {
        sc(|s| s.tab = 0);
    }
    if ui.button_tip([r[0] + half + 4.0, by, lw - half - 4.0, 20.0], &tr!("menu.pl.unload_all"), true, &[&tr!("menu.pl.unload_all_tip")]) {
        act(w, &["unload", "all"]);
    }

    // Right: the selected placement.
    let info = runtime::with(|s| {
        let slot = s.sel()?;
        let p = &slot.placement;
        let (lo, hi) = p.bounds();
        Some((
            display_name(p),
            p.origin,
            [0, 1, 2].map(|i| hi[i] - lo[i] + 1),
            runtime::rotation_text(p.rotation),
            runtime::mirror_text(p.mirror),
            runtime::layer_text(s.layer),
            s.moving,
            slot.shown,
            slot.locked,
            s.tally().map(|t| (t.percent(), t.missing, t.wrong_block + t.wrong_state, t.extra)),
            s.settings.tool_placement,
        ))
    });
    let Some((name, origin, size, rotation, mirror, layers, moving, shown, locked, tally, tool_placement)) = info else { return };
    let x = r[0] + lw + 8.0;
    // Scrolled where the window is too low for it (at 720p).
    let area = [x, r[1], r[2] - lw - 8.0, r[3]];
    let content = 37.0 + 8.0 * 24.0 - 4.0;
    let wd = if content > area[3] { area[2] - 10.0 } else { area[2] };
    let free = !locked;
    let half = ((wd - 4.0) / 2.0).floor();
    let third = ((wd - 8.0) / 3.0).floor();
    let mut y = ui.scroll_begin("placement_info", area, content);
    ui.text_fit(x, y + 1.0, wd, &format!("{}. {name}", selected + 1), 0xFFFFFF);
    y += 12.0;
    ui.text_fit(x, y, wd, &tr!("menu.pl.size", size[0], size[1], size[2], rotation, mirror), 0xC0C0C0);
    y += 11.0;
    match tally {
        Some((pct, missing, wrong, extra)) => ui.text_fit(x, y, wd, &tr!("menu.pl.tally", format!("{pct:.1}"), missing, wrong, extra), 0x55FF55),
        None => ui.text_fit(x, y, wd, &tr!("menu.pl.checking"), 0xA0A0A0),
    }
    y += 14.0;

    // Shown, locked, gone.
    let label = if shown { tr!("menu.pl.hide") } else { tr!("menu.pl.show") };
    if ui.button_tip([x, y, third, 20.0], &label, true, &[&tr!("menu.pl.hide_tip")]) {
        act(w, &["show", "toggle"]);
    }
    let tip = [tr!("menu.pl.lock_tip_1"), tr!("menu.pl.lock_tip_2")];
    let tip: Vec<&str> = tip.iter().map(String::as_str).collect();
    let label = if locked { tr!("menu.pl.unlock") } else { tr!("menu.pl.lock") };
    if ui.button_tip([x + third + 4.0, y, third, 20.0], &label, true, &tip) {
        act(w, &["lock", "toggle"]);
    }
    if ui.button_tip([x + 2.0 * (third + 4.0), y, wd - 2.0 * (third + 4.0), 20.0], &tr!("menu.pl.unload"), true, &[&tr!("menu.pl.unload_tip")]) {
        act(w, &["unload"]);
    }
    y += 24.0;

    // Where it is.
    let origin_label = tr!("menu.pl.origin");
    let label_w = (ui.text_width(&origin_label) + 6.0).max(36.0);
    ui.text(x, y + 6.0, &origin_label, if free { 0xFFFFFF } else { 0x808080 });
    // The fields follow the origin unless one is being typed in.
    let typing = ui.focused("ox") || ui.focused("oy") || ui.focused("oz");
    if !typing && sc(|s| s.origin != Some(origin)) {
        ui.set_edit("ox", &origin[0].to_string());
        ui.set_edit("oy", &origin[1].to_string());
        ui.set_edit("oz", &origin[2].to_string());
        sc(|s| s.origin = Some(origin));
    }
    let set_label = tr!("menu.set");
    let set_w = (ui.text_width(&set_label) + 12.0).max(40.0);
    let fx = x + label_w;
    let fw = ((wd - label_w - set_w - 6.0) / 3.0).floor();
    let ox = ui.edit("ox", [fx, y + 1.0, fw - 2.0, 18.0], "x", 9);
    let oy = ui.edit("oy", [fx + fw, y + 1.0, fw - 2.0, 18.0], "y", 9);
    let oz = ui.edit("oz", [fx + 2.0 * fw, y + 1.0, fw - 2.0, 18.0], "z", 9);
    let set = ui.button([fx + 3.0 * fw + 2.0, y, set_w, 20.0], &set_label, free) || (typing && ui.key("key.keyboard.enter"));
    if set {
        match (ox.trim().parse::<i32>(), oy.trim().parse::<i32>(), oz.trim().parse::<i32>()) {
            (Ok(a), Ok(b), Ok(c)) => act(w, &["origin", &a.to_string(), &b.to_string(), &c.to_string()]),
            _ => runtime::with(|s| s.message = tr!("menu.pl.origin_numbers")),
        }
        sc(|s| s.origin = None);
    }
    y += 24.0;
    // Nudge buttons: 1 block, 10 with Shift.
    let step = if ui.shift { 10 } else { 1 };
    let bw = ((wd - 10.0) / 6.0).floor();
    let nudges: [(&str, [i32; 3]); 6] = [("-X", [-1, 0, 0]), ("+X", [1, 0, 0]), ("-Y", [0, -1, 0]), ("+Y", [0, 1, 0]), ("-Z", [0, 0, -1]), ("+Z", [0, 0, 1])];
    for (i, (label, d)) in nudges.into_iter().enumerate() {
        let tip = tr!("menu.pl.nudge_tip", step, label);
        if ui.button_tip([x + i as f32 * (bw + 2.0), y, bw, 20.0], label, free, &[&tip]) {
            let d = d.map(|v| (v * step).to_string());
            act(w, &["move", &d[0], &d[1], &d[2]]);
        }
    }
    y += 24.0;
    if ui.button([x, y, half, 20.0], &tr!("menu.pl.move_to_me"), free) {
        act(w, &["origin", "here"]);
    }
    let label = tr!("menu.pl.move_mode", on_off(moving));
    if ui.button_tip([x + half + 4.0, y, wd - half - 4.0, 20.0], &label, free, &[&tr!("menu.pl.move_mode_tip")]) {
        runtime::toggle_move_mode(w);
        if runtime::with(|s| s.moving) {
            ui.close();
        }
    }
    y += 24.0;

    // How it is turned.
    if ui.button([x, y, third, 20.0], &tr!("menu.pl.rotate_left"), free) {
        act(w, &["rotate", "ccw"]);
    }
    if ui.button([x + third + 4.0, y, third, 20.0], &tr!("menu.pl.rotate_right"), free) {
        act(w, &["rotate", "cw"]);
    }
    let tip = [tr!("menu.pl.mirror_tip")];
    let tip: Vec<&str> = tip.iter().map(String::as_str).collect();
    if ui.button_tip([x + 2.0 * (third + 4.0), y, wd - 2.0 * (third + 4.0), 20.0], &tr!("chat.mirror", mirror), free, &tip) {
        action(w, "mirror_next");
    }
    y += 24.0;

    // Layers (of every placement).
    if ui.button([x, y, wd, 20.0], &tr!("chat.layers", layers), true) {
        let next = runtime::next_layer_mode();
        act(w, &["layer", next]);
    }
    y += 24.0;
    if ui.button([x, y, third, 20.0], &tr!("menu.pl.layer_down"), true) {
        act(w, &["layer", "down"]);
    }
    if ui.button([x + third + 4.0, y, third, 20.0], &tr!("menu.pl.layer_up"), true) {
        act(w, &["layer", "up"]);
    }
    if ui.button([x + 2.0 * (third + 4.0), y, wd - 2.0 * (third + 4.0), 20.0], &tr!("menu.pl.all_layers"), true) {
        act(w, &["layer", "all"]);
    }
    y += 24.0;

    // The tool moves placements in its placement mode.
    let label = if tool_placement { tr!("menu.pl.tool_placement") } else { tr!("menu.pl.tool_area") };
    let tip = [tr!("menu.pl.tool_tip_1"), tr!("menu.pl.tool_tip_2"), tr!("menu.pl.tool_tip_3")];
    let tip: Vec<&str> = tip.iter().map(String::as_str).collect();
    if ui.button_tip([x, y, wd, 20.0], &label, true, &tip) {
        act(w, &["tool", "mode", "toggle"]);
    }
    ui.scroll_end("placement_info", area, content);
}

fn tab_save(w: &World<'_>, ui: &mut Ui<'_>, r: Rect) {
    let area = runtime::with(|s| s.area);
    let mut y = r[1];
    ui.text_fit(r[0], y + 2.0, r[2], &tr!("menu.save.title"), 0xFFFFFF);
    y += 16.0;
    let corner_labels = [tr!("menu.save.corner", 1), tr!("menu.save.corner", 2)];
    let name_label = tr!("menu.save.name");
    let label_w = corner_labels.iter().chain([&name_label]).map(|t| ui.text_width(t) + 6.0).fold(52.0, f32::max);
    let (set_label, look_label, here_label) = (tr!("menu.set"), tr!("menu.save.look"), tr!("menu.save.here"));
    let set_w = (ui.text_width(&set_label) + 12.0).max(40.0);
    let bw = (ui.text_width(&look_label).max(ui.text_width(&here_label)) + 12.0).max(48.0);
    let fw = ((r[2] - label_w - set_w - 2.0 * bw - 3.0 * 4.0) / 3.0).floor();
    for (i, corner) in area.into_iter().enumerate() {
        let ids = [0, 1, 2].map(|k| format!("c{}{}", i + 1, ["x", "y", "z"][k]));
        // The fields follow the corner unless one is being typed in.
        let typing = ids.iter().any(|id| ui.focused(id));
        if !typing && sc(|s| s.corners[i] != corner) {
            for (k, id) in ids.iter().enumerate() {
                ui.set_edit(id, &corner.map_or(String::new(), |c| c[k].to_string()));
            }
            sc(|s| s.corners[i] = corner);
        }
        let rgb = if i == 0 { 0xFF7070 } else { 0x7090FF };
        ui.text(r[0], y + 6.0, &corner_labels[i], rgb);
        let fx = r[0] + label_w;
        let v = [0, 1, 2].map(|k| ui.edit(&ids[k], [fx + k as f32 * fw, y + 1.0, fw - 2.0, 18.0], ["x", "y", "z"][k], 9));
        let mut x = fx + 3.0 * fw + 2.0;
        let cmd = if i == 0 { "pos1" } else { "pos2" };
        if ui.button([x, y, set_w, 20.0], &set_label, true) {
            match v.iter().map(|s| s.trim().parse::<i32>()).collect::<Result<Vec<_>, _>>() {
                Ok(c) => act(w, &[cmd, &c[0].to_string(), &c[1].to_string(), &c[2].to_string()]),
                Err(_) => runtime::with(|s| s.message = tr!("menu.save.corner_numbers", i + 1)),
            }
            sc(|s| s.corners[i] = None);
        }
        x += set_w + 4.0;
        if ui.button_tip([x, y, bw, 20.0], &look_label, true, &[&tr!("menu.save.look_tip")]) {
            act(w, &[cmd]);
        }
        x += bw + 4.0;
        if ui.button_tip([x, y, bw, 20.0], &here_label, true, &[&tr!("menu.save.here_tip")]) {
            act(w, &[cmd, "here"]);
        }
        y += 24.0;
    }
    let size = runtime::with(|s| s.area_size());
    let text = match size {
        Some(d) => tr!("menu.save.volume", d[0], d[1], d[2], runtime::volume_text(d)),
        None => tr!("menu.save.set_corners"),
    };
    ui.text(r[0], y + 2.0, &text, if size.is_some() { 0x55FF55 } else { 0xA0A0A0 });
    y += 16.0;
    ui.text(r[0], y + 6.0, &name_label, 0xFFFFFF);
    let name = ui.edit("save_name", [r[0] + label_w, y + 1.0, r[2] - label_w - 104.0, 18.0], "my_build", 80);
    let stem = runtime::file_stem(&name);
    let exists = stem.as_ref().is_ok_and(|s| store::schematics_dir(w).is_ok_and(|d| d.join(format!("{s}.litematic")).exists()));
    let confirm = sc(|s| s.overwrite.clone()).is_some_and(|n| stem.as_ref().is_ok_and(|s| *s == n));
    let label = if exists && confirm { tr!("menu.save.overwrite") } else { tr!("menu.save.save") };
    let tip = [tr!("menu.save.save_tip_1"), tr!("menu.save.save_tip_2")];
    let tip: Vec<&str> = tip.iter().map(String::as_str).collect();
    let progress = runtime::with(|s| runtime::save_lines(w, s));
    if let Some((lines, done)) = &progress {
        // While it runs: Cancel in Save's place, the progress above the note at the bottom.
        if ui.button_tip([r[0] + r[2] - 100.0, y, 100.0, 20.0], &tr!("menu.save.cancel"), runtime::with(|s| s.save.is_some()), &[&tr!("menu.save.cancel_tip")]) {
            act(w, &["save", "cancel"]);
        }
        let bar = [r[0], r[1] + r[3] - 80.0, r[2], 20.0];
        ui.fill(bar, 0xC000_0000);
        ui.fill([bar[0], bar[1], (bar[2] * done).floor(), bar[3]], 0xC0FF_8800);
        ui.frame(bar, 0xFF80_8080);
        if let Some((text, _)) = lines.first() {
            ui.text_fit(bar[0] + 4.0, bar[1] + 6.0, bar[2] - 8.0, text, 0xFFFFFF);
        }
        if let Some((text, rgb)) = lines.get(1) {
            ui.text_fit(r[0], bar[1] + 24.0, r[2], text, *rgb);
        }
    } else if ui.button_tip([r[0] + r[2] - 100.0, y, 100.0, 20.0], &label, size.is_some() && stem.is_ok(), &tip) {
        match &stem {
            Ok(s) if exists && !confirm => {
                let s = s.clone();
                runtime::with(|st| st.message = tr!("menu.save.exists", format!("{s}.litematic"), tr!("menu.save.overwrite")));
                sc(|st| st.overwrite = Some(s));
            }
            _ => {
                let msg = match runtime::start_save(w, &name, exists) {
                    Ok(m) => m,
                    Err(e) => tr!("error", e),
                };
                runtime::with(|s| s.message = msg);
                sc(|s| s.overwrite = None);
            }
        }
    }
    y += 26.0;
    let half = ((r[2] - 4.0) / 2.0).floor();
    if ui.button([r[0], y, half, 20.0], &tr!("menu.save.clear"), area.iter().any(Option::is_some)) {
        act(w, &["area", "clear"]);
    }
    if ui.button([r[0] + half + 4.0, y, r[2] - half - 4.0, 20.0], &tr!("menu.open_folder"), true) {
        action(w, "folder");
    }
    y += 26.0;
    let (tool, item) = runtime::with(|s| (s.settings.tool, s.settings.tool_item.clone()));
    let label = tr!("menu.save.tool", on_off(tool), item_name(&item));
    if ui.button_tip([r[0], y, r[2], 20.0], &label, true, &[&tr!("menu.save.tool_tip")]) {
        sc(|s| s.tab = 3);
    }
    y += 26.0;
    if tool {
        y += ui.text_wrapped(r[0], y, r[2], &tr!("menu.save.tool_hint", item_name(&item)), 0xA0A0A0);
    }
    // The note at the bottom, where the window leaves room for it (not at 720p with a large GUI).
    if y + 4.0 <= r[1] + r[3] - 30.0 {
        ui.text_wrapped(r[0], r[1] + r[3] - 30.0, r[2], &tr!("menu.save.note"), 0x808080);
    }
}

/// Items the Tool tab offers at a click.
const TOOL_ITEMS: [&str; 8] = [
    "minecraft:stick",
    "minecraft:blaze_rod",
    "minecraft:breeze_rod",
    "minecraft:bone",
    "minecraft:feather",
    "minecraft:wooden_axe",
    "minecraft:golden_hoe",
    "minecraft:arrow",
];

/// The selection tool: which item it is, how it aims, and its controls.
fn tab_tool(w: &World<'_>, ui: &mut Ui<'_>, r: Rect) {
    let (on, item) = runtime::with(|s| (s.settings.tool, s.settings.tool_item.clone()));
    let mut y = r[1];
    ui.fill([r[0], y, 40.0, 40.0], 0x9000_0000);
    ui.frame([r[0], y, 40.0, 40.0], if on { 0xFF55_FF55 } else { 0xFF80_8080 });
    ui.item(r[0] + 4.0, y + 4.0, &item, 32.0);
    let held = w.held();
    ui.text_fit(r[0] + 48.0, y + 6.0, r[2] - 48.0, &tr!("menu.tool.title", item_name(&item)), 0xFFFFFF);
    let (state, rgb) = match (on, held.as_deref() == Some(item.as_str())) {
        (false, _) => (tr!("menu.tool.off"), 0xFF7070),
        (true, true) => (tr!("menu.tool.in_hand"), 0x55FF55),
        (true, false) => (tr!("menu.tool.on"), 0xFFFF55),
    };
    ui.text_fit(r[0] + 48.0, y + 22.0, r[2] - 48.0, &state, rgb);
    y += 48.0;
    ui.text_fit(r[0], y + 2.0, r[2], &tr!("menu.tool.item"), 0xFFFFFF);
    y += 14.0;
    let size = 22.0;
    for (i, it) in TOOL_ITEMS.iter().enumerate() {
        let x = r[0] + i as f32 * (size + 3.0);
        let current = *it == item;
        if ui.button_tip([x, y, size, size], "", !current, &[&item_name(it)]) {
            act(w, &["tool", it]);
        }
        ui.item(x + 3.0, y + 3.0, it, 16.0);
        if current {
            ui.frame([x - 1.0, y - 1.0, size + 2.0, size + 2.0], 0xFFFF_FF55);
        }
    }
    let ex = r[0] + TOOL_ITEMS.len() as f32 * (size + 3.0) + 4.0;
    let (set_label, held_label) = (tr!("menu.set"), tr!("menu.tool.held"));
    let set_w = (ui.text_width(&set_label) + 14.0).max(36.0);
    let held_w = (ui.text_width(&held_label) + 14.0).max(72.0);
    let ew = r[0] + r[2] - ex - set_w - held_w - 8.0;
    let typed = ui.edit("tool_item", [ex, y + 2.0, ew, 18.0], &tr!("menu.tool.item_id"), 64);
    if ui.button([ex + ew + 4.0, y + 1.0, set_w, 20.0], &set_label, !typed.trim().is_empty()) {
        act(w, &["tool", typed.trim()]);
        ui.set_edit("tool_item", "");
    }
    if ui.button_tip([r[0] + r[2] - held_w, y + 1.0, held_w, 20.0], &held_label, held.is_some(), &[&tr!("menu.tool.held_tip")])
        && let Some(h) = &held
    {
        act(w, &["tool", h.as_str()]);
    }
    y += size + 10.0;
    y += settings_grid(ui, w, SELECTION, [r[0], y, r[2], 0.0], 2) + 4.0;
    // As many lines of help as the window leaves room for.
    for i in 1..=4 {
        if y + 9.0 > r[1] + r[3] {
            break;
        }
        ui.text_fit(r[0], y, r[2], &tr!(&format!("menu.tool.help_{i}")), 0xA0A0A0);
        y += 11.0;
    }
}

fn tab_printer(w: &World<'_>, ui: &mut Ui<'_>, r: Rect) {
    let (on, placed, note, loaded) = runtime::with(|s| (s.settings.printer_on, s.placed, s.printer_note.clone(), !s.slots.is_empty()));
    let label = if on { tr!("menu.printer.on") } else { tr!("menu.printer.off") };
    if ui.button_tip([r[0], r[1], r[2], 20.0], &label, loaded, &[&tr!("menu.printer.tip")]) {
        act(w, &["printer"]);
    }
    let mut y = r[1] + 26.0;
    if !loaded {
        ui.text(r[0], y, &tr!("menu.printer.load_first"), 0xA0A0A0);
        y += 12.0;
    }
    ui.text(r[0], y, &tr!("menu.printer.placed", placed), 0xE0E0E0);
    y += 11.0;
    if !note.is_empty() {
        ui.text_fit(r[0], y, r[2], &note, 0xA0A0A0);
    }
    y += 16.0;
    y += settings_grid(ui, w, PRINTER, [r[0], y, r[2], 0.0], 2) + 4.0;
    ui.text(r[0], y + 2.0, &tr!("setting.easy_place"), 0xFFFFFF);
    y += 13.0;
    y += settings_grid(ui, w, EASY_PLACE, [r[0], y, r[2], 0.0], 2);
    // The note at the bottom, where the window leaves room for it.
    if y <= r[1] + r[3] - 22.0 {
        ui.text_wrapped(r[0], r[1] + r[3] - 20.0, r[2], &tr!("menu.printer.note"), 0x808080);
    }
}

fn tab_display(w: &World<'_>, ui: &mut Ui<'_>, r: Rect) {
    let (faces, sections, todo) = crate::overlay::stats();
    let (radius, limit) = runtime::with(|s| (runtime::render_radius(w, &s.settings), s.settings.max_faces));
    let full = limit != 0 && faces >= limit;
    let mib = (faces * crate::overlay::QUAD_BYTES) as f64 / (1024.0 * 1024.0);
    let state = if full { tr!("menu.display.full") } else if todo > 0 { tr!("menu.display.todo", todo) } else { String::new() };
    let line = tr!("menu.display.stats", faces, sections, format!("{mib:.1}"), radius, state);
    ui.text_fit(r[0], r[1] + r[3] - 22.0, r[2], &line, if full { 0xFFFF55 } else { 0xA0A0A0 });
    ui.text_fit(r[0], r[1] + r[3] - 11.0, r[2], &tr!("menu.display.beyond"), 0x808080);
    // The settings above the two lines, scrolled where the window is too low for them (720p,
    // a large GUI scale). A grid row is 24 high (settings_grid), a title 13, the gap 4.
    let sections = [(tr!("menu.display.overlay"), DISPLAY), (tr!("menu.display.hud"), HUD)];
    let area = [r[0], r[1], r[2], r[3] - 26.0];
    let content: f32 = sections.iter().map(|(_, keys)| 13.0 + keys.len().div_ceil(2) as f32 * 24.0 + 4.0).sum::<f32>() - 4.0;
    let wd = if content > area[3] { area[2] - 10.0 } else { area[2] };
    let mut y = ui.scroll_begin("display", area, content);
    for (title, keys) in sections {
        ui.text(r[0], y + 2.0, &title, 0xFFFFFF);
        y += 13.0;
        y += settings_grid(ui, w, keys, [r[0], y, wd, 0.0], 2) + 4.0;
    }
    ui.scroll_end("display", area, content);
}

fn tab_materials(w: &World<'_>, ui: &mut Ui<'_>, r: Rect) {
    let list = runtime::material_list(w);
    let hud = runtime::with(|s| s.settings.materials_hud);
    let hud_label = tr!("menu.mat.hud", on_off(hud));
    let copy_label = tr!("menu.mat.copy");
    let bw = (ui.text_width(&hud_label) + 16.0).max(150.0);
    let copy_w = (ui.text_width(&copy_label) + 16.0).max(70.0);
    if ui.button_tip([r[0] + r[2] - bw, r[1], bw, 20.0], &hud_label, true, &[&tr!("menu.mat.hud_tip")]) {
        runtime::with(|s| s.settings.materials_hud = !s.settings.materials_hud);
        runtime::save_settings(w);
    }
    if ui.button_tip([r[0] + r[2] - bw - copy_w - 4.0, r[1], copy_w, 20.0], &copy_label, !list.is_empty(), &[&tr!("menu.mat.copy_tip")]) {
        act(w, &["materials", "copy"]);
    }
    let fetch: u64 = list.iter().map(|m| m.to_fetch()).sum();
    let summary = if list.is_empty() { tr!("menu.mat.empty") } else { tr!("menu.mat.summary", list.len(), fetch) };
    ui.text_fit(r[0], r[1] + 6.0, r[2] - bw - copy_w - 12.0, &summary, 0xE0E0E0);
    // The highlights: what they mark, and a row of the list below picks one item.
    let (missing, wrong, only) = runtime::with(|s| (s.settings.highlight_missing, s.settings.highlight_wrong, s.highlight_item.clone()));
    let title = tr!("menu.mat.highlight");
    ui.text(r[0], r[1] + 30.0, &title, 0xE0E0E0);
    let tip = tr!("menu.mat.highlight_tip");
    let (missing_label, wrong_label) = (tr!("menu.mat.missing", on_off(missing)), tr!("menu.mat.wrong", on_off(wrong)));
    let x1 = r[0] + (ui.text_width(&title) + 6.0).max(52.0);
    let w1 = (ui.text_width(&missing_label) + 14.0).max(86.0);
    let w2 = (ui.text_width(&wrong_label) + 14.0).max(80.0);
    if ui.button_tip([x1, r[1] + 24.0, w1, 20.0], &missing_label, true, &[&tip]) {
        runtime::with(|s| s.settings.highlight_missing = !s.settings.highlight_missing);
        runtime::save_settings(w);
    }
    if ui.button_tip([x1 + w1 + 4.0, r[1] + 24.0, w2, 20.0], &wrong_label, true, &[&tip]) {
        runtime::with(|s| s.settings.highlight_wrong = !s.settings.highlight_wrong);
        runtime::save_settings(w);
    }
    let shown = only.as_deref().map_or_else(|| tr!("menu.mat.all_items"), runtime::pretty_item);
    let x3 = x1 + w1 + w2 + 8.0;
    if ui.button_tip([x3, r[1] + 24.0, r[0] + r[2] - x3, 20.0], &shown, only.is_some(), &[&tr!("menu.mat.only_tip")]) {
        runtime::with(|s| s.highlight_item = None);
    }
    let table = [r[0], r[1] + 48.0, r[2], r[3] - 48.0];
    let cols = [table[0] + table[2] - 150.0, table[0] + table[2] - 90.0, table[0] + table[2] - 12.0];
    ui.text(table[0] + 24.0, table[1], &tr!("menu.mat.col_item"), 0xFFFFFF);
    ui.text_right(cols[0], table[1], &tr!("menu.mat.col_missing"), 0xFFFFFF);
    ui.text_right(cols[1], table[1], &tr!("menu.mat.col_total"), 0xFFFFFF);
    ui.text_right(cols[2], table[1], &tr!("menu.mat.col_carried"), 0xFFFFFF);
    let area = [table[0], table[1] + 11.0, table[2], table[3] - 11.0];
    ui.fill(area, 0x9000_0000);
    let row_h = 18.0;
    let content = list.len() as f32 * row_h + 2.0;
    let y0 = ui.scroll_begin("materials", area, content);
    let mut save = false;
    for (i, m) in list.iter().enumerate() {
        let y = y0 + 1.0 + i as f32 * row_h;
        if y + row_h < area[1] || y > area[1] + area[3] {
            continue;
        }
        let row = [area[0] + 1.0, y, area[2] - 8.0, row_h];
        if i % 2 == 1 {
            ui.fill(row, 0x18FF_FFFF);
        }
        let name = runtime::pretty_item(&m.item);
        if only.as_deref() == Some(m.item.as_str()) {
            ui.fill(row, 0x5055_FFFF);
        }
        if ui.hovered(row) {
            ui.fill(row, 0x25FF_FFFF);
            ui.tooltip(&[&name, &tr!("menu.mat.to_fetch", m.to_fetch()), &tr!("menu.mat.click_only"), &m.item]);
        }
        if ui.clicked(row) {
            runtime::with(|s| {
                let again = s.highlight_item.as_deref() == Some(m.item.as_str());
                s.highlight_item = (!again).then(|| m.item.clone());
                if !again && !s.settings.highlight_wrong {
                    s.settings.highlight_missing = true;
                }
            });
            save = true;
        }
        ui.item(row[0] + 2.0, y + 1.0, &m.item, 16.0);
        ui.text_fit(row[0] + 23.0, y + 5.0, cols[0] - row[0] - 80.0, &name, 0xE0E0E0);
        let done = m.to_fetch() == 0;
        ui.text_right(cols[0], y + 5.0, &m.missing.to_string(), if m.missing == 0 { 0x55FF55 } else { 0xFF7070 });
        ui.text_right(cols[1], y + 5.0, &m.total.to_string(), 0xA0A0A0);
        ui.text_right(cols[2], y + 5.0, &m.available.to_string(), if done { 0x55FF55 } else { 0xFFAA00 });
    }
    ui.scroll_end("materials", area, content);
    if save {
        runtime::save_settings(w);
    }
}

fn tab_credits(w: &World<'_>, ui: &mut Ui<'_>, r: Rect) {
    // Scrolled where the window is too low for it (720p, a large GUI scale).
    let area = [r[0], r[1], r[2], r[3] - 24.0];
    let people = [
        ("Horizonmatica".to_string(), "keksi2k".to_string(), "github.com/keksi2k/horizonmatica"),
        ("Litematica".to_string(), tr!("menu.credits.litematica"), "github.com/sakura-ryoko/litematica"),
        ("Litematica Printer".to_string(), tr!("menu.credits.printer"), "github.com/aleksilassila/litematica-printer"),
        ("Horizon".to_string(), tr!("menu.credits.horizon"), "github.com/horizon-rust/horizon"),
        (tr!("menu.credits.formats"), "Litematica, Sponge/WorldEdit, MCEdit, Mojang".to_string(), ".litematic .schem .schematic .nbt"),
    ];
    let content = 4.0 + 12.0 + 20.0 + people.len() as f32 * 39.0 + 16.0 + 10.0;
    let wd = if content > area[3] { area[2] - 10.0 } else { area[2] };
    let cx = r[0] + wd / 2.0;
    let mut y = ui.scroll_begin("credits", area, content) + 4.0;
    ui.text_centered(cx, y, &tr!("menu.credits.title"), 0xFFFFFF);
    y += 12.0;
    ui.text_centered(cx, y, &tr!("menu.credits.all_credit"), 0xA0A0A0);
    y += 20.0;
    for (what, who, link) in people {
        ui.text_centered(cx, y, &what, 0x55FFFF);
        y += 11.0;
        ui.text_centered(cx, y, &who, 0xE0E0E0);
        y += 11.0;
        ui.text_centered(cx, y, link, 0x808080);
        y += 17.0;
    }
    ui.text_centered(cx, y, &ui.fit(&tr!("menu.credits.not_endorsed"), wd), 0xA0A0A0);
    y += 16.0;
    ui.text_centered(cx, y, &ui.fit(&tr!("menu.credits.keys"), wd), 0x808080);
    ui.scroll_end("credits", area, content);
    if ui.button([cx - 100.0, r[1] + r[3] - 20.0, 200.0, 20.0], &tr!("menu.credits.chat"), true) {
        action(w, "credits_chat");
    }
}
