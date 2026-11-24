//! Blazematica's state and everything it does each tick, on commands, keys and clicks. The game
//! is read through `World` (the mod API); the game-independent logic is `blazematica_core`.

use crate::settings::{self, Mode, Settings};
use crate::store;
use crate::world::{Blocks, StateId, World};
use crate::i18n::{item_name, on_off as on_off_text};
use blazematica_core::formats::Format;
use blazematica_core::materials::{self, Material};
use blazematica_core::placement::{LayerAxis, LayerFilter};
use blazematica_core::printer::{Recent, Stuck};
use blazematica_core::verify::{self, Status, Tally};
use blazematica_core::{BlockState, LayerMode, Mirror, Placement, Rotation, Schematic};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

/// Cells the verifier scans per tick.
const SCAN_PER_TICK: u64 = 32_768;

// ---------------------------------------------------------------- state

#[derive(Default)]
struct Scan {
    /// The placement scanned: its slot id and revision.
    key: (u64, u64),
    index: u64,
    tally: Tally,
    missing: BTreeMap<String, u64>,
    unloaded: u64,
}

/// One loaded schematic. Several can be loaded at once; one of them is selected, and the
/// commands, move mode, the verifier's HUD and the material list are about that one.
pub struct Slot {
    /// Unique for the session (never reused): caches key palette entries by it.
    pub id: u64,
    pub placement: Placement,
    /// Drawn and printed. A hidden placement stays loaded and remembered.
    pub shown: bool,
    /// Can't be moved, turned or mirrored (by commands, move mode or the tool).
    pub locked: bool,
    /// Every item the placement needs (counted once on load).
    pub totals: BTreeMap<String, u64>,
}

/// Where a block of a placement comes from: (slot index, (region, palette index)).
pub type Ref = (usize, (usize, u32));

/// What the game makes of a palette entry (cached per placement revision).
#[derive(Clone, Copy, Debug)]
pub struct Resolved {
    pub id: Option<StateId>,
    pub air: bool,
    /// Water (1) or lava (2) itself, not a waterlogged block; 0 for anything else.
    pub fluid: u8,
}

/// What the selection tool's wheel moves (Litematica's selected corner or box).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Grab {
    /// The whole box: both corners.
    #[default]
    Box,
    /// One corner (0 or 1).
    Corner(usize),
}

#[derive(Default)]
pub struct State {
    settings_loaded: bool,
    /// Game ticks seen (client ticks while in a world).
    pub ticks: u64,
    /// The world (and dimension) the placements belong to: `store::world_key`.
    world_key: Option<String>,
    pub settings: Settings,
    /// Lines for the chat, sent at the next hook that may write to it.
    pub out: Vec<String>,
    /// The loaded placements, drawn in this order (a later one's blocks win where they overlap).
    pub slots: Vec<Slot>,
    /// Index into `slots` of the selected placement.
    pub selected: usize,
    next_id: u64,
    /// Bumped whenever a placement is added, removed, shown, hidden, moved or turned
    /// (`sync_rev`): everything built from them is stale.
    pub rev: u64,
    rev_of: Vec<(u64, u64, bool)>,
    /// The area selection's two corners (`/lm pos1`, `/lm pos2`), saved with `/lm save`.
    pub area: [Option<[i32; 3]>; 2],
    pub layer: LayerFilter,
    pub blocks: Blocks,
    /// Item per wanted state (None: no item, such as air or fire).
    items: HashMap<BlockState, Option<String>>,
    unknown: Vec<String>,
    scan: Scan,
    /// The verifier's last full result, for the slot id `tally_of`.
    tally_of: u64,
    tally: Option<Tally>,
    pub missing: BTreeMap<String, u64>,
    unloaded: u64,
    /// Palette entry (slot id, region, index) -> what the game makes of it, for `resolved_rev`.
    resolved: HashMap<(u64, usize, u32), Resolved>,
    resolved_rev: u64,
    pub recent: Recent,
    pub stuck: Stuck,
    /// Move mode: the wheel and the arrow keys move the placement.
    pub moving: bool,
    /// Wheel steps not yet a whole block (touchpads send fractions).
    wheel: f32,
    /// Schematic files being read (`start_load`).
    loads: Vec<LoadJob>,
    /// Files a memory warning was given for, and when: loading them again soon loads them anyway.
    memory_warned: Option<(Vec<String>, std::time::Instant)>,
    /// The save under way (`/lm save`), and the file its thread is writing.
    pub save: Option<SaveJob>,
    save_write: Option<SaveWrite>,
    /// What the selection tool's wheel moves, and the mouse buttons whose press it took.
    pub grab: Grab,
    tool_down: [bool; 3],
    /// The credits line was shown this session (on the first load).
    credited: bool,
    pub printer_note: String,
    pub easy_note: String,
    pub placed: u64,
    /// The highlights show only blocks of this item (a session's choice).
    pub highlight_item: Option<String>,
    /// The last message a command or a load gave (the menu shows it).
    pub message: String,
    /// A paste under way (`/lm paste`).
    paste: Option<Paste>,
    /// Commands for the game to send as the player's (`take_commands`).
    commands: Vec<String>,
    /// Hover info switched off (`/lm hover off`) for this session.
    no_hover: bool,
}

impl State {
    /// The settings were read from the store (once per session, in a world).
    pub fn settings_ready(&self) -> bool {
        self.settings_loaded
    }
    pub fn sel(&self) -> Option<&Slot> {
        self.slots.get(self.selected)
    }
    pub fn placement(&self) -> Option<&Placement> {
        self.sel().map(|s| &s.placement)
    }
    fn placement_mut(&mut self) -> Option<&mut Placement> {
        self.slots.get_mut(self.selected).map(|s| &mut s.placement)
    }
    /// The verifier's last full result, when it is the selected placement's.
    pub fn tally(&self) -> Option<&Tally> {
        self.tally.as_ref().filter(|_| self.sel().is_some_and(|x| x.id == self.tally_of))
    }
    pub fn state_of(&self, (slot, r): Ref) -> &BlockState {
        self.slots[slot].placement.palette_state(r)
    }
    /// Bumps `rev` when the placements changed since the last call.
    fn sync_rev(&mut self) {
        let now: Vec<(u64, u64, bool)> = self.slots.iter().map(|s| (s.id, s.placement.revision, s.shown)).collect();
        if now != self.rev_of {
            self.rev_of = now;
            self.rev += 1;
        }
    }
    /// The block wanted at `w`: the last shown placement's with one there. As with regions, a
    /// later placement's air doesn't erase an earlier placement's block.
    pub fn wanted_at(&self, w: [i32; 3]) -> Option<Ref> {
        let mut found: Option<Ref> = None;
        for (i, slot) in self.slots.iter().enumerate() {
            if !slot.shown {
                continue;
            }
            if let Some(r) = slot.placement.wanted_ref(w)
                && (found.is_none() || !slot.placement.palette_state(r).is_air())
            {
                found = Some((i, r));
            }
        }
        found
    }
    /// The box around every shown placement (min, max inclusive).
    pub fn shown_bounds(&self) -> Option<([i32; 3], [i32; 3])> {
        self.slots.iter().filter(|s| s.shown).map(|s| s.placement.bounds()).reduce(|(alo, ahi), (blo, bhi)| {
            ([0, 1, 2].map(|i| alo[i].min(blo[i])), [0, 1, 2].map(|i| ahi[i].max(bhi[i])))
        })
    }
    /// The area selection's box, once both corners are set.
    pub fn area_box(&self) -> Option<([i32; 3], [i32; 3])> {
        let [Some(a), Some(b)] = self.area else { return None };
        Some(([0, 1, 2].map(|i| a[i].min(b[i])), [0, 1, 2].map(|i| a[i].max(b[i]))))
    }
    /// The area selection's size in blocks, checked (`None` without both corners).
    pub fn area_size(&self) -> Option<[i32; 3]> {
        let [Some(a), Some(b)] = self.area else { return None };
        blazematica_core::area::size(a, b)
    }
    /// Drops every placement (another world, `/lm unload all`).
    fn drop_all(&mut self) {
        reset_caches(self);
        self.slots.clear();
        self.selected = 0;
        self.settings.printer_on = false;
        self.moving = false;
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

pub fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with_borrow_mut(f)
}

/// Queues a chat line (`flush` sends it).
pub fn say(text: &str) {
    with(|s| s.out.push(format!("[Blazematica] {text}")));
}

/// The chat lines queued, to send.
/// The commands to send as the player's, in order (a paste's `/setblock`s).
pub fn take_commands() -> Vec<String> {
    with(|s| std::mem::take(&mut s.commands))
}

/// The game refused to send one of `take_commands`'s commands: a paste stops.
pub fn command_refused(why: &str) {
    with(|s| {
        if let Some(p) = s.paste.take() {
            s.out.push(tr!("error", tr!("err.paste_refused", p.placed, why)));
        }
    });
}

pub fn take_out() -> Vec<String> {
    with(|s| std::mem::take(&mut s.out))
}

fn report(r: Result<Vec<String>, String>) {
    match r {
        Ok(lines) => {
            for l in lines {
                say(&l);
            }
        }
        Err(e) => say(&tr!("error", e)),
    }
}

/// "not in a world", for `ok_or_else`.
fn no_world() -> String {
    tr!("err.not_in_world")
}

/// A placement's move that would leave the world, in words.
fn outside_world() -> String {
    use blazematica_core::area::{MAX_XZ, MAX_Y, MIN_XZ, MIN_Y};
    tr!("err.outside_world", MIN_XZ, MAX_XZ, MIN_Y, MAX_Y)
}

/// A corner where a world has blocks, else the coordinate that is out, in words.
fn check_corner(c: [i32; 3]) -> Result<[i32; 3], String> {
    use blazematica_core::area::{MAX_XZ, MAX_Y, MIN_XZ, MIN_Y};
    blazematica_core::area::check_corner(c).map_err(|_| {
        let (lo, hi) = ([MIN_XZ, MIN_Y, MIN_XZ], [MAX_XZ, MAX_Y, MAX_XZ]);
        let k = (0..3).find(|&k| !(lo[k]..=hi[k]).contains(&c[k])).unwrap_or(0);
        tr!("err.coordinate_range", ["x", "y", "z"][k], lo[k], hi[k], c[k])
    })
}

/// A rotation in words.
pub fn rotation_text(r: Rotation) -> String {
    tr!(&format!("rotation.{}", r.name()))
}

/// A mirror in words.
pub fn mirror_text(m: Mirror) -> String {
    tr!(&format!("mirror.{}", m.name()))
}

/// The layers shown, in words.
pub fn layer_text(layer: LayerFilter) -> String {
    let axis = layer.axis.name();
    match (layer.axis, layer.mode) {
        (LayerAxis::Y, LayerMode::All) => tr!("layer.all"),
        (LayerAxis::Y, LayerMode::Single(y)) => tr!("layer.single", y),
        (LayerAxis::Y, LayerMode::Above(y)) => tr!("layer.above", y),
        (LayerAxis::Y, LayerMode::Below(y)) => tr!("layer.below", y),
        (LayerAxis::Y, LayerMode::Range(a, b)) => tr!("layer.range", a.min(b), a.max(b)),
        (_, LayerMode::All) => format!("{} ({axis})", tr!("layer.all")),
        (_, LayerMode::Single(n)) => format!("{axis} = {n}"),
        (_, LayerMode::Above(n)) => format!("{axis} >= {n}"),
        (_, LayerMode::Below(n)) => format!("{axis} <= {n}"),
        (_, LayerMode::Range(a, b)) => format!("{axis} {}..{}", a.min(b), a.max(b)),
    }
}

/// A schematic file format in words.
pub fn format_text(f: Format) -> String {
    match f {
        Format::Litematic => tr!("format.litematic"),
        Format::Sponge(v) => tr!("format.sponge", v),
        Format::Structure => tr!("format.structure"),
        Format::McEdit => tr!("format.mcedit"),
    }
}

// ---------------------------------------------------------------- settings and placements

pub fn ensure_settings(w: &World<'_>) {
    if with(|s| s.settings_loaded) {
        return;
    }
    settings::migrate(w);
    with(|s| {
        s.settings = settings::load(w, s.settings.printer_on);
        s.settings_loaded = true;
    });
}

/// The settings changed elsewhere (the mod's page in Options > Mods): read again.
pub fn settings_changed() {
    with(|s| s.settings_loaded = false);
}

pub fn save_settings(w: &World<'_>) {
    let st = with(|s| s.settings.clone());
    settings::save(w, &st);
}

pub fn render_radius(w: &World<'_>, st: &Settings) -> i32 {
    if st.radius_view {
        w.host.graphics_options().render_distance.max(1) as i32 * 16
    } else {
        st.radius
    }
}

/// Where a saved placement goes: origin, rotation, mirror.
type At = ([i32; 3], Rotation, Mirror);

/// Saves where the placements are: the selected one as the last one (`/lm reload`), all of them
/// as this world's, which come back when the world is joined again.
///
/// A world's value is `v2`, the selected index, then one line per placement:
/// `x|y|z|rotation|mirror|flags|file` (flags: 1 shown, 2 locked).
pub fn save_placements(w: &World<'_>) {
    let (last, world) = with(|s| {
        let last = s.placement().map(|p| {
            format!("{}|{}|{}|{}|{}|{}", p.file_name, p.origin[0], p.origin[1], p.origin[2], p.rotation.name(), p.mirror.name())
        });
        let mut world = if s.slots.is_empty() { String::new() } else { format!("v2\n{}", s.selected) };
        for slot in &s.slots {
            let p = &slot.placement;
            world += &format!(
                "\n{}|{}|{}|{}|{}|{}|{}",
                p.origin[0],
                p.origin[1],
                p.origin[2],
                p.rotation.name(),
                p.mirror.name(),
                u8::from(slot.shown) | u8::from(slot.locked) << 1,
                p.file_name
            );
        }
        (last, world)
    });
    if let Some(last) = last {
        store::write_value(w, "last", &last);
    }
    if let Some(key) = store::world_key(w) {
        store::write_value(w, &key, &world);
    }
}

/// A world's saved placements (either format): the selected index, and per placement its file,
/// where it goes, and whether it is shown and locked.
fn parse_world(v: &str) -> (usize, Vec<(String, At, (bool, bool))>) {
    let mut lines = v.lines();
    if lines.next() != Some("v2") {
        return (0, parse_saved(v).map(|(file, at)| (file, at, (true, false))).into_iter().collect());
    }
    let selected = lines.next().and_then(|l| l.trim().parse().ok()).unwrap_or(0);
    let entries = lines
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(7, '|').collect();
            if parts.len() != 7 || parts[6].is_empty() {
                return None;
            }
            let n = |i: usize| parts[i].parse::<i32>().ok();
            let at = ([n(0)?, n(1)?, n(2)?], Rotation::parse(parts[3]).unwrap_or_default(), Mirror::parse(parts[4]).unwrap_or_default());
            let flags = parts[5].parse::<u8>().unwrap_or(1);
            Some((parts[6].to_string(), at, (flags & 1 != 0, flags & 2 != 0)))
        })
        .collect();
    (selected, entries)
}

/// A saved placement: file, origin, rotation, mirror.
fn parse_saved(v: &str) -> Option<(String, At)> {
    let parts: Vec<&str> = v.trim().split('|').collect();
    if parts.len() != 6 || parts[0].is_empty() {
        return None;
    }
    let n = |i: usize| parts[i].parse::<i32>().ok();
    let at = ([n(1)?, n(2)?, n(3)?], Rotation::parse(parts[4]).unwrap_or_default(), Mirror::parse(parts[5]).unwrap_or_default());
    Some((parts[0].to_string(), at))
}

/// Joining a world (or another dimension of it): its placements come back where they were left;
/// those of another world go, and so does the area selection.
fn restore_step(w: &World<'_>) {
    let Some(key) = store::world_key(w) else { return };
    if with(|s| s.world_key.as_deref() == Some(key.as_str())) {
        return;
    }
    with(|s| {
        s.world_key = Some(key.clone());
        s.drop_all();
        s.area = [None; 2];
    });
    crate::overlay::clear();
    let Some(saved) = store::read_value(w, &key) else { return };
    let (selected, entries) = parse_world(&saved);
    if entries.is_empty() {
        return;
    }
    let flags = entries.iter().map(|e| e.2).collect();
    let files = entries.into_iter().map(|(file, at, _)| (file, at)).collect();
    let lines = start_load(w, files, Then::Restore { flags, selected }).unwrap_or_else(|e| vec![tr!("err.restore", e)]);
    for l in lines {
        say(&l);
    }
}

// ---------------------------------------------------------------- block lookups

fn reset_caches(s: &mut State) {
    // Keyed by palette index and checked only against the placement's revision, which a new
    // placement starts again at 0: without this a new schematic shows the old one's blocks.
    s.resolved.clear();
    s.items.clear();
    s.unknown.clear();
    s.scan = Scan::default();
    s.tally = None;
    s.missing.clear();
    s.unloaded = 0;
    s.recent.clear();
    s.stuck.clear();
}

pub fn wanted_id(w: &World<'_>, s: &mut State, st: &BlockState) -> Option<StateId> {
    let id = s.blocks.resolve(w, st);
    if id.is_none() && !st.is_air() && !st.is_void() && !s.unknown.contains(&st.name) {
        s.unknown.push(st.name.clone());
    }
    id
}

pub fn item_for(w: &World<'_>, s: &mut State, st: &BlockState) -> Option<String> {
    if let Some(i) = s.items.get(st) {
        return i.clone();
    }
    let item = match st.name.as_str() {
        "minecraft:water" | "minecraft:bubble_column" => Some("minecraft:water_bucket".to_string()),
        "minecraft:lava" => Some("minecraft:lava_bucket".to_string()),
        name if materials::planted_by(name).is_some() => materials::planted_by(name).map(String::from),
        _ => wanted_id(w, s, st).and_then(|id| s.blocks.item(w, id)),
    };
    s.items.insert(st.clone(), item.clone());
    item
}

fn fluid_of(st: &BlockState) -> u8 {
    match st.name.as_str() {
        "minecraft:water" | "minecraft:bubble_column" => 1,
        "minecraft:lava" => 2,
        _ => 0,
    }
}

pub fn resolve_ref(w: &World<'_>, s: &mut State, at: Ref) -> Option<Resolved> {
    if s.resolved_rev != s.rev {
        s.resolved.clear();
        s.resolved_rev = s.rev;
    }
    let key = (s.slots.get(at.0)?.id, at.1.0, at.1.1);
    if let Some(v) = s.resolved.get(&key) {
        return Some(*v);
    }
    let state = s.state_of(at).clone();
    let id = wanted_id(w, s, &state);
    let res = Resolved { id, air: state.is_air(), fluid: fluid_of(&state) };
    s.resolved.insert(key, res);
    Some(res)
}

/// The verifier's verdict at `w` against the shown placements (None: outside them, unloaded, or
/// nothing to say), with where its wanted state is and what the game makes of it.
pub fn quick_status(w: &World<'_>, s: &mut State, p: [i32; 3]) -> Option<(Status, Ref, Resolved)> {
    let at = s.wanted_at(p)?;
    judge(w, s, p, at)
}

/// The verdict at `p` against the placement in slot `slot` alone (shown or not).
fn quick_status_in(w: &World<'_>, s: &mut State, slot: usize, p: [i32; 3]) -> Option<(Status, Ref, Resolved)> {
    let r = s.slots.get(slot)?.placement.wanted_ref(p)?;
    judge(w, s, p, (slot, r))
}

/// The verdict at `p` for the wanted block `at`. The common cases (the right block, air where a
/// block is missing) need no state compared.
fn judge(w: &World<'_>, s: &mut State, p: [i32; 3], at: Ref) -> Option<(Status, Ref, Resolved)> {
    let res = resolve_ref(w, s, at)?;
    let actual_id = w.block(p)?;
    if !res.air && res.id == Some(actual_id) {
        return Some((Status::Correct, at, res));
    }
    let actual = s.blocks.state(w, actual_id);
    if actual.is_air() {
        return (!res.air).then_some((Status::Missing, at, res));
    }
    let replaceable = s.blocks.replaceable(w, actual_id) || (fluid_of(&actual) != 0 && actual.prop("level") != Some("0"));
    let replaceable = replaceable || fluid_of(&actual) != 0;
    let wanted = s.state_of(at).clone();
    let status = verify::classify_power(Some(&wanted), &actual, replaceable, s.settings.strict, s.settings.power)?;
    Some((status, at, res))
}

/// The verifier's verdict at `p` against the shown placements, and the state wanted there.
pub fn status_at(w: &World<'_>, s: &mut State, p: [i32; 3]) -> Option<(Status, BlockState)> {
    let (status, at, _) = quick_status(w, s, p)?;
    Some((status, s.state_of(at).clone()))
}

// ---------------------------------------------------------------- loading

/// Counts every item a placement needs (once per load).
fn count_totals(w: &World<'_>, s: &mut State, p: &Placement, histograms: &[Vec<u64>]) -> BTreeMap<String, u64> {
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for (ri, histogram) in histograms.iter().enumerate() {
        let palette = &p.palettes()[ri];
        for (i, &n) in histogram.iter().enumerate() {
            let st = &palette[i];
            let per = u64::from(materials::items_for_state(st));
            if n == 0 || per == 0 {
                continue;
            }
            if let Some(item) = item_for(w, s, st) {
                *counts.entry(item).or_insert(0) += n * per;
            }
        }
    }
    counts
}

/// What a placement is called: its schematic's name, else its file's.
pub fn display_name(p: &Placement) -> String {
    if p.schematic.metadata.name.is_empty() { p.file_name.clone() } else { p.schematic.metadata.name.clone() }
}

/// A schematic file as read off the game thread: parsed, and its cells counted per palette entry
/// (for the material totals). Placing it is quick.
struct ReadFile {
    file: String,
    schematic: Schematic,
    format: Format,
    /// The data version it was saved with, and how many block states were brought up to date.
    from_version: i64,
    upgraded: usize,
    histograms: Vec<Vec<u64>>,
}

fn read_file(dir: &std::path::Path, query: &str) -> Result<ReadFile, String> {
    let file = blazematica_core::find_schematic(dir, query)?;
    let read_error = |e: String| match e.as_str() {
        blazematica_core::nbt::OUT_OF_MEMORY => tr!("err.out_of_memory", &file),
        _ => format!("{file}: {e}"),
    };
    let bytes = blazematica_core::nbt::read_input(&dir.join(&file)).map_err(read_error)?;
    let (mut schematic, format) = blazematica_core::formats::load(&bytes).map_err(read_error)?;
    drop(bytes);
    let from_version = schematic.data_version;
    let upgraded = crate::fix::upgrade(&mut schematic);
    let histograms = schematic.regions.iter().map(|r| r.histogram()).collect();
    Ok(ReadFile { file, schematic, format, from_version, upgraded, histograms })
}

/// What becomes of files once they are read.
enum Then {
    /// `/lm load`: a new placement, selected.
    Load,
    /// `/lm reload`: replaces the placement of that file at that place, else a new one.
    Reload,
    /// Joining a world: its placements back, each shown or hidden, and the one selected.
    Restore { flags: Vec<(bool, bool)>, selected: usize },
}

/// Files a thread of their own is reading (`start_load`); `load_step` places them.
struct LoadJob {
    files: Vec<(String, At)>,
    then: Then,
    /// The world they are for: left before they are in, they are dropped.
    world: Option<String>,
    started: std::time::Instant,
    bytes: u64,
    done: std::sync::mpsc::Receiver<Vec<Result<ReadFile, String>>>,
}

impl LoadJob {
    fn label(&self) -> String {
        match self.files.as_slice() {
            [(f, _)] => f.clone(),
            many => tr!("chat.n_placements", many.len()),
        }
    }
}

/// How long a command waits for its files before they go on loading in the background.
const LOAD_WAIT: std::time::Duration = std::time::Duration::from_millis(150);

/// How long loading the same files again after a memory warning loads them anyway.
const WARNED_FOR: std::time::Duration = std::time::Duration::from_secs(60);

/// A warning where the files likely need more memory than the computer has to give, unless the
/// same files were warned about a moment ago (then they load anyway). Never a limit of its own.
fn memory_warning(dir: &std::path::Path, queries: &[String]) -> Option<String> {
    let room = crate::memory::Room::now()?;
    let names: Vec<String> = queries.iter().filter_map(|q| blazematica_core::find_schematic(dir, q).ok()).collect();
    let need = names.iter().filter_map(|f| crate::memory::need(&dir.join(f))).fold(0u64, u64::saturating_add);
    if room.fits(need) {
        return None;
    }
    let again = with(|s| match s.memory_warned.take() {
        Some((warned, at)) if warned == names && at.elapsed() < WARNED_FOR => true,
        _ => {
            s.memory_warned = Some((names.clone(), std::time::Instant::now()));
            false
        }
    });
    if again {
        return None;
    }
    let label = match names.as_slice() {
        [one] => one.clone(),
        many => tr!("chat.n_placements", many.len()),
    };
    Some(match room {
        crate::memory::Room::Free(free) => tr!("chat.memory_free", label, human_size(need), human_size(free)),
        crate::memory::Room::Total(total) => tr!("chat.memory_total", label, human_size(need), human_size(total)),
    })
}

fn start_load(w: &World<'_>, files: Vec<(String, At)>, then: Then) -> Result<Vec<String>, String> {
    use std::sync::mpsc::RecvTimeoutError;
    let dir = store::schematics_dir(w)?;
    let queries: Vec<String> = files.iter().map(|(f, _)| f.clone()).collect();
    let bytes = queries
        .iter()
        .filter_map(|q| blazematica_core::find_schematic(&dir, q).ok())
        .filter_map(|f| std::fs::metadata(dir.join(f)).ok())
        .map(|m| m.len())
        .sum();
    // Joining a world brings back what was loaded before: no question then.
    if !matches!(then, Then::Restore { .. })
        && let Some(warning) = memory_warning(&dir, &queries)
    {
        return Ok(vec![warning]);
    }
    let (tx, done) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("blazematica-load".into())
        .spawn(move || {
            let _ = tx.send(queries.iter().map(|q| read_file(&dir, q)).collect());
        })
        .map_err(|e| tr!("err.start_reading", e))?;
    let job = LoadJob { files, then, world: store::world_key(w), started: std::time::Instant::now(), bytes, done };
    match job.done.recv_timeout(LOAD_WAIT) {
        Ok(read) => finish_load(w, job, read),
        Err(RecvTimeoutError::Disconnected) => Err(tr!("err.reading_failed", job.label())),
        Err(RecvTimeoutError::Timeout) => {
            let line = tr!("chat.loading_background", job.label(), human_size(job.bytes));
            with(|s| s.loads.push(job));
            Ok(vec![line])
        }
    }
}

/// Places the files of a job that are in; jobs for a world since left are dropped.
fn load_step(w: &World<'_>) {
    use std::sync::mpsc::TryRecvError;
    let jobs = with(|s| std::mem::take(&mut s.loads));
    let mut left = Vec::new();
    for job in jobs {
        let lines = match job.done.try_recv() {
            Err(TryRecvError::Empty) => {
                left.push(job);
                continue;
            }
            Ok(_) if job.world != store::world_key(w) => continue,
            Ok(read) => finish_load(w, job, read),
            Err(TryRecvError::Disconnected) => Err(tr!("err.reading_failed", job.label())),
        };
        let lines = lines.unwrap_or_else(|e| vec![tr!("error", e)]);
        if let Some(first) = lines.first() {
            with(|s| s.message = first.clone());
        }
        for l in &lines {
            say(l);
        }
    }
    with(|s| {
        left.append(&mut s.loads);
        s.loads = left;
    });
}

/// The HUD's line for each job still reading.
fn load_lines(s: &State) -> Vec<(String, u32)> {
    s.loads
        .iter()
        .map(|j| {
            let secs = j.started.elapsed().as_secs();
            (tr!("hud.loading", j.label(), human_size(j.bytes), secs), 0xFFFF55)
        })
        .collect()
}

fn finish_load(w: &World<'_>, job: LoadJob, read: Vec<Result<ReadFile, String>>) -> Result<Vec<String>, String> {
    match job.then {
        Then::Load => {
            let (read, at) = first_read(&job.files, read)?;
            let (slot, tail) = place(w, read, at)?;
            let mut msg = loaded_line(&slot, false) + &tail;
            let count = with(|s| {
                s.slots.push(slot);
                s.selected = s.slots.len() - 1;
                s.layer.mode = LayerMode::All;
                s.moving = false;
                s.slots.len()
            });
            save_placements(w);
            if count > 1 {
                msg += &tr!("chat.placement_n_of", count, count);
            }
            if !with(|s| std::mem::replace(&mut s.credited, true)) {
                say(&tr!("chat.credit_note"));
            }
            Ok(vec![msg])
        }
        Then::Reload => {
            let (read, at) = first_read(&job.files, read)?;
            let (slot, tail) = place(w, read, at)?;
            let file = slot.placement.file_name.clone();
            let same = with(|s| s.slots.iter().any(|x| x.placement.file_name == file && x.placement.origin == at.0));
            let msg = loaded_line(&slot, same) + &tail;
            with(|s| {
                let same = s.slots.iter().position(|x| x.placement.file_name == slot.placement.file_name && x.placement.origin == at.0);
                match same {
                    Some(i) => {
                        let (shown, locked) = (s.slots[i].shown, s.slots[i].locked);
                        s.slots[i] = Slot { shown, locked, ..slot };
                        s.selected = i;
                    }
                    None => {
                        s.slots.push(slot);
                        s.selected = s.slots.len() - 1;
                    }
                }
            });
            save_placements(w);
            Ok(vec![msg])
        }
        Then::Restore { flags, selected } => {
            let mut lines = Vec::new();
            let mut restored = Vec::new();
            for (((file, at), read), (shown, locked)) in job.files.iter().zip(read).zip(flags) {
                match read.and_then(|read| place(w, read, *at)) {
                    Ok((mut slot, _)) => {
                        slot.shown = shown;
                        slot.locked = locked;
                        restored.push(tr!("chat.file_at", file, at.0[0], at.0[1], at.0[2]));
                        with(|s| s.slots.push(slot));
                    }
                    Err(e) => lines.push(tr!("err.restore_file", file, e)),
                }
            }
            with(|s| s.selected = selected.min(s.slots.len().saturating_sub(1)));
            match restored.as_slice() {
                [] => {}
                [one] => lines.push(tr!("chat.restored_one", one)),
                many => lines.push(tr!("chat.restored_many", many.len(), many.join(", "))),
            }
            Ok(lines)
        }
    }
}

fn first_read(files: &[(String, At)], read: Vec<Result<ReadFile, String>>) -> Result<(ReadFile, At), String> {
    let at = files.first().ok_or_else(|| tr!("err.no_file"))?.1;
    Ok((read.into_iter().next().ok_or_else(|| tr!("err.no_file"))??, at))
}

/// "Loaded house (12x5x9, 1 regions) at …" (or "Reloaded …") for a placement.
fn loaded_line(slot: &Slot, reloaded: bool) -> String {
    let p = &slot.placement;
    let (lo, hi) = p.bounds();
    let size = [0, 1, 2].map(|i| hi[i] - lo[i] + 1);
    let (name, regions, o) = (display_name(p), p.schematic.regions.len(), p.origin);
    if reloaded {
        tr!("chat.reloaded", name, size[0], size[1], size[2], regions, o[0], o[1], o[2])
    } else {
        tr!("chat.loaded", name, size[0], size[1], size[2], regions, o[0], o[1], o[2])
    }
}

/// Places a file that was read: a new slot, not yet added, and what `loaded_line` is followed by
/// (its format, updated states, unknown blocks).
fn place(w: &World<'_>, read: ReadFile, at: At) -> Result<(Slot, String), String> {
    let ReadFile { file, schematic, format, from_version, upgraded, histograms } = read;
    let (origin, rotation, mirror) = at;
    let placement = Placement::placed(file, schematic, origin, rotation, mirror).map_err(|_| outside_world())?;
    let (slot, unknown) = with(|s| {
        let totals = count_totals(w, s, &placement, &histograms);
        let mut unknown: Vec<String> = Vec::new();
        for st in placement.palettes().iter().flatten() {
            if !st.is_air() && !st.is_void() && wanted_id(w, s, st).is_none() && !unknown.contains(&st.name) {
                unknown.push(st.name.clone());
            }
        }
        s.next_id += 1;
        (Slot { id: s.next_id, placement, shown: true, locked: false, totals }, unknown)
    });
    let mut msg = String::new();
    if format != Format::Litematic {
        msg.push_str(&format!(" · {}", format_text(format)));
    }
    if upgraded > 0 {
        msg.push_str(&tr!("chat.upgraded", upgraded, from_version));
    }
    if !unknown.is_empty() {
        msg += &tr!("chat.unknown_blocks", unknown.join(", "));
    }
    Ok((slot, msg))
}

fn load(w: &World<'_>, query: &str, at: Option<At>) -> Result<Vec<String>, String> {
    let at = match at {
        Some(a) => a,
        None => (w.feet().ok_or_else(no_world)?, Rotation::None, Mirror::None),
    };
    start_load(w, vec![(query.to_string(), at)], Then::Load)
}

// ---------------------------------------------------------------- commands

/// Every subcommand with the words it takes, and the name shown for what it takes besides.
pub const WORDS: &[(&str, &[&str], Option<&str>)] = &[
    ("help", &[], None),
    ("list", &[], None),
    ("load", &[], Some("file")),
    ("reload", &[], None),
    ("unload", &["all"], Some("placement")),
    ("placements", &[], None),
    ("select", &["next", "prev"], Some("placement")),
    ("show", &["on", "off"], None),
    ("hide", &[], None),
    ("lock", &["on", "off"], None),
    ("unlock", &[], None),
    ("origin", &["here"], Some("x y z")),
    ("move", &[], Some("dx dy dz")),
    ("rotate", &["cw", "ccw", "180", "none"], None),
    ("mirror", &["none", "x", "z"], None),
    ("pos1", &["here"], Some("x y z")),
    ("pos2", &["here"], Some("x y z")),
    ("area", &["clear"], None),
    ("save", &["status", "cancel"], Some("name")),
    ("tool", &["on", "off", "mode"], Some("item")),
    ("layer", &["all", "up", "down", "here", "above", "below", "range", "axis", "axis x", "axis y", "axis z"], Some("coordinate")),
    ("render", &["on", "off"], None),
    ("through", &["on", "off"], None),
    ("highlight", &["missing", "wrong", "item", "off"], None),
    ("mode", &["ghost", "boxes"], None),
    ("radius", &["view"], Some("blocks")),
    ("faces", &["300k", "1m", "2m", "5m", "10m", "none"], Some("number")),
    ("fluids", &["on", "off"], None),
    ("info", &[], None),
    ("materials", &["on", "off", "copy"], None),
    ("printer", &["on", "off", "speed", "reach", "air"], None),
    ("easyplace", &["on", "off", "every", "air"], None),
    ("folder", &[], None),
    ("credits", &[], None),
];

/// `/lm help`'s lines (the commands' words stay English; what they do is translated).
fn help_lines() -> Vec<String> {
    (1..=15).map(|i| tr!(&format!("help.{i}"))).collect()
}

/// Runs `/blazematica …`; true when the command was ours.
pub fn command(w: &World<'_>, line: &str) -> bool {
    let line = line.trim_start_matches('/');
    let mut words = line.split_whitespace();
    match words.next() {
        Some("blazematica" | "lm") => {}
        _ => return false,
    }
    let args: Vec<&str> = words.collect();
    report(run(w, &args));
    true
}

fn int(s: Option<&&str>, what: &str) -> Result<i32, String> {
    s.ok_or_else(|| tr!("err.missing", what))?.parse::<i32>().map_err(|_| tr!("err.not_number", what))
}

fn on_off(s: Option<&&str>, current: bool) -> Result<bool, String> {
    match s.copied() {
        None | Some("toggle") => Ok(!current),
        Some("on" | "true" | "1") => Ok(true),
        Some("off" | "false" | "0") => Ok(false),
        Some(v) => Err(tr!("err.on_off", v)),
    }
}

fn need_placement(s: &State) -> Result<(), String> {
    if s.slots.is_empty() { Err(tr!("err.no_schematic")) } else { Ok(()) }
}

/// The selected placement, to change it.
fn selected_mut(s: &mut State) -> Result<&mut Placement, String> {
    need_placement(s)?;
    if let Some(slot) = s.sel()
        && slot.locked
    {
        return Err(tr!("err.locked", display_name(&slot.placement)));
    }
    s.placement_mut().ok_or_else(|| tr!("err.no_selected"))
}

/// A placement by its number in `/lm placements` (from 1), else by (the start of) its name.
fn find_slot(s: &State, query: &str) -> Result<usize, String> {
    if let Ok(n) = query.parse::<usize>() {
        return (1..=s.slots.len()).contains(&n).then(|| n - 1).ok_or_else(|| tr!("err.no_placement_n", n, s.slots.len()));
    }
    let q = query.to_lowercase();
    let names: Vec<(usize, String, String)> =
        s.slots.iter().enumerate().map(|(i, x)| (i, display_name(&x.placement).to_lowercase(), x.placement.file_name.to_lowercase())).collect();
    if let Some((i, ..)) = names.iter().find(|(_, n, f)| *n == q || *f == q) {
        return Ok(*i);
    }
    let hits: Vec<usize> = names.iter().filter(|(_, n, f)| n.starts_with(&q) || f.starts_with(&q)).map(|(i, ..)| *i).collect();
    match hits.as_slice() {
        [one] => Ok(*one),
        [] => Err(tr!("err.no_placement_named", query)),
        _ => Err(tr!("err.placement_ambiguous", query, hits.len())),
    }
}

/// One line per placement for `/lm placements`.
fn placement_lines(s: &State) -> Vec<String> {
    s.slots
        .iter()
        .enumerate()
        .map(|(i, slot)| {
            let p = &slot.placement;
            let flags = match (slot.shown, slot.locked) {
                (true, false) => String::new(),
                (false, false) => tr!("chat.flag_hidden"),
                (true, true) => tr!("chat.flag_locked"),
                (false, true) => tr!("chat.flag_hidden_locked"),
            };
            let line = tr!("chat.placement_line", i + 1, display_name(p), p.origin[0], p.origin[1], p.origin[2]);
            format!("{}{line}{flags}", if i == s.selected { "> " } else { "  " })
        })
        .collect()
}

/// The block the player looks at (up to 64 blocks away), else where they stand.
fn target_block(w: &World<'_>) -> Option<[i32; 3]> {
    w.ray(64.0).map(|(pos, _)| pos).or_else(|| w.feet())
}

pub fn run(w: &World<'_>, args: &[&str]) -> Result<Vec<String>, String> {
    ensure_settings(w);
    let sub = args.first().copied().unwrap_or("help");
    let rest = &args[args.len().min(1)..];
    let changed = |w: &World<'_>| save_placements(w);
    match sub {
        "help" | "?" => Ok(help_lines()),
        "credits" | "about" => Ok(crate::credits()),
        "folder" => {
            let dir = store::schematics_dir(w)?;
            open_folder(&dir);
            Ok(vec![tr!("chat.folder", dir.display())])
        }
        "list" => {
            let dir = store::schematics_dir(w)?;
            let names = blazematica_core::list_schematics(&dir);
            if names.is_empty() {
                return Ok(vec![tr!("chat.no_files", dir.display())]);
            }
            Ok(vec![tr!("chat.schematics", names.len(), names.join(", "))])
        }
        "load" => {
            if rest.is_empty() {
                return Err(tr!("err.usage_load"));
            }
            load(w, &rest.join(" "), None)
        }
        "reload" => {
            let last = store::read_value(w, "last").ok_or_else(|| tr!("err.nothing_before"))?;
            let (file, at) = parse_saved(&last).ok_or_else(|| tr!("err.saved_unreadable"))?;
            start_load(w, vec![(file, at)], Then::Reload)
        }
        "unload" | "remove" => {
            let msg = if rest.first() == Some(&"all") {
                let n = with(|s| {
                    let n = s.slots.len();
                    s.drop_all();
                    n
                });
                tr!("chat.unloaded_all", n)
            } else {
                let (name, left) = with(|s| {
                    need_placement(s)?;
                    let i = if rest.is_empty() { s.selected } else { find_slot(s, &rest.join(" "))? };
                    let slot = s.slots.remove(i);
                    if s.selected > i || s.selected >= s.slots.len() {
                        s.selected = s.selected.saturating_sub(1);
                    }
                    if s.slots.is_empty() {
                        s.drop_all();
                    } else {
                        s.moving = false;
                    }
                    Ok::<_, String>((display_name(&slot.placement), s.slots.len()))
                })?;
                if left == 0 { tr!("chat.unloaded", name) } else { tr!("chat.unloaded_left", name, left) }
            };
            if with(|s| s.slots.is_empty()) {
                crate::overlay::clear();
            }
            save_placements(w);
            Ok(vec![msg])
        }
        "placements" | "pl" => with(|s| {
            need_placement(s)?;
            let mut out = vec![tr!("chat.placements_header", s.slots.len())];
            out.extend(placement_lines(s));
            Ok(out)
        }),
        "select" | "sel" => {
            let line = with(|s| {
                need_placement(s)?;
                let n = s.slots.len();
                s.selected = match rest.first().copied() {
                    None | Some("next") => (s.selected + 1) % n,
                    Some("prev" | "previous") => (s.selected + n - 1) % n,
                    Some(_) => find_slot(s, &rest.join(" "))?,
                };
                s.moving = false;
                let p = s.placement().ok_or_else(|| tr!("err.no_selected"))?;
                Ok::<_, String>(tr!("chat.selected", s.selected + 1, n, display_name(p)))
            })?;
            save_placements(w);
            Ok(vec![line])
        }
        "lock" | "unlock" => {
            let line = with(|s| {
                need_placement(s)?;
                let i = s.selected;
                let slot = &mut s.slots[i];
                slot.locked = if sub == "unlock" { false } else { on_off(rest.first(), slot.locked)? };
                let locked = slot.locked;
                if locked {
                    s.moving = false;
                }
                let name = display_name(&s.slots[i].placement);
                Ok::<_, String>(if locked {
                    tr!("chat.locked", name)
                } else {
                    tr!("chat.unlocked", name)
                })
            })?;
            save_placements(w);
            Ok(vec![line])
        }
        "show" | "hide" => {
            let line = with(|s| {
                need_placement(s)?;
                let i = s.selected;
                let slot = &mut s.slots[i];
                slot.shown = if sub == "hide" { false } else { on_off(rest.first(), slot.shown)? };
                if !slot.shown {
                    s.moving = false;
                }
                let slot = &s.slots[i];
                let name = display_name(&slot.placement);
                Ok::<_, String>(if slot.shown { tr!("chat.shown", name) } else { tr!("chat.hidden", name) })
            })?;
            save_placements(w);
            Ok(vec![line])
        }
        "pos1" | "pos2" | "corner1" | "corner2" => {
            let at = match rest.first().copied() {
                None => target_block(w).ok_or_else(no_world)?,
                Some("here") => w.feet().ok_or_else(no_world)?,
                _ => [int(rest.first(), "x")?, int(rest.get(1), "y")?, int(rest.get(2), "z")?],
            };
            let at = check_corner(at)?;
            let i = usize::from(sub.ends_with('2'));
            let size = with(|s| {
                s.area[i] = Some(at);
                s.area_size()
            });
            let mut line = tr!("chat.corner_at", i + 1, at[0], at[1], at[2]);
            if let Some(d) = size {
                line += &tr!("chat.corner_area", d[0], d[1], d[2]);
            }
            Ok(vec![line])
        }
        "area" | "selection" => {
            if matches!(rest.first().copied(), Some("clear" | "none" | "off")) {
                with(|s| s.area = [None; 2]);
                return Ok(vec![tr!("chat.area_cleared")]);
            }
            let (area, size) = with(|s| (s.area, s.area_size()));
            let corner = |c: Option<[i32; 3]>| c.map_or_else(|| tr!("not_set"), |c| format!("{} {} {}", c[0], c[1], c[2]));
            let mut line = tr!("chat.area", corner(area[0]), corner(area[1]));
            if let Some(d) = size {
                line += &tr!("chat.area_volume", d[0], d[1], d[2], volume_text(d));
            }
            Ok(vec![line])
        }
        "tool" => {
            let line = with(|s| -> Result<String, String> {
                let st = &mut s.settings;
                match rest.first().copied() {
                    None => {}
                    Some("mode") => {
                        st.tool_placement = match rest.get(1).copied() {
                            Some("area" | "selection") => false,
                            Some("placement" | "placements" | "move") => true,
                            None | Some("toggle") => !st.tool_placement,
                            Some(v) => return Err(tr!("err.tool_mode", v)),
                        };
                        st.tool = true;
                        return Ok(if st.tool_placement { tr!("chat.tool_placement_mode") } else { tr!("chat.tool_area_mode") });
                    }
                    Some("on" | "off" | "toggle") => st.tool = on_off(rest.first(), st.tool)?,
                    Some(item) => {
                        let item = item.to_ascii_lowercase();
                        st.tool_item = if item.contains(':') { item } else { format!("minecraft:{item}") };
                        st.tool = true;
                    }
                }
                Ok(if st.tool {
                    tr!("chat.tool_on", item_name(&st.tool_item))
                } else {
                    tr!("chat.tool_off")
                })
            })?;
            save_settings(w);
            Ok(vec![line])
        }
        "paste" => Ok(vec![match rest {
            ["cancel" | "stop"] => cancel_paste()?,
            ["status"] => paste_status()?,
            [] => start_paste()?,
            _ => return Err(tr!("err.paste_usage")),
        }]),
        "hover" => {
            let on = match rest {
                [] => with(|s| s.no_hover),
                ["on"] => true,
                ["off"] => false,
                _ => return Err(tr!("err.hover_usage")),
            };
            with(|s| s.no_hover = !on);
            Ok(vec![if on { tr!("chat.hover_on") } else { tr!("chat.hover_off") }])
        }
        "save" => {
            match rest {
                ["cancel" | "stop"] => return Ok(vec![cancel_save()?]),
                ["status"] => {
                    let lines = with(|s| save_lines(w, s)).map(|(l, _)| l.into_iter().map(|(t, _)| t).collect());
                    return Ok(lines.unwrap_or_else(|| vec![tr!("chat.no_save")]));
                }
                _ => {}
            }
            let overwrite = rest.last() == Some(&"overwrite") && rest.len() > 1;
            let name = rest[..rest.len() - usize::from(overwrite)].join(" ");
            if name.is_empty() {
                return Err(tr!("err.usage_save"));
            }
            Ok(vec![start_save(w, &name, overwrite)?])
        }
        "info" | "verify" => with(|s| {
            need_placement(s)?;
            let p = s.placement().ok_or_else(|| tr!("err.no_selected"))?;
            let (lo, hi) = p.bounds();
            let mut out = vec![tr!(
                "chat.info",
                p.file_name,
                s.selected + 1,
                s.slots.len(),
                p.origin[0],
                p.origin[1],
                p.origin[2],
                rotation_text(p.rotation),
                mirror_text(p.mirror),
                format!("{lo:?}"),
                format!("{hi:?}"),
                layer_text(s.layer)
            )];
            match s.tally() {
                Some(t) => out.push(tr!(
                    "chat.tally",
                    format!("{:.1}", t.percent()),
                    t.correct,
                    t.missing,
                    t.wrong_block,
                    t.wrong_state,
                    t.extra,
                    if s.unloaded > 0 { tr!("chat.tally_unloaded", s.unloaded) } else { String::new() }
                )),
                None => out.push(tr!("chat.scanning")),
            }
            Ok(out)
        }),
        "origin" => {
            let at = match rest.first().copied() {
                Some("here") | None => w.feet().ok_or_else(no_world)?,
                _ => [int(rest.first(), "x")?, int(rest.get(1), "y")?, int(rest.get(2), "z")?],
            };
            with(|s| selected_mut(s)?.set_origin(at).map_err(|_| outside_world()))?;
            changed(w);
            Ok(vec![tr!("chat.origin_set", at[0], at[1], at[2])])
        }
        "move" | "nudge" => {
            let d = [int(rest.first(), "dx")?, int(rest.get(1), "dy")?, int(rest.get(2), "dz")?];
            let o = with(|s| {
                let p = selected_mut(s)?;
                p.nudge(d).map_err(|_| outside_world())?;
                Ok::<_, String>(p.origin)
            })?;
            changed(w);
            Ok(vec![tr!("chat.origin_now", o[0], o[1], o[2])])
        }
        "rotate" => {
            let r = with(|s| {
                let p = selected_mut(s)?;
                let r = match rest.first().copied() {
                    None | Some("cw") => p.rotation.then(Rotation::Clockwise90),
                    Some("ccw") => p.rotation.then(Rotation::CounterClockwise90),
                    Some(v) => Rotation::parse(v).ok_or_else(|| tr!("err.rotate"))?,
                };
                p.set_rotation(r).map_err(|_| outside_world())?;
                Ok::<_, String>(r)
            })?;
            changed(w);
            Ok(vec![tr!("chat.rotation", rotation_text(r))])
        }
        "mirror" => {
            let m = Mirror::parse(rest.first().copied().unwrap_or("none")).ok_or_else(|| tr!("err.mirror"))?;
            with(|s| selected_mut(s)?.set_mirror(m).map_err(|_| outside_world()))?;
            changed(w);
            Ok(vec![tr!("chat.mirror", mirror_text(m))])
        }
        "layer" if rest.first() == Some(&"axis") => {
            let [_, name] = rest else { return Err("Usage: /lm layer axis x|y|z".into()) };
            let axis = LayerAxis::parse(name).ok_or("Usage: /lm layer axis x|y|z")?;
            let layer = with(|s| {
                s.layer.axis = axis;
                s.layer
            });
            Ok(vec![tr!("chat.layers", layer_text(layer))])
        }
        "layer" => {
            let feet = w.feet().unwrap_or([0, 64, 0]);
            let l = with(|s| {
                need_placement(s)?;
                let axis = s.layer.axis;
                let base = axis.coord(s.placement().ok_or_else(|| tr!("err.no_selected"))?.bounds().0);
                let current = match s.layer.mode {
                    LayerMode::All => base,
                    LayerMode::Single(n) | LayerMode::Above(n) | LayerMode::Below(n) | LayerMode::Range(n, _) => n,
                };
                s.layer.mode = match rest.first().copied() {
                    None | Some("all") => LayerMode::All,
                    Some("up") => match s.layer.mode {
                        LayerMode::All => LayerMode::Single(base),
                        l => l.shifted(1),
                    },
                    Some("down") => match s.layer.mode {
                        LayerMode::All => LayerMode::Single(base),
                        l => l.shifted(-1),
                    },
                    Some("here") => LayerMode::Single(axis.coord(feet)),
                    Some("above") => LayerMode::Above(rest.get(1).map_or(Ok(current), |_| int(rest.get(1), axis.name()))?),
                    Some("below") => LayerMode::Below(rest.get(1).map_or(Ok(current), |_| int(rest.get(1), axis.name()))?),
                    Some("range") => LayerMode::Range(int(rest.get(1), axis.name())?, int(rest.get(2), axis.name())?),
                    Some(_) => LayerMode::Single(int(rest.first(), axis.name())?),
                };
                Ok::<_, String>(s.layer)
            })?;
            Ok(vec![tr!("chat.layers", layer_text(l))])
        }
        "render" | "mode" | "through" | "xray" | "radius" | "faces" | "fluids" => {
            let view = w.host.graphics_options().render_distance as i32 * 16;
            let msg = with(|s| {
                let st = &mut s.settings;
                let msg = match sub {
                    "radius" => {
                        match rest.first().copied() {
                            Some("view") => st.radius_view = true,
                            Some(v) => {
                                st.radius = v.parse::<i32>().map_err(|_| tr!("err.radius", v))?.max(16);
                                st.radius_view = false;
                            }
                            None => {}
                        }
                        if st.radius_view {
                            tr!("chat.radius_view", view)
                        } else {
                            tr!("chat.radius", st.radius)
                        }
                    }
                    "faces" => {
                        if let Some(v) = rest.first() {
                            st.max_faces = settings::parse_faces(v).ok_or_else(|| tr!("err.faces", v))?;
                        }
                        tr!("chat.faces", settings::faces_text(st.max_faces))
                    }
                    "fluids" => {
                        st.hide_fluids = !on_off(rest.first(), !st.hide_fluids)?;
                        if st.hide_fluids { tr!("chat.fluids_hidden") } else { tr!("chat.fluids_shown") }
                    }
                    "render" => {
                        st.render = on_off(rest.first(), st.render)?;
                        tr!("chat.rendering", on_off_text(st.render))
                    }
                    "through" | "xray" => {
                        st.through = on_off(rest.first(), st.through)?;
                        tr!("chat.through", on_off_text(st.through))
                    }
                    _ => {
                        st.mode = match rest.first().copied() {
                            Some("ghost" | "ghosts") => Mode::Ghost,
                            Some("box" | "boxes") => Mode::Boxes,
                            None => {
                                if st.mode == Mode::Ghost {
                                    Mode::Boxes
                                } else {
                                    Mode::Ghost
                                }
                            }
                            Some(v) => return Err(tr!("err.mode", v)),
                        };
                        tr!("chat.mode", if st.mode == Mode::Ghost { tr!("setting.mode.ghost") } else { tr!("setting.mode.boxes") })
                    }
                };
                Ok::<_, String>(msg)
            })?;
            save_settings(w);
            Ok(vec![msg])
        }
        "highlight" | "hl" => {
            let list = material_list(w);
            let line = with(|s| -> Result<String, String> {
                match rest.first().copied() {
                    None => {}
                    Some("off") => {
                        s.settings.highlight_missing = false;
                        s.settings.highlight_wrong = false;
                        s.highlight_item = None;
                    }
                    Some("missing") => s.settings.highlight_missing = on_off(rest.get(1), s.settings.highlight_missing)?,
                    Some("wrong") => s.settings.highlight_wrong = on_off(rest.get(1), s.settings.highlight_wrong)?,
                    Some("item") => match rest[1..].join("_").to_lowercase().as_str() {
                        "" | "all" | "none" => s.highlight_item = None,
                        name => {
                            let item = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
                            if !list.iter().any(|m| m.item == item) {
                                return Err(tr!("err.not_in_list", item_name(&item)));
                            }
                            s.highlight_item = Some(item);
                            if !s.settings.highlight_wrong {
                                s.settings.highlight_missing = true;
                            }
                        }
                    },
                    Some(v) => return Err(tr!("err.highlight", v)),
                }
                Ok(highlight_text(s))
            })?;
            save_settings(w);
            Ok(vec![line])
        }
        "materials" | "mats" => {
            if rest.first() == Some(&"copy") {
                return copy_materials(w);
            }
            let line = with(|s| -> Result<String, String> {
                s.settings.materials_hud = on_off(rest.first(), s.settings.materials_hud)?;
                Ok(tr!("chat.materials_hud", on_off_text(s.settings.materials_hud)))
            })?;
            save_settings(w);
            Ok(vec![line])
        }
        "printer" | "print" => {
            let msg = with(|s| {
                let st = &mut s.settings;
                let msg = match rest.first().copied() {
                    Some("speed") => {
                        st.printer.interval = int(rest.get(1), &tr!("word.ticks"))?.clamp(1, 20) as u32;
                        tr!("chat.printer_speed", st.printer.interval)
                    }
                    Some("reach") => {
                        let reach = tr!("word.reach");
                        let r: f64 = rest.get(1).ok_or_else(|| tr!("err.missing", reach))?.parse().map_err(|_| tr!("err.not_number", reach))?;
                        st.printer.reach = r.clamp(1.0, 6.0);
                        tr!("chat.printer_reach", format!("{:.1}", st.printer.reach))
                    }
                    Some("air") => {
                        st.printer.air_place = on_off(rest.get(1), st.printer.air_place)?;
                        if st.printer.air_place { tr!("chat.printer_air_on") } else { tr!("chat.printer_air_off") }
                    }
                    v => {
                        st.printer_on = on_off(v.as_ref(), st.printer_on)?;
                        s.recent.clear();
                        if s.settings.printer_on && crate::hands::multiplayer(w.host) {
                            tr!("chat.printer_single_player")
                        } else if s.settings.printer_on {
                            tr!("chat.printer_on")
                        } else {
                            tr!("chat.printer_off")
                        }
                    }
                };
                Ok::<_, String>(msg)
            })?;
            save_settings(w);
            Ok(vec![msg])
        }
        "easyplace" | "easy" => {
            let msg = with(|s| {
                let st = &mut s.settings;
                let msg = match rest.first().copied() {
                    Some("every") => {
                        st.easy_interval = int(rest.get(1), &tr!("word.ticks"))?.clamp(1, 10) as u32;
                        tr!("chat.easy_speed", st.easy_interval)
                    }
                    Some("air") => {
                        st.easy_air = on_off(rest.get(1), st.easy_air)?;
                        if st.easy_air { tr!("chat.easy_air_on") } else { tr!("chat.easy_air_off") }
                    }
                    v => {
                        st.easy_place = on_off(v.as_ref(), st.easy_place)?;
                        if st.easy_place && crate::hands::multiplayer(w.host) {
                            tr!("chat.easy_single_player")
                        } else if st.easy_place {
                            tr!("chat.easy_on")
                        } else {
                            tr!("chat.easy_off")
                        }
                    }
                };
                Ok::<_, String>(msg)
            })?;
            save_settings(w);
            Ok(vec![msg])
        }
        other => Err(tr!("err.unknown_subcommand", other)),
    }
}

/// Opens a folder in the system's file manager.
fn open_folder(dir: &std::path::Path) {
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let program = "xdg-open";
    let _ = std::process::Command::new(program).arg(dir).spawn();
}

// ---------------------------------------------------------------- keys

/// Everything a key can do: (id, label, default key). Registered as the game's key mappings
/// (Options > Controls > Blazematica); the id is the action's (`action`).
pub const BINDS: &[(&str, &str, &str)] = &[
    ("open_menu", "Open the Blazematica menu", "key.keyboard.m"),
    ("toggle_render", "Toggle schematic rendering", "key.keyboard.v"),
    ("layer_up", "Layer up", "key.keyboard.page.up"),
    ("layer_down", "Layer down", "key.keyboard.page.down"),
    ("layer_mode", "Cycle layer mode", "key.keyboard.home"),
    ("toggle_printer", "Printer on/off", "key.keyboard.k"),
    ("toggle_easy_place", "Easy place on/off", "key.keyboard.unknown"),
    ("toggle_materials", "Material list on/off", "key.keyboard.j"),
    ("move_mode", "Move mode", "key.keyboard.n"),
    ("toggle_tool", "Selection tool on/off", "key.keyboard.unknown"),
    ("reload", "Reload the last schematic", "key.keyboard.unknown"),
    ("rotate_cw", "Rotate clockwise", "key.keyboard.unknown"),
    ("rotate_ccw", "Rotate counter-clockwise", "key.keyboard.unknown"),
    ("mirror_next", "Mirror (next axis)", "key.keyboard.unknown"),
    ("here", "Move the placement to me", "key.keyboard.unknown"),
    ("show_toggle", "Show or hide the placement", "key.keyboard.unknown"),
    ("select_next", "Select next placement", "key.keyboard.unknown"),
    ("select_prev", "Select previous placement", "key.keyboard.unknown"),
    ("unload", "Unload this placement", "key.keyboard.unknown"),
    ("unload_all", "Unload all placements", "key.keyboard.unknown"),
    ("area_pos1", "Area corner 1 at my position", "key.keyboard.unknown"),
    ("area_pos2", "Area corner 2 at my position", "key.keyboard.unknown"),
    ("area_clear", "Clear the area", "key.keyboard.unknown"),
    ("materials_copy", "Copy the material list", "key.keyboard.unknown"),
    ("folder", "Open the schematics folder", "key.keyboard.unknown"),
];

/// The next layer mode (all, this layer, above, below), as the key cycles them.
pub fn next_layer_mode() -> &'static str {
    with(|s| match s.layer.mode {
        LayerMode::All => "here",
        LayerMode::Single(_) => "above",
        LayerMode::Above(_) => "below",
        _ => "all",
    })
}

/// A key's or a menu row's action.
pub fn action(w: &World<'_>, action: &str) {
    let r = match action {
        "move_mode" => {
            toggle_move_mode(w);
            return;
        }
        "credits_chat" => {
            for line in crate::credits() {
                with(|s| s.out.push(line));
            }
            return;
        }
        "toggle_materials" => {
            // The HUD list only: don't fill the chat each press.
            with(|s| s.settings.materials_hud = !s.settings.materials_hud);
            save_settings(w);
            return;
        }
        "mirror_next" => {
            let next = with(|s| match s.placement().map(|p| p.mirror) {
                // x flips x (FRONT_BACK), z flips z (LEFT_RIGHT).
                Some(Mirror::None) => "x",
                Some(Mirror::FrontBack) => "z",
                _ => "none",
            });
            run(w, &["mirror", next])
        }
        "layer_next" | "layer_mode" => {
            let next = next_layer_mode();
            run(w, &["layer", next])
        }
        "folder" => run(w, &["folder"]),
        "reload" => run(w, &["reload"]),
        "here" => run(w, &["origin", "here"]),
        "rotate_cw" => run(w, &["rotate", "cw"]),
        "rotate_ccw" => run(w, &["rotate", "ccw"]),
        "unload" => run(w, &["unload"]),
        "unload_all" => run(w, &["unload", "all"]),
        "select_next" => run(w, &["select", "next"]),
        "show_toggle" => run(w, &["show", "toggle"]),
        "printer_toggle" | "toggle_printer" => run(w, &["printer"]),
        "toggle_render" => run(w, &["render"]),
        "layer_up" => run(w, &["layer", "up"]),
        "layer_down" => run(w, &["layer", "down"]),
        "toggle_easy_place" => run(w, &["easyplace"]),
        "toggle_tool" => {
            let on = with(|s| s.settings.tool);
            run(w, &["tool", if on { "off" } else { "on" }])
        }
        "select_prev" => run(w, &["select", "prev"]),
        "area_pos1" => run(w, &["pos1", "here"]),
        "area_pos2" => run(w, &["pos2", "here"]),
        "area_clear" => run(w, &["area", "clear"]),
        "materials" | "materials_copy" => run(w, &["materials", "copy"]),
        other => match other.strip_prefix("load:") {
            Some(file) => run(w, &["load", file]),
            None => return,
        },
    };
    report(r);
}

/// The registered keys pressed since the last tick, in order (move mode takes Page up/down and
/// Home for itself).
pub fn key_clicked(w: &World<'_>, id: &str) {
    if with(|s| s.moving) && matches!(id, "layer_up" | "layer_down" | "layer_mode") {
        return;
    }
    if id == "open_menu" {
        crate::menu::open(w);
        return;
    }
    action(w, id);
}

// ---------------------------------------------------------------- move mode

pub fn toggle_move_mode(w: &World<'_>) {
    if with(|s| !s.moving && s.sel().is_some_and(|x| x.locked)) {
        say(&tr!("chat.move_locked"));
        return;
    }
    let on = with(|s| {
        s.placement()?;
        s.moving = !s.moving;
        s.wheel = 0.0;
        Some(s.moving)
    });
    match on {
        None => say(&tr!("chat.move_none")),
        Some(true) => say(&tr!("chat.move_on")),
        Some(false) => {
            save_placements(w);
            say(&tr!("chat.move_off"))
        }
    }
}

/// GLFW's modifier bits, as the input hooks give them.
const SHIFT: u32 = 1;
const CONTROL: u32 = 2;
const ALT: u32 = 4;

/// Blocks per step: 5 with Shift held.
fn move_step(modifiers: u32) -> i32 {
    if modifiers & SHIFT != 0 { 5 } else { 1 }
}

/// Minecraft's view vector for a yaw and pitch in degrees.
pub fn view_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    let (yaw, pitch) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()]
}

/// The unit axis closest to `dir`.
fn dominant(dir: [f64; 3]) -> [i32; 3] {
    let i = (0..3).max_by(|&a, &b| dir[a].abs().total_cmp(&dir[b].abs())).unwrap_or(0);
    let mut out = [0; 3];
    out[i] = if dir[i] < 0.0 { -1 } else { 1 };
    out
}

/// Moves the placement by `d` without a chat line (the HUD shows the origin). False when it is
/// locked.
fn shift(d: [i32; 3]) -> bool {
    with(|s| {
        if s.sel().is_some_and(|x| x.locked) {
            return false;
        }
        // At the edge of the world it stays put.
        if s.placement_mut().is_some_and(|p| p.nudge(d).is_ok()) {
            s.scan = Scan::default();
        }
        true
    })
}

/// A key pressed while playing (`input_key`): move mode's keys.
pub fn raw_key(w: &World<'_>, key: &str, modifiers: u32) {
    if !with(|s| s.moving) {
        return;
    }
    let k = move_step(modifiers);
    let Some((yaw, _)) = w.look() else { return };
    let view = view_vector(yaw, 0.0);
    let fwd = dominant([view[0], 0.0, view[2]]);
    let right = [-fwd[2], 0, fwd[0]];
    let d = match key {
        "key.keyboard.up" => fwd,
        "key.keyboard.down" => fwd.map(|v| -v),
        "key.keyboard.right" => right,
        "key.keyboard.left" => right.map(|v| -v),
        "key.keyboard.page.up" => [0, 1, 0],
        "key.keyboard.page.down" => [0, -1, 0],
        "key.keyboard.r" => {
            report(run(w, &["rotate", "cw"]).map(|_| Vec::new()));
            return;
        }
        "key.keyboard.home" => {
            report(run(w, &["origin", "here"]).map(|_| Vec::new()));
            return;
        }
        "key.keyboard.enter" | "key.keyboard.keypad.enter" => {
            toggle_move_mode(w);
            return;
        }
        _ => return,
    };
    if !shift(d.map(|v| v * k)) {
        say(&tr!("chat.placement_locked"));
    }
}

/// The mouse wheel while playing: in move mode it moves the placement along the axis you look
/// along; with the selection tool and Alt or Ctrl, it moves the area or the placement. Whether
/// the wheel was taken (the hotbar stays put).
pub fn wheel(w: &World<'_>, steps: f32, modifiers: u32) -> bool {
    if !with(|s| s.moving && s.placement().is_some()) {
        return tool_wheel(w, steps, modifiers);
    }
    let n = wheel_steps(steps);
    if n == 0 {
        return true;
    }
    let Some((yaw, pitch)) = w.look() else { return true };
    let axis = dominant(view_vector(yaw, pitch));
    let k = n.saturating_mul(move_step(modifiers));
    if !shift(axis.map(|v| v.saturating_mul(k))) {
        say(&tr!("chat.placement_locked"));
    }
    true
}

/// Whole wheel steps (touchpads send fractions; the rest is kept).
fn wheel_steps(steps: f32) -> i32 {
    with(|s| {
        s.wheel += steps;
        let n = s.wheel.trunc();
        s.wheel -= n;
        n as i32
    })
}

// ---------------------------------------------------------------- the selection tool

/// How far middle click looks for a corner.
const PICK_RANGE: f64 = 512.0;

/// Whether the selection tool is in the main hand (and switched on).
pub fn tool_held(w: &World<'_>) -> bool {
    let item = with(|s| s.settings.tool.then(|| s.settings.tool_item.clone()));
    item.is_some_and(|item| w.held().is_some_and(|h| h == item))
}

/// The face of the block `pos` that the point `hit` lies on.
fn face_of(pos: [i32; 3], hit: [f64; 3]) -> [i32; 3] {
    let mut best = (f64::MAX, [0, 1, 0]);
    for i in 0..3 {
        let lo = (hit[i] - f64::from(pos[i])).abs();
        let hi = (hit[i] - f64::from(pos[i] + 1)).abs();
        let mut f = [0; 3];
        if lo < best.0 {
            f[i] = -1;
            best = (lo, f);
        }
        let mut f = [0; 3];
        if hi < best.0 {
            f[i] = 1;
            best = (hi, f);
        }
    }
    best.1
}

/// Where a tool click puts a corner: the first block along the view within the tool's reach
/// (water and lava count when the setting says so), or the block in front of the face hit.
fn tool_target(w: &World<'_>) -> Option<[i32; 3]> {
    let (range, front, fluids) = with(|s| (s.settings.tool_range, s.settings.tool_front, s.settings.tool_fluids));
    let (origin, dir) = w.aim()?;
    let hit = w.ray(range).map(|(pos, point)| {
        let length = (0..3).map(|i| (point[i] - origin[i]).powi(2)).sum::<f64>().sqrt();
        let face = face_of(pos, point);
        (pos, [0, 1, 2].map(|i| pos[i] + face[i]), length)
    });
    let hit = match fluids.then(|| first_fluid(w, origin, dir, hit.map_or(range, |h| h.2))).flatten() {
        Some((cell, before)) => (cell, before),
        None => hit.map(|h| (h.0, h.1))?,
    };
    Some(if front { hit.1 } else { hit.0 })
}

/// The first water or lava cell along the ray within `reach`, and the cell before it.
fn first_fluid(w: &World<'_>, origin: [f64; 3], dir: [f64; 3], reach: f64) -> Option<([i32; 3], [i32; 3])> {
    let cell_at = |t: f64| [0, 1, 2].map(|i| (origin[i] + dir[i] * t).floor() as i32);
    let mut last = cell_at(0.0);
    let mut t = 0.0;
    while t < reach {
        t += 0.05;
        let cell = cell_at(t);
        if cell == last {
            continue;
        }
        if let Some(id) = w.block(cell) {
            let fluid = with(|s| {
                let st = s.blocks.state(w, id);
                fluid_of(&st) != 0 || st.prop("waterlogged") == Some("true")
            });
            if fluid {
                return Some((cell, last));
            }
        }
        last = cell;
    }
    None
}

/// Where the ray from `o` along `d` enters the box `lo..hi`, if it does.
fn ray_box(o: [f64; 3], d: [f64; 3], lo: [f64; 3], hi: [f64; 3]) -> Option<f64> {
    let (mut near, mut far) = (0.0f64, PICK_RANGE);
    for i in 0..3 {
        if d[i].abs() < 1e-9 {
            if o[i] < lo[i] || o[i] > hi[i] {
                return None;
            }
            continue;
        }
        let (a, b) = ((lo[i] - o[i]) / d[i], (hi[i] - o[i]) / d[i]);
        near = near.max(a.min(b));
        far = far.min(a.max(b));
        if near > far {
            return None;
        }
    }
    Some(near)
}

/// A mouse button while playing (GLFW: 0 left, 1 right, 2 middle). With the selection tool in
/// hand: left click puts corner 1 on the block looked at, right click corner 2, middle click
/// picks what the wheel moves. The game doesn't see these clicks. Whether the click was taken.
pub fn mouse(w: &World<'_>, button: i32, pressed: bool, modifiers: u32) -> bool {
    let Ok(b) = usize::try_from(button) else { return false };
    if b > 2 {
        return false;
    }
    // A release goes where its press went.
    if !pressed {
        return with(|s| std::mem::take(&mut s.tool_down[b]));
    }
    if !tool_held(w) {
        return false;
    }
    with(|s| s.tool_down[b] = true);
    // Ctrl + middle click: the other mode.
    if b == 2 && modifiers & CONTROL != 0 {
        report(run(w, &["tool", "mode", "toggle"]));
        return true;
    }
    if with(|s| s.settings.tool_placement) {
        placement_click(w, b);
        return true;
    }
    if b < 2 {
        if let Some(at) = tool_target(w) {
            with(|s| {
                s.area[b] = Some(blazematica_core::area::clamp_corner(at));
                s.grab = Grab::Corner(b);
            });
        }
        return true;
    }
    let Some((origin, dir)) = w.aim() else { return true };
    with(|s| {
        let corner = |i: usize| {
            let c = s.area[i]?.map(f64::from);
            ray_box(origin, dir, c.map(|v| v - 0.05), c.map(|v| v + 1.05)).map(|t| (t, i))
        };
        let looked = [corner(0), corner(1)].into_iter().flatten().min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, i)| i);
        s.grab = match looked {
            // The corner already picked, picked again: back to the box.
            Some(i) if s.grab != Grab::Corner(i) => Grab::Corner(i),
            _ => Grab::Box,
        };
    });
    true
}

/// The placement looked at: the nearest box the view enters (inside two, the smaller).
fn looked_placement(w: &World<'_>) -> Option<usize> {
    let (origin, dir) = w.aim()?;
    with(|s| {
        s.slots
            .iter()
            .enumerate()
            .filter(|(_, x)| x.shown)
            .filter_map(|(i, x)| {
                let (lo, hi) = x.placement.bounds();
                let t = ray_box(origin, dir, lo.map(f64::from), hi.map(|v| f64::from(v) + 1.0))?;
                let volume: i64 = (0..3).map(|k| i64::from(hi[k] - lo[k] + 1)).product();
                Some((t, volume, i))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
            .map(|(.., i)| i)
    })
}

/// A click with the tool in placement mode: left (or middle) selects the placement looked at,
/// right puts the selected one's origin on the block looked at.
fn placement_click(w: &World<'_>, b: usize) {
    if b == 1 {
        let Some(at) = tool_target(w) else { return };
        report(run(w, &["origin", &at[0].to_string(), &at[1].to_string(), &at[2].to_string()]).map(|_| Vec::new()));
        return;
    }
    match looked_placement(w) {
        Some(i) if with(|s| s.selected != i) => report(run(w, &["select", &(i + 1).to_string()]).map(|_| Vec::new())),
        Some(_) => {}
        None => say(&tr!("chat.no_placement_there")),
    }
}

/// The wheel with the tool in placement mode: Alt + wheel moves the selected placement along
/// the axis you look along, Ctrl + wheel turns it (up: clockwise). Shift: 5 blocks.
fn placement_wheel(w: &World<'_>, steps: f32, modifiers: u32) -> bool {
    let (alt, ctrl) = (modifiers & ALT != 0, modifiers & CONTROL != 0);
    if !(alt || ctrl) || with(|s| s.placement().is_none()) {
        return false;
    }
    let n = wheel_steps(steps);
    if n == 0 {
        return true;
    }
    if ctrl {
        for _ in 0..n.unsigned_abs() {
            if let Err(e) = run(w, &["rotate", if n > 0 { "cw" } else { "ccw" }]) {
                say(&tr!("error", e));
                break;
            }
        }
        return true;
    }
    let Some((yaw, pitch)) = w.look() else { return true };
    let axis = dominant(view_vector(yaw, pitch));
    let k = n.saturating_mul(move_step(modifiers));
    if shift(axis.map(|v| v.saturating_mul(k))) {
        save_placements(w);
    } else {
        say(&tr!("chat.placement_locked"));
    }
    true
}

/// The wheel with the selection tool in hand: Alt + wheel moves the picked corner (or the whole
/// box) along the axis you look along, Ctrl + wheel pushes the box's face in that direction out
/// (wheel up) or in (wheel down). Shift: 5 blocks. Without Alt or Ctrl the wheel is the hotbar's.
fn tool_wheel(w: &World<'_>, steps: f32, modifiers: u32) -> bool {
    let (alt, ctrl) = (modifiers & ALT != 0, modifiers & CONTROL != 0);
    if !(alt || ctrl) || !tool_held(w) {
        return false;
    }
    if with(|s| s.settings.tool_placement) {
        return placement_wheel(w, steps, modifiers);
    }
    if with(|s| s.area.iter().all(Option::is_none)) {
        return false;
    }
    let n = wheel_steps(steps);
    if n == 0 {
        return true;
    }
    let Some((yaw, pitch)) = w.look() else { return true };
    let axis = dominant(view_vector(yaw, pitch));
    let k = n.saturating_mul(move_step(modifiers));
    with(|s| {
        if ctrl {
            grow(s, axis, k);
        } else {
            let d = axis.map(|a| a.saturating_mul(k));
            let moved = |c: Option<[i32; 3]>| c.map(|c| blazematica_core::area::offset_corner(c, d));
            match s.grab {
                Grab::Corner(i) => s.area[i] = moved(s.area[i]),
                Grab::Box => s.area = s.area.map(moved),
            }
        }
    });
    true
}

/// Pushes the face of the box that `axis` points at by `k` blocks: out when `k` is positive, in
/// when negative (never past the opposite face).
fn grow(s: &mut State, axis: [i32; 3], k: i32) {
    let [Some(a), Some(b)] = s.area else { return };
    let i = (0..3).find(|&i| axis[i] != 0).unwrap_or(0);
    let sign = axis[i];
    let f = if (a[i] - b[i]) * sign > 0 || (a[i] == b[i] && sign > 0) { 0 } else { 1 };
    let (c, other) = if f == 0 { (a, b[i]) } else { (b, a[i]) };
    let mut d = [0; 3];
    d[i] = sign.saturating_mul(k);
    let mut c = blazematica_core::area::offset_corner(c, d);
    if (i64::from(c[i]) - i64::from(other)) * i64::from(sign) < 0 {
        c[i] = other;
    }
    s.area[f] = Some(c);
}

/// The HUD's lines while the selection tool is in hand.
fn tool_lines(s: &State) -> Vec<(String, u32)> {
    if s.settings.tool_placement {
        let selected = match s.sel() {
            Some(slot) => {
                let p = &slot.placement;
                let locked = if slot.locked { tr!("hud.tool_locked") } else { String::new() };
                tr!("hud.tool_selected", display_name(p), p.origin[0], p.origin[1], p.origin[2], rotation_text(p.rotation), locked)
            }
            None => tr!("hud.tool_none"),
        };
        return vec![(tr!("hud.tool_placement_1"), 0x55FFFF), (selected, 0xE0E0E0), (tr!("hud.tool_placement_2"), 0xAAAAAA)];
    }
    let c = |i: usize| s.area[i].map_or_else(|| tr!("not_set"), |c| format!("{} {} {}", c[0], c[1], c[2]));
    let mut corners = tr!("hud.tool_corners", c(0), c(1));
    if let Some(d) = s.area_size() {
        corners += &format!(" · {} x {} x {}", d[0], d[1], d[2]);
    }
    let what = match s.grab {
        Grab::Box => tr!("hud.tool_the_box"),
        Grab::Corner(i) => tr!("hud.tool_corner_n", i + 1),
    };
    vec![(tr!("hud.tool_area_1"), 0xFFFF55), (corners, 0xE0E0E0), (tr!("hud.tool_area_2", what), 0xAAAAAA)]
}

// ---------------------------------------------------------------- saving an area

/// Most positions one save takes: as many as a file may hold to be loaded again (2^31).
const MAX_SAVE: u64 = blazematica_core::schematic::MAX_VOLUME;
/// Time a save may take per tick.
const SAVE_BUDGET: std::time::Duration = std::time::Duration::from_millis(4);

/// A save under way: the area is read chunk by chunk as the chunks are loaded (as Litematica's
/// task does), a few per tick; each chunk as it was when it was read.
pub struct SaveJob {
    name: String,
    file: String,
    path: std::path::PathBuf,
    pub lo: [i32; 3],
    pub hi: [i32; 3],
    builder: blazematica_core::schematic::RegionBuilder,
    /// Game state → palette index.
    ids: HashMap<StateId, u32>,
    /// Chunk columns not read yet.
    pending: std::collections::BTreeSet<(i32, i32)>,
    total: usize,
    /// Of `pending`, those not loaded, nearest first (updated every tick).
    pub unloaded: Vec<(i32, i32)>,
    started: std::time::Instant,
    /// The player asked to replace an existing file (`/lm save <name> overwrite`, or the menu's
    /// Overwrite): without it, a file there when the save ends is kept.
    overwrite: bool,
    /// The world (`world_key`) the area is in.
    world: Option<String>,
}

/// A file being written by its own thread: its name and the line it reports when done.
struct SaveWrite {
    file: String,
    done: std::sync::mpsc::Receiver<String>,
}

/// The file name (without `.litematic`) for a schematic called `name`: characters no file system
/// takes become `_`.
pub fn file_stem(name: &str) -> Result<String, String> {
    let name = name.trim();
    let name = if name.to_ascii_lowercase().ends_with(".litematic") { &name[..name.len() - 10] } else { name };
    let stem: String = name
        .chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    let stem = stem.trim_end_matches(['.', ' ']).trim().to_string();
    if stem.is_empty() {
        return Err(tr!("err.name_empty"));
    }
    if stem.chars().count() > 100 {
        return Err(tr!("err.name_long"));
    }
    Ok(stem)
}

/// Starts saving the area selection as `<name>.litematic` in the schematics folder: one region
/// with every block and its state, as Litematica writes it (version 7). Block entity data and
/// entities aren't known to the client this way and aren't saved.
pub fn start_save(w: &World<'_>, name: &str, overwrite: bool) -> Result<String, String> {
    if let Some(file) = with(|s| s.save.as_ref().map(|j| j.file.clone()).or_else(|| s.save_write.as_ref().map(|w| w.file.clone()))) {
        return Err(tr!("err.still_saving", file));
    }
    let (lo, hi) = with(|s| s.area_box()).ok_or_else(|| tr!("err.no_corners"))?;
    let size = blazematica_core::area::size(lo, hi).ok_or_else(|| tr!("err.area_too_large"))?;
    match blazematica_core::area::volume(size) {
        Some(volume) if volume <= MAX_SAVE => {}
        _ => return Err(tr!("err.area_volume", volume_text(size), MAX_SAVE)),
    }
    let stem = file_stem(name)?;
    let file = format!("{stem}.litematic");
    let path = store::schematics_dir(w)?.join(&file);
    if path.exists() && !overwrite {
        return Err(tr!("err.file_exists", file, name));
    }
    let pending: std::collections::BTreeSet<(i32, i32)> =
        (lo[0] >> 4..=hi[0] >> 4).flat_map(|cx| (lo[2] >> 4..=hi[2] >> 4).map(move |cz| (cx, cz))).collect();
    let total = pending.len();
    let job = SaveJob {
        name: name.trim().to_string(),
        file: file.clone(),
        path,
        lo,
        hi,
        builder: blazematica_core::schematic::RegionBuilder::new(size),
        ids: HashMap::new(),
        pending,
        total,
        unloaded: Vec::new(),
        started: std::time::Instant::now(),
        overwrite,
        world: store::world_key(w),
    };
    with(|s| s.save = Some(job));
    Ok(tr!("chat.saving", file, size[0], size[1], size[2], total))
}

/// The save's work for this tick: a finished file reported; chunks read while the time lasts;
/// the last one in, the file handed to a thread of its own to write.
fn save_step(w: &World<'_>) {
    if let Some(line) = with(|s| s.save_write.as_ref().and_then(|sw| sw.done.try_recv().ok())) {
        with(|s| {
            s.save_write = None;
            s.message = line.clone();
        });
        say(&line);
    }
    let Some(mut job) = with(|s| s.save.take()) else { return };
    if job.world != store::world_key(w) {
        let line = tr!("chat.save_left_world", job.file);
        with(|s| s.message = line.clone());
        say(&line);
        return;
    }
    let start = std::time::Instant::now();
    let loaded: Vec<(i32, i32)> = job.pending.iter().copied().filter(|&(cx, cz)| chunk_ready(w, &job, cx, cz)).collect();
    for c in loaded {
        if start.elapsed() > SAVE_BUDGET {
            break;
        }
        save_chunk(w, &mut job, c);
        job.pending.remove(&c);
    }
    if job.pending.is_empty() {
        finish_save(w, job);
        return;
    }
    let me = w.feet().unwrap_or(job.lo);
    let mut unloaded: Vec<(i32, i32)> = job.pending.iter().copied().filter(|&(cx, cz)| !chunk_ready(w, &job, cx, cz)).collect();
    unloaded.sort_by_key(|&(cx, cz)| {
        let (dx, dz) = (i64::from(cx * 16 + 8 - me[0]), i64::from(cz * 16 + 8 - me[2]));
        dx * dx + dz * dz
    });
    job.unloaded = unloaded;
    with(|s| s.save = Some(job));
}

/// Whether chunk column `cx`, `cz` is loaded where the area is (its corner cell at the area's
/// bottom is read).
fn chunk_ready(w: &World<'_>, job: &SaveJob, cx: i32, cz: i32) -> bool {
    let x = (cx * 16).max(job.lo[0]);
    let z = (cz * 16).max(job.lo[2]);
    w.loaded([x, job.lo[1], z])
}

/// Reads one chunk column's part of the area.
fn save_chunk(w: &World<'_>, job: &mut SaveJob, (cx, cz): (i32, i32)) {
    let (lo, hi) = (job.lo, job.hi);
    let (x0, x1) = ((cx * 16).max(lo[0]), (cx * 16 + 15).min(hi[0]));
    let (z0, z1) = ((cz * 16).max(lo[2]), (cz * 16 + 15).min(hi[2]));
    for y in lo[1]..=hi[1] {
        for z in z0..=z1 {
            for x in x0..=x1 {
                let Some(id) = w.block([x, y, z]) else { continue };
                let i = match job.ids.get(&id) {
                    Some(&i) => i,
                    None => {
                        let state = with(|s| s.blocks.state(w, id));
                        let i = job.builder.intern(state);
                        job.ids.insert(id, i);
                        i
                    }
                };
                if i != 0 {
                    job.builder.set([x - lo[0], y - lo[1], z - lo[2]], i);
                }
            }
        }
    }
}

/// Every chunk is in: the schematic is built and written by a thread of its own.
fn finish_save(w: &World<'_>, job: SaveJob) {
    let SaveJob { name, file, path, lo, hi, builder, started, overwrite, .. } = job;
    let asked = name.clone();
    let size = blazematica_core::area::size(lo, hi).unwrap_or([1; 3]);
    let volume = blazematica_core::area::volume(size).and_then(|v| i64::try_from(v).ok()).unwrap_or(0);
    let (blocks, states) = (builder.non_air(), builder.states());
    let stem = file.trim_end_matches(".litematic").to_string();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    let author = w.player.and_then(|p| p.name(w.host)).unwrap_or_default();
    let schematic = Schematic {
        version: 7,
        data_version: i64::from(crate::DATA_VERSION),
        metadata: blazematica_core::schematic::Metadata {
            name,
            author,
            description: String::new(),
            region_count: 1,
            total_volume: volume,
            total_blocks: blocks as i64,
            enclosing_size: size,
            time_created: now,
            time_modified: now,
        },
        regions: vec![builder.finish(&stem, Vec::new())],
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let name = file.clone();
    std::thread::spawn(move || {
        let written = schematic.to_bytes().map_err(Some).and_then(|bytes| commit_file(&path, &bytes, overwrite));
        let line = match written {
            Ok(()) => tr!("chat.saved", name, size[0], size[1], size[2], blocks, states, format!("{:.1}", started.elapsed().as_secs_f32())),
            Err(None) => tr!("error", tr!("err.file_exists", name, asked)),
            Err(Some(e)) => tr!("error", tr!("err.write", name, e)),
        };
        let _ = tx.send(line);
    });
    with(|s| s.save_write = Some(SaveWrite { file, done: rx }));
}

/// Writes `bytes` to `path` through a temporary file of its own in the same folder, created
/// exclusively so it can't replace anything. `path` itself is replaced only when `overwrite`, the
/// player's explicit choice; otherwise a file there (even one made while the save ran) is kept
/// and the result is `Err(None)`. `Err(Some(why))` for a failed write.
fn commit_file(path: &std::path::Path, bytes: &[u8], overwrite: bool) -> Result<(), Option<String>> {
    use std::io::{ErrorKind, Write};
    let shown = |p: &std::path::Path, e: std::io::Error| Some(format!("{}: {e}", p.display()));
    let dir = path.parent().ok_or_else(|| Some(format!("{}: no folder", path.display())))?;
    let base = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut made = None;
    for n in 0..64u32 {
        let tmp = dir.join(format!(".{base}.{stamp}.{n}.tmp"));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(f) => {
                made = Some((tmp, f));
                break;
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(shown(&tmp, e)),
        }
    }
    let Some((tmp, mut f)) = made else {
        return Err(Some(format!("{}: no free temporary file name", dir.display())));
    };
    let mut done = f.write_all(bytes).and_then(|()| f.sync_all()).map_err(|e| shown(&tmp, e));
    drop(f);
    if done.is_ok() {
        done = if overwrite {
            std::fs::rename(&tmp, path).map_err(|e| shown(path, e))
        } else {
            // A hard link is made only where no file is, so nothing already there is replaced.
            match std::fs::hard_link(&tmp, path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(None),
                // No hard links on this drive: nothing is created, as the file couldn't appear
                // whole in one step without the risk of replacing one.
                Err(e) => Err(Some(tr!("err.save_no_link", path.display(), e))),
            }
        };
    }
    let _ = std::fs::remove_file(&tmp);
    done
}

fn cancel_save() -> Result<String, String> {
    with(|s| match s.save.take() {
        Some(job) => Ok(tr!("chat.save_cancelled", job.file)),
        None if s.save_write.is_some() => Err(tr!("err.writing")),
        None => Err(tr!("err.no_save")),
    })
}

/// "north-east" and the like, for a step of `dx`, `dz` (north is -z).
fn compass(dx: i32, dz: i32) -> String {
    const NAMES: [&str; 8] = ["e", "se", "s", "sw", "w", "nw", "n", "ne"];
    let angle = f64::from(dz).atan2(f64::from(dx)).to_degrees().rem_euclid(360.0);
    tr!(&format!("compass.{}", NAMES[((angle + 22.5) / 45.0) as usize % 8]))
}

/// The save's progress: lines for the HUD and `/lm save status`, and how far it is (0 to 1).
pub fn save_lines(w: &World<'_>, s: &State) -> Option<(Vec<(String, u32)>, f32)> {
    if let Some(sw) = &s.save_write {
        return Some((vec![(tr!("hud.writing", sw.file), 0xFFAA00)], 1.0));
    }
    let job = s.save.as_ref()?;
    let done = job.total - job.pending.len();
    let mut lines = vec![(tr!("hud.saving", job.file, done, job.total), 0xFFAA00)];
    if let Some(&(cx, cz)) = job.unloaded.first() {
        let me = w.feet().unwrap_or(job.lo);
        let (dx, dz) = (cx * 16 + 8 - me[0], cz * 16 + 8 - me[2]);
        let far = f64::from(dx).hypot(f64::from(dz)).round();
        lines.push((
            tr!("hud.not_loaded", job.unloaded.len(), far, compass(dx, dz), cx * 16 + 8, cz * 16 + 8),
            0xFFAA00,
        ));
    }
    Some((lines, done as f32 / job.total.max(1) as f32))
}

// ---------------------------------------------------------------- per tick

/// Each client tick in a world.
pub fn tick(w: &World<'_>) {
    if !w.in_world() {
        return;
    }
    ensure_settings(w);
    with(|s| s.ticks += 1);
    restore_step(w);
    load_step(w);
    with(State::sync_rev);
    scan_step(w);
    save_step(w);
    paste_step(w);
}

// ---------------------------------------------------------------- paste

/// Blocks a paste places in one tick, at most.
const PASTE_PER_TICK: usize = 48;
/// Positions a paste looks at in one tick, at most (and no longer than `PASTE_BUDGET`).
const PASTE_CELLS: usize = 16_384;
const PASTE_BUDGET: std::time::Duration = std::time::Duration::from_millis(2);
/// Ticks the first block may take to come back from the server before the paste gives up.
const PASTE_PROBE_TICKS: u32 = 40;

/// A paste: the selected placement's missing and wrong blocks set with `/setblock`, region by
/// region from the bottom up, within the shown layers. The server decides whether the player may:
/// the first block is sent alone, and the rest only once it has appeared.
struct Paste {
    slot: u64,
    boxes: Vec<([i32; 3], [i32; 3])>,
    /// The box being walked and the next position in it (x, then z, then y).
    at: usize,
    next: [i32; 3],
    /// The first block sent, and the ticks since, until the world shows it.
    probe: Option<([i32; 3], u32)>,
    placed: u64,
    /// Positions in chunks the client doesn't hold: left for a second paste.
    unloaded: u64,
    started: std::time::Instant,
    /// The placement's revision when the paste began: a change (turned, mirrored, reloaded)
    /// stops it, as its palette no longer matches `ids`.
    revision: u64,
    /// The palette check under way: the next region and entry to look up, None once all are.
    check: Option<(usize, usize)>,
    /// The game's exact state for each palette entry, by region (None: air, structure void, or
    /// not checked yet). Commands are written from these alone.
    ids: Vec<Vec<Option<StateId>>>,
}

/// Palette entries a paste checks in one tick, at most (and no longer than `PASTE_BUDGET`).
const PASTE_CHECKS: usize = 512;

fn start_paste() -> Result<String, String> {
    if with(|s| s.paste.is_some()) {
        return Err(tr!("err.still_pasting"));
    }
    with(|s| {
        let slot = s.sel().ok_or_else(|| tr!("err.no_schematic"))?;
        let boxes: Vec<_> = slot.placement.region_boxes().into_iter().filter_map(|b| s.layer.clip_bounds(b)).collect();
        let Some(&(first, _)) = boxes.first() else { return Err(tr!("err.paste_empty")) };
        let name = display_name(&slot.placement);
        // Every block the paste could send is checked first, a slice each tick (`paste_step`):
        // nothing is sent until all of them are exact game states.
        let ids = slot.placement.palettes().iter().map(|p| vec![None; p.len()]).collect();
        let revision = slot.placement.revision;
        s.paste = Some(Paste {
            revision,
            check: Some((0, 0)),
            ids,
            slot: slot.id,
            boxes,
            at: 0,
            next: first,
            probe: None,
            placed: 0,
            unloaded: 0,
            started: std::time::Instant::now(),
        });
        Ok(tr!("chat.pasting", name))
    })
}

fn cancel_paste() -> Result<String, String> {
    with(|s| s.paste.take().map(|p| tr!("chat.paste_cancelled", p.placed)).ok_or_else(|| tr!("err.no_paste")))
}

fn paste_status() -> Result<String, String> {
    with(|s| {
        let p = s.paste.as_ref().ok_or_else(|| tr!("err.no_paste"))?;
        Ok(tr!("chat.paste_status", p.placed, p.at + 1, p.boxes.len()))
    })
}

/// The paste's work for this tick: the probe watched, then positions walked and commands queued
/// within the limits.
fn paste_step(w: &World<'_>) {
    with(|s| {
        let Some(mut p) = s.paste.take() else { return };
        let Some(slot) = s.slots.iter().position(|x| x.id == p.slot) else {
            s.out.push(tr!("error", tr!("err.paste_gone")));
            return;
        };
        if s.slots[slot].placement.revision != p.revision {
            s.out.push(tr!("error", tr!("err.paste_changed", p.placed)));
            return;
        }
        // The palette check: every entry must be an exact game state, or nothing is sent.
        if let Some((mut ri, mut pi)) = p.check {
            let start = std::time::Instant::now();
            let mut looked = 0;
            loop {
                let palettes = s.slots[slot].placement.palettes();
                let Some(region) = palettes.get(ri) else {
                    p.check = None;
                    break;
                };
                let Some(b) = region.get(pi).cloned() else {
                    (ri, pi) = (ri + 1, 0);
                    continue;
                };
                if looked >= PASTE_CHECKS || start.elapsed() >= PASTE_BUDGET {
                    p.check = Some((ri, pi));
                    break;
                }
                looked += 1;
                pi += 1;
                if b.is_air() || b.is_void() {
                    continue;
                }
                match s.blocks.strict(w, &b) {
                    Some(id) => p.ids[ri][pi - 1] = Some(id),
                    None => {
                        s.out.push(tr!("error", tr!("err.paste_unknown", clip_chars(&b.name, 64))));
                        return;
                    }
                }
            }
            s.paste = Some(p);
            return;
        }
        if let Some((at, ticks)) = p.probe {
            match quick_status_in(w, s, slot, at) {
                Some((Status::Correct, ..)) => p.probe = None,
                _ if ticks >= PASTE_PROBE_TICKS => {
                    s.out.push(tr!("error", tr!("err.paste_denied")));
                    return;
                }
                _ => {
                    p.probe = Some((at, ticks + 1));
                    s.paste = Some(p);
                    return;
                }
            }
        }
        let start = std::time::Instant::now();
        let (mut cells, mut sent) = (0, 0);
        while p.at < p.boxes.len() && sent < PASTE_PER_TICK && cells < PASTE_CELLS && start.elapsed() < PASTE_BUDGET {
            let (lo, hi) = p.boxes[p.at];
            let at = p.next;
            // Next: x, then z, then y; then the next box.
            let mut n = at;
            n[0] += 1;
            if n[0] > hi[0] {
                n[0] = lo[0];
                n[2] += 1;
                if n[2] > hi[2] {
                    n[2] = lo[2];
                    n[1] += 1;
                }
            }
            if n[1] > hi[1] {
                p.at += 1;
                if let Some(&(first, _)) = p.boxes.get(p.at) {
                    n = first;
                }
            }
            p.next = n;
            cells += 1;
            if !w.loaded(at) {
                p.unloaded += 1;
                continue;
            }
            let Some((status, r, _)) = quick_status_in(w, s, slot, at) else { continue };
            if !matches!(status, Status::Missing | Status::WrongBlock | Status::WrongState) {
                continue;
            }
            // The game's own text for the exact state the check found, never the file's.
            let (_, (ri, pi)) = r;
            let checked = p.ids.get(ri).and_then(|v| usize::try_from(pi).ok().and_then(|i| v.get(i))).copied().flatten();
            let Some(state) = checked.map(|id| s.blocks.state(w, id)).and_then(|g| g.command_text()) else {
                s.out.push(tr!("error", tr!("err.paste_unknown", clip_chars(&s.state_of(r).name, 64))));
                return;
            };
            s.commands.push(format!("/setblock {} {} {} {state}", at[0], at[1], at[2]));
            p.placed += 1;
            sent += 1;
            if p.placed == 1 {
                p.probe = Some((at, 0));
                break;
            }
        }
        if p.at >= p.boxes.len() && p.probe.is_none() {
            let secs = format!("{:.1}", p.started.elapsed().as_secs_f32());
            s.out.push(tr!("chat.pasted", p.placed, secs));
            if p.unloaded > 0 {
                s.out.push(tr!("chat.paste_unloaded", p.unloaded));
            }
            return;
        }
        s.paste = Some(p);
    });
}

// ---------------------------------------------------------------- hover info

/// How far along the view hover info looks.
const HOVER_REACH: f64 = 32.0;

/// Hover info: the first position along the view, up to the first real block in the way, where
/// the build differs from the shown placements, and what should be there.
fn hover_line(w: &World<'_>, s: &mut State) -> Option<(String, u32)> {
    if s.no_hover {
        return None;
    }
    let (eye, dir) = w.aim()?;
    let hit = w.ray(HOVER_REACH).map(|(b, _)| b);
    let mut cell = eye.map(|v| v.floor() as i32);
    let step = dir.map(|d| if d > 0.0 { 1 } else if d < 0.0 { -1 } else { 0 });
    let delta = dir.map(|d| if d == 0.0 { f64::INFINITY } else { (1.0 / d).abs() });
    let mut next = [0, 1, 2].map(|i| match step[i] {
        1 => (f64::from(cell[i]) + 1.0 - eye[i]) / dir[i],
        -1 => (eye[i] - f64::from(cell[i])) / -dir[i],
        _ => f64::INFINITY,
    });
    let mut travelled = 0.0;
    while travelled <= HOVER_REACH {
        if let Some((status, r, _)) = quick_status(w, s, cell) {
            let wanted = s.state_of(r).clone();
            let actual = w.block(cell).map(|id| s.blocks.state(w, id));
            let text = match status {
                Status::Missing => Some(tr!("hud.hover.missing", short_name(&wanted))),
                Status::WrongBlock => actual.map(|a| tr!("hud.hover.wrong", short_name(&a), short_name(&wanted))),
                Status::Extra => actual.map(|a| tr!("hud.hover.extra", short_name(&a))),
                Status::WrongState => actual.map(|a| tr!("hud.hover.state", short_name(&wanted), state_diff(&a, &wanted))),
                _ => None,
            };
            if let Some(text) = text {
                return Some((text, 0xFFFF55));
            }
        }
        if Some(cell) == hit {
            return None;
        }
        let i = if next[0] <= next[1] && next[0] <= next[2] {
            0
        } else if next[1] <= next[2] {
            1
        } else {
            2
        };
        if !next[i].is_finite() {
            return None;
        }
        travelled = next[i];
        next[i] += delta[i];
        cell[i] += step[i];
    }
    None
}

/// A block's id without `minecraft:`.
fn short_name(state: &BlockState) -> String {
    clip_chars(state.name.strip_prefix("minecraft:").unwrap_or(&state.name), 64)
}

/// `text` cut to `max` characters with "...", so a file's long text stays short on screen.
pub fn clip_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}...", &text[..i]),
        None => text.to_string(),
    }
}

/// The properties that differ, as `facing: east -> north`.
fn state_diff(actual: &BlockState, wanted: &BlockState) -> String {
    let mut out = String::new();
    for (k, v) in &wanted.properties {
        let have = actual.prop(k).unwrap_or("-");
        if have == v {
            continue;
        }
        // Once the line is full, the rest isn't looked at.
        if out.len() >= DIFF_CHARS {
            out.push_str(", ...");
            break;
        }
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(&format!("{}: {} -> {}", clip_chars(k, 32), clip_chars(have, 32), clip_chars(v, 32)));
    }
    out
}

/// About as much of a property diff as the HUD line shows.
const DIFF_CHARS: usize = 160;

/// The verifier, over and over, for the selected placement alone (shown or not): its progress
/// in the HUD and its material list. Another placement selected starts it over.
fn scan_step(w: &World<'_>) {
    with(|s| {
        let Some(slot) = s.sel() else { return };
        let (sel, key) = (s.selected, (slot.id, slot.placement.revision));
        let (lo, hi) = slot.placement.bounds();
        let size = [0, 1, 2].map(|i| (hi[i] - lo[i] + 1) as u64);
        let volume = size[0] * size[1] * size[2];
        if s.scan.key != key {
            s.scan = Scan { key, ..Scan::default() };
            if s.tally_of != key.0 {
                s.tally = None;
                s.missing.clear();
                s.unloaded = 0;
            }
        }
        let end = (s.scan.index + SCAN_PER_TICK).min(volume);
        let mut i = s.scan.index;
        while i < end {
            let x = (i % size[0]) as i32;
            let z = ((i / size[0]) % size[2]) as i32;
            let y = (i / (size[0] * size[2])) as i32;
            i += 1;
            let p = [lo[0] + x, lo[1] + y, lo[2] + z];
            if !w.loaded(p) {
                if s.slots[sel].placement.wanted(p).is_some_and(|b| !b.is_air()) {
                    s.scan.unloaded += 1;
                }
                continue;
            }
            let Some((status, at, _)) = quick_status_in(w, s, sel, p) else { continue };
            let wanted = s.state_of(at).clone();
            s.scan.tally.add(status);
            if matches!(status, Status::Missing | Status::WrongBlock) {
                let per = u64::from(materials::items_for_state(&wanted));
                if per > 0
                    && let Some(item) = item_for(w, s, &wanted)
                {
                    *s.scan.missing.entry(item).or_insert(0) += per;
                }
            }
        }
        s.scan.index = i;
        if i >= volume {
            let done = std::mem::replace(&mut s.scan, Scan { key, ..Scan::default() });
            s.tally_of = key.0;
            s.tally = Some(done.tally);
            s.missing = done.missing;
            s.unloaded = done.unloaded;
        }
    });
}

// ---------------------------------------------------------------- HUD

/// What the highlights show, in words.
pub fn highlight_text(s: &State) -> String {
    let what = match (s.settings.highlight_missing, s.settings.highlight_wrong) {
        (false, false) => return tr!("hl.off"),
        (true, false) => tr!("hl.missing"),
        (false, true) => tr!("hl.wrong"),
        (true, true) => tr!("hl.both"),
    };
    match &s.highlight_item {
        Some(item) => tr!("hl.only", what, item_name(item)),
        None => tr!("hl.all", what),
    }
}

fn inventory_counts(w: &World<'_>) -> BTreeMap<String, u64> {
    let mut m = BTreeMap::new();
    for (item, n) in w.inventory() {
        *m.entry(item).or_insert(0) += n;
    }
    m
}

/// A material's name in the player's language (the game's own item names).
pub fn pretty_item(item: &str) -> String {
    item_name(item)
}

/// Puts the selected placement's material list on the clipboard, as a table a spreadsheet takes.
fn copy_materials(w: &World<'_>) -> Result<Vec<String>, String> {
    let list = material_list(w);
    if list.is_empty() {
        return Err(tr!("err.no_items"));
    }
    if !w.host.set_clipboard_text(&materials::table(&list, &tr!("materials.table_header"), pretty_item)) {
        return Err(tr!("err.clipboard"));
    }
    Ok(vec![tr!("chat.copied", list.len())])
}

pub fn material_list(w: &World<'_>) -> Vec<Material> {
    let inv = inventory_counts(w);
    with(|s| {
        let Some(slot) = s.sel() else { return Vec::new() };
        let missing = if s.tally().is_some() { s.missing.clone() } else { slot.totals.clone() };
        materials::build_list(&slot.totals, &missing, &inv)
    })
}

pub fn volume_text(size: [i32; 3]) -> String {
    blazematica_core::area::volume(size).map_or_else(|| tr!("too_many"), |v| v.to_string())
}

pub fn human_size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => tr!("size.gb", format!("{:.1}", b as f64 / f64::from(1u32 << 30))),
        b if b >= 1 << 20 => tr!("size.mb", format!("{:.1}", b as f64 / f64::from(1u32 << 20))),
        b if b >= 1 << 10 => tr!("size.kb", format!("{:.0}", b as f64 / 1024.0)),
        b => tr!("size.b", b),
    }
}

/// The info box's lines (top left) and whether the material list shows (top right).
pub fn hud_lines(w: &World<'_>) -> Vec<(String, u32)> {
    let tool = tool_held(w);
    let mut lines = with(|s| {
        let area = s.area_size();
        let Some(slot) = s.sel() else {
            return match area {
                Some(d) if s.settings.hud => {
                    vec![(tr!("hud.area_only", d[0], d[1], d[2]), 0x55FFFFu32)]
                }
                _ => Vec::new(),
            };
        };
        let p = &slot.placement;
        if !s.settings.hud && !s.moving {
            return Vec::new();
        }
        let count = if s.slots.len() > 1 { format!(" ({}/{})", s.selected + 1, s.slots.len()) } else { String::new() };
        let hidden = if slot.shown { String::new() } else { tr!("hud.hidden") };
        let mut l = vec![(format!("Blazematica: {}{count}{hidden}", display_name(p)), 0x55FFFFu32)];
        if s.moving {
            l.push((tr!("hud.move_mode"), 0xFFFF55));
        }
        l.push((tr!("hud.origin", p.origin[0], p.origin[1], p.origin[2], rotation_text(p.rotation), mirror_text(p.mirror)), 0xE0E0E0));
        let hidden = if s.settings.render { String::new() } else { tr!("hud.render_off") };
        l.push((tr!("hud.layers", layer_text(s.layer), hidden), 0xE0E0E0));
        match s.tally() {
            Some(t) => {
                l.push((tr!("hud.done", format!("{:.1}", t.percent()), t.missing), 0x55FF55));
                if t.wrong_block + t.wrong_state + t.extra > 0 {
                    l.push((tr!("hud.wrong", t.wrong_block, t.wrong_state, t.extra), 0xFFAA00));
                }
            }
            None => l.push((tr!("hud.verifying"), 0xAAAAAA)),
        }
        if let Some(p) = &s.paste {
            l.push((tr!("hud.paste", p.placed), 0xFF55FF));
        }
        if let Some(line) = hover_line(w, s) {
            l.push(line);
        }
        if s.settings.printer_on {
            l.push((tr!("hud.printer", s.placed), 0xFF55FF));
            if !s.printer_note.is_empty() {
                l.push((s.printer_note.clone(), 0xAAAAAA));
            }
        }
        if s.settings.easy_place {
            l.push((tr!("hud.easy_place"), 0xFF55FF));
            if !s.easy_note.is_empty() {
                l.push((s.easy_note.clone(), 0xAAAAAA));
            }
        }
        if s.settings.highlight_missing || s.settings.highlight_wrong {
            l.push((highlight_text(s), 0x55FFFF));
        }
        if let Some(d) = area {
            l.push((tr!("hud.area", d[0], d[1], d[2]), 0xE0E0E0));
        }
        l
    });
    if let Some((save, _)) = with(|s| save_lines(w, s)) {
        lines.extend(save);
    }
    lines.extend(with(|s| load_lines(s)));
    if tool && with(|s| s.settings.tool_hud) {
        lines.extend(with(|s| tool_lines(s)));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::commit_file;

    fn folder(name: &str) -> std::path::PathBuf {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("blazematica-{name}-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn leftovers(dir: &std::path::Path) -> Vec<String> {
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".tmp")).collect()
    }

    #[test]
    fn a_long_property_diff_stops_once_the_line_is_full() {
        use super::{BlockState, DIFF_CHARS, state_diff};
        let mut wanted = BlockState::parse("minecraft:stone").unwrap();
        for i in 0..10_000 {
            wanted.properties.insert(format!("p{i:05}"), "on".into());
        }
        let actual = BlockState::parse("minecraft:stone").unwrap();
        let diff = state_diff(&actual, &wanted);
        assert!(diff.len() < DIFF_CHARS + 120 && diff.ends_with(", ..."), "{} chars", diff.len());
        assert_eq!(state_diff(&BlockState::parse("minecraft:stone[a=1]").unwrap(), &BlockState::parse("minecraft:stone[a=2]").unwrap()), "a: 1 -> 2");
    }

    #[test]
    fn a_save_never_replaces_a_file_unless_asked() {
        let dir = folder("commit");
        let path = dir.join("hut.litematic");
        assert_eq!(commit_file(&path, b"first", false), Ok(()));
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        // Made meanwhile, or already there: kept without the player's overwrite.
        assert_eq!(commit_file(&path, b"second", false), Err(None));
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        assert_eq!(commit_file(&path, b"third", true), Ok(()));
        assert_eq!(std::fs::read(&path).unwrap(), b"third");
        assert!(leftovers(&dir).is_empty(), "temporary files are removed: {:?}", leftovers(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_named_like_the_old_temporary_one_is_left_alone() {
        let dir = folder("tmpname");
        std::fs::write(dir.join("hut.litematic.tmp"), b"mine").unwrap();
        assert_eq!(commit_file(&dir.join("hut.litematic"), b"saved", false), Ok(()));
        assert_eq!(std::fs::read(dir.join("hut.litematic.tmp")).unwrap(), b"mine");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
