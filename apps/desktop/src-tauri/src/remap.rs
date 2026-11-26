#![cfg_attr(not(windows), allow(dead_code))]
use crate::{model::Profile, state::Core, storage};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex, MutexGuard, OnceLock, TryLockError,
};

pub const UNSUPPORTED: &str = "Remapping is not supported on this platform yet.";

pub fn supported() -> bool {
    cfg!(target_os = "windows")
}

pub const KEY_SLOTS: usize = 512;
pub const MOUSE_SLOTS: usize = 3;
const SLOTS: usize = KEY_SLOTS + MOUSE_SLOTS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyOut {
    pub id: u16,
    pub vk: u8,
}
impl KeyOut {
    pub fn scan(self) -> u16 {
        self.id & 0xFF
    }
    pub fn extended(self) -> bool {
        self.id & 0x100 != 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Action {
    #[default]
    Pass,
    Disable,
    Send { mods: u8, key: Option<KeyOut> },
}

pub const MOD_CTRL: u8 = 1;
pub const MOD_SHIFT: u8 = 2;
pub const MOD_ALT: u8 = 4;
pub const MOD_WIN: u8 = 8;
const MODIFIERS: [(u8, KeyOut); 4] = [
    (MOD_CTRL, KeyOut { id: 0x1D, vk: 0 }),
    (MOD_SHIFT, KeyOut { id: 0x2A, vk: 0 }),
    (MOD_ALT, KeyOut { id: 0x38, vk: 0 }),
    (MOD_WIN, KeyOut { id: 0x15B, vk: 0 }),
];
const ESCAPE: u16 = 0x01;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Middle = 0,
    Back = 1,
    Forward = 2,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Key(u16),
    Mouse(MouseButton),
}
impl Source {
    fn slot(self) -> Option<usize> {
        match self {
            Self::Key(id) if (id as usize) < KEY_SLOTS && id & 0xFF != 0 => Some(id as usize),
            Self::Key(_) => None,
            Self::Mouse(button) => Some(KEY_SLOTS + button as usize),
        }
    }
}

struct KeyDef {
    code: &'static str,
    id: u16,
    vk: u8,
}
const fn k(code: &'static str, id: u16) -> KeyDef {
    KeyDef { code, id, vk: 0 }
}
const fn v(code: &'static str, id: u16, vk: u8) -> KeyDef {
    KeyDef { code, id, vk }
}
const E: u16 = 0x100;
const KEYS: &[KeyDef] = &[
    k("Escape", 0x01),
    k("Digit1", 0x02),
    k("Digit2", 0x03),
    k("Digit3", 0x04),
    k("Digit4", 0x05),
    k("Digit5", 0x06),
    k("Digit6", 0x07),
    k("Digit7", 0x08),
    k("Digit8", 0x09),
    k("Digit9", 0x0A),
    k("Digit0", 0x0B),
    k("Minus", 0x0C),
    k("Equal", 0x0D),
    k("Backspace", 0x0E),
    k("Tab", 0x0F),
    k("KeyQ", 0x10),
    k("KeyW", 0x11),
    k("KeyE", 0x12),
    k("KeyR", 0x13),
    k("KeyT", 0x14),
    k("KeyY", 0x15),
    k("KeyU", 0x16),
    k("KeyI", 0x17),
    k("KeyO", 0x18),
    k("KeyP", 0x19),
    k("BracketLeft", 0x1A),
    k("BracketRight", 0x1B),
    k("Enter", 0x1C),
    k("ControlLeft", 0x1D),
    k("KeyA", 0x1E),
    k("KeyS", 0x1F),
    k("KeyD", 0x20),
    k("KeyF", 0x21),
    k("KeyG", 0x22),
    k("KeyH", 0x23),
    k("KeyJ", 0x24),
    k("KeyK", 0x25),
    k("KeyL", 0x26),
    k("Semicolon", 0x27),
    k("Quote", 0x28),
    k("Backquote", 0x29),
    k("ShiftLeft", 0x2A),
    k("Backslash", 0x2B),
    k("KeyZ", 0x2C),
    k("KeyX", 0x2D),
    k("KeyC", 0x2E),
    k("KeyV", 0x2F),
    k("KeyB", 0x30),
    k("KeyN", 0x31),
    k("KeyM", 0x32),
    k("Comma", 0x33),
    k("Period", 0x34),
    k("Slash", 0x35),
    k("ShiftRight", 0x36),
    k("NumpadMultiply", 0x37),
    k("AltLeft", 0x38),
    k("Space", 0x39),
    k("CapsLock", 0x3A),
    k("F1", 0x3B),
    k("F2", 0x3C),
    k("F3", 0x3D),
    k("F4", 0x3E),
    k("F5", 0x3F),
    k("F6", 0x40),
    k("F7", 0x41),
    k("F8", 0x42),
    k("F9", 0x43),
    k("F10", 0x44),
    k("ScrollLock", 0x46),
    k("Numpad7", 0x47),
    k("Numpad8", 0x48),
    k("Numpad9", 0x49),
    k("NumpadSubtract", 0x4A),
    k("Numpad4", 0x4B),
    k("Numpad5", 0x4C),
    k("Numpad6", 0x4D),
    k("NumpadAdd", 0x4E),
    k("Numpad1", 0x4F),
    k("Numpad2", 0x50),
    k("Numpad3", 0x51),
    k("Numpad0", 0x52),
    k("NumpadDecimal", 0x53),
    k("IntlBackslash", 0x56),
    k("F11", 0x57),
    k("F12", 0x58),
    k("F13", 0x64),
    k("F14", 0x65),
    k("F15", 0x66),
    k("F16", 0x67),
    k("F17", 0x68),
    k("F18", 0x69),
    k("F19", 0x6A),
    k("F20", 0x6B),
    k("F21", 0x6C),
    k("F22", 0x6D),
    k("F23", 0x6E),
    k("IntlRo", 0x73),
    k("F24", 0x76),
    k("NumpadComma", 0x7E),
    k("NumpadEnter", E | 0x1C),
    k("ControlRight", E | 0x1D),
    k("NumpadDivide", E | 0x35),
    v("PrintScreen", E | 0x37, 0x2C),
    k("AltRight", E | 0x38),
    v("NumLock", E | 0x45, 0x90),
    k("Home", E | 0x47),
    k("ArrowUp", E | 0x48),
    k("PageUp", E | 0x49),
    k("ArrowLeft", E | 0x4B),
    k("ArrowRight", E | 0x4D),
    k("End", E | 0x4F),
    k("ArrowDown", E | 0x50),
    k("PageDown", E | 0x51),
    k("Insert", E | 0x52),
    k("Delete", E | 0x53),
    k("MetaLeft", E | 0x5B),
    k("MetaRight", E | 0x5C),
    k("ContextMenu", E | 0x5D),
    v("MediaTrackPrevious", E | 0x10, 0xB1),
    v("MediaTrackNext", E | 0x19, 0xB0),
    v("AudioVolumeMute", E | 0x20, 0xAD),
    v("MediaPlayPause", E | 0x22, 0xB3),
    v("MediaStop", E | 0x24, 0xB2),
    v("AudioVolumeDown", E | 0x2E, 0xAE),
    v("AudioVolumeUp", E | 0x30, 0xAF),
];

fn key_by_code(code: &str) -> Option<KeyOut> {
    KEYS.iter()
        .find(|d| d.code.eq_ignore_ascii_case(code))
        .map(|d| KeyOut { id: d.id, vk: d.vk })
}

fn key_token(token: &str) -> Option<KeyOut> {
    let token = token.trim();
    if let Some(key) = key_by_code(token) {
        return Some(key);
    }
    let lower = token.to_lowercase();
    let mut chars = lower.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_lowercase() {
            return key_by_code(&format!("Key{c}"));
        }
        if c.is_ascii_digit() {
            return key_by_code(&format!("Digit{c}"));
        }
    }
    let code = match lower.as_str() {
        "esc" => "Escape",
        "del" => "Delete",
        "ins" => "Insert",
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "pgup" => "PageUp",
        "pgdn" => "PageDown",
        "return" => "Enter",
        "espaço" | "espaco" | "spacebar" => "Space",
        "caps" => "CapsLock",
        "prtsc" | "printscr" => "PrintScreen",
        "menu" => "ContextMenu",
        "altgr" => "AltRight",
        _ => modifier_token(&lower).map(|bit| match bit {
            MOD_CTRL => "ControlLeft",
            MOD_SHIFT => "ShiftLeft",
            MOD_ALT => "AltLeft",
            _ => "MetaLeft",
        })?,
    };
    key_by_code(code)
}
fn modifier_token(lower: &str) -> Option<u8> {
    Some(match lower {
        "ctrl" | "control" => MOD_CTRL,
        "shift" => MOD_SHIFT,
        "alt" => MOD_ALT,
        "win" | "windows" | "meta" | "super" | "cmd" => MOD_WIN,
        _ => return None,
    })
}

pub fn parse_source(value: &str) -> Option<Source> {
    let value = value.trim();
    if value.contains('+') && value.len() > 1 {
        return None;
    }
    let mouse = match value.to_lowercase().as_str() {
        "mousemiddle" | "middleclick" | "mouse3" => Some(MouseButton::Middle),
        "mouseback" | "xbutton1" | "mouse4" => Some(MouseButton::Back),
        "mouseforward" | "xbutton2" | "mouse5" => Some(MouseButton::Forward),
        _ => None,
    };
    match mouse {
        Some(button) => Some(Source::Mouse(button)),
        None => key_token(value).map(|key| Source::Key(key.id)),
    }
}

pub fn parse_target(value: &str) -> Option<Action> {
    let value = value.trim();
    let lower = value.to_lowercase();
    if [
        "disabled",
        "disable",
        "desativar",
        "desativado",
        "none",
        "nenhum",
    ]
    .contains(&lower.as_str())
    {
        return Some(Action::Disable);
    }
    if value.is_empty() {
        return None;
    }
    let (mut mods, mut key) = (0u8, None);
    for token in value.split('+').map(str::trim) {
        if token.is_empty() {
            return None;
        }
        if let Some(bit) = modifier_token(&token.to_lowercase()) {
            mods |= bit;
            continue;
        }
        if key.replace(key_token(token)?).is_some() {
            return None;
        }
    }
    Some(Action::Send { mods, key })
}

pub fn normalize_app(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_matches('"').trim();
    let name = trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed).trim();
    let lower = name.to_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower).trim();
    (!stem.is_empty()).then(|| stem.to_string())
}

#[derive(Clone)]
pub struct Table {
    slots: Box<[Action; SLOTS]>,
}
impl Default for Table {
    fn default() -> Self {
        Self {
            slots: Box::new([Action::Pass; SLOTS]),
        }
    }
}
impl Table {
    #[cfg(test)]
    pub fn get(&self, source: Source) -> Action {
        source
            .slot()
            .map(|slot| self.slots[slot])
            .unwrap_or(Action::Pass)
    }
    fn overlay(&mut self, profile: &Profile) -> Vec<usize> {
        let mut ignored = vec![];
        for (index, mapping) in profile.mappings.iter().enumerate() {
            let parsed = parse_source(&mapping.from)
                .and_then(|source| source.slot())
                .zip(parse_target(&mapping.to));
            match parsed {
                Some((slot, action)) => self.slots[slot] = action,
                None => ignored.push(index),
            }
        }
        ignored
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileState {
    pub id: String,
    pub applied: bool,
    pub reason: Option<&'static str>,
    pub ignored: Vec<usize>,
}
struct AppTable {
    app: String,
    table: Arc<Table>,
    ids: Vec<String>,
}
pub struct Compiled {
    default: Arc<Table>,
    default_ids: Vec<String>,
    apps: Vec<AppTable>,
    empty: Arc<Table>,
    pub profiles: Vec<ProfileState>,
}
pub struct Resolved {
    pub table: Arc<Table>,
    pub ids: Vec<String>,
}
impl Compiled {
    pub fn new(profiles: &[Profile]) -> Self {
        let mut states = vec![];
        let mut default = Table::default();
        let mut default_ids = vec![];
        let mut apps: Vec<(String, Vec<&Profile>)> = vec![];
        for profile in profiles {
            if profile.device_id.is_some() {
                states.push(ProfileState {
                    id: profile.id.clone(),
                    applied: false,
                    reason: Some("device_filter"),
                    ignored: vec![],
                });
                continue;
            }
            match profile.application.as_deref().and_then(normalize_app) {
                None => {
                    let ignored = default.overlay(profile);
                    default_ids.push(profile.id.clone());
                    states.push(ProfileState {
                        id: profile.id.clone(),
                        applied: true,
                        reason: None,
                        ignored,
                    });
                }
                Some(app) => {
                    match apps.iter_mut().find(|(a, _)| *a == app) {
                        Some((_, list)) => list.push(profile),
                        None => apps.push((app, vec![profile])),
                    }
                    states.push(ProfileState {
                        id: profile.id.clone(),
                        applied: true,
                        reason: None,
                        ignored: vec![],
                    });
                }
            }
        }
        let apps = apps
            .into_iter()
            .map(|(app, list)| {
                let mut table = default.clone();
                for profile in &list {
                    let ignored = table.overlay(profile);
                    if let Some(state) = states.iter_mut().find(|s| s.id == profile.id) {
                        state.ignored = ignored;
                    }
                }
                let mut ids = default_ids.clone();
                ids.extend(list.iter().map(|p| p.id.clone()));
                AppTable {
                    app,
                    table: Arc::new(table),
                    ids,
                }
            })
            .collect();
        Self {
            default: Arc::new(default),
            default_ids,
            apps,
            empty: Arc::new(Table::default()),
            profiles: states,
        }
    }
    pub fn resolve(&self, exe: Option<&str>) -> Resolved {
        let app = exe.and_then(normalize_app);
        match app.and_then(|app| self.apps.iter().find(|a| a.app == app)) {
            Some(entry) => Resolved {
                table: entry.table.clone(),
                ids: entry.ids.clone(),
            },
            None => Resolved {
                table: self.default.clone(),
                ids: self.default_ids.clone(),
            },
        }
    }
    pub fn paused(&self) -> Resolved {
        Resolved {
            table: self.empty.clone(),
            ids: vec![],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Synth {
    pub key: KeyOut,
    pub down: bool,
}
#[derive(Default)]
pub struct Out {
    buf: [Option<Synth>; 8],
    len: usize,
}
impl Out {
    fn push(&mut self, synth: Synth) {
        if self.len < self.buf.len() {
            self.buf[self.len] = Some(synth);
            self.len += 1;
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = Synth> + '_ {
        self.buf[..self.len].iter().flatten().copied()
    }
}
trait Sink {
    fn emit(&mut self, synth: Synth);
}
impl Sink for Out {
    fn emit(&mut self, synth: Synth) {
        self.push(synth);
    }
}
impl Sink for Vec<Synth> {
    fn emit(&mut self, synth: Synth) {
        self.push(synth);
    }
}
fn press(action: Action, sink: &mut impl Sink) {
    if let Action::Send { mods, key } = action {
        for (bit, key) in MODIFIERS {
            if mods & bit != 0 {
                sink.emit(Synth { key, down: true });
            }
        }
        if let Some(key) = key {
            sink.emit(Synth { key, down: true });
        }
    }
}
fn release(action: Action, sink: &mut impl Sink) {
    if let Action::Send { mods, key } = action {
        if let Some(key) = key {
            sink.emit(Synth { key, down: false });
        }
        for (bit, key) in MODIFIERS.iter().rev() {
            if mods & bit != 0 {
                sink.emit(Synth {
                    key: *key,
                    down: false,
                });
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Slot {
    #[default]
    Idle,
    Passed,
    Held(Action),
    Orphan,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Swallow,
    Emergency,
}

pub struct Engine {
    enabled: bool,
    table: Arc<Table>,
    slots: Box<[Slot; SLOTS]>,
    held: usize,
    ctrl: u8,
    alt: u8,
}
impl Default for Engine {
    fn default() -> Self {
        Self {
            enabled: false,
            table: Arc::new(Table::default()),
            slots: Box::new([Slot::Idle; SLOTS]),
            held: 0,
            ctrl: 0,
            alt: 0,
        }
    }
}
impl Engine {
    #[cfg(test)]
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    #[cfg(test)]
    pub fn holding(&self) -> bool {
        self.held > 0
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Vec<Synth> {
        let released = if enabled { vec![] } else { self.release_all() };
        if enabled && !self.enabled {
            // A disabled engine gets no events, so orphans from the last
            // session were released on the system and are gone.
            self.slots.fill(Slot::Idle);
            self.ctrl = 0;
            self.alt = 0;
        }
        self.enabled = enabled;
        released
    }
    pub fn set_table(&mut self, table: Arc<Table>) -> Vec<Synth> {
        if Arc::ptr_eq(&self.table, &table) {
            return vec![];
        }
        let released = self.release_all();
        self.table = table;
        released
    }
    pub fn release_all(&mut self) -> Vec<Synth> {
        let mut out = vec![];
        if self.held == 0 {
            return out;
        }
        for slot in self.slots.iter_mut() {
            if let Slot::Held(action) = *slot {
                release(action, &mut out);
                *slot = Slot::Orphan;
            }
        }
        self.held = 0;
        out
    }
    pub fn suspend(&mut self) -> Vec<Synth> {
        self.ctrl = 0;
        self.alt = 0;
        self.release_all()
    }
    pub fn emergency(&mut self) -> Vec<Synth> {
        self.set_enabled(false)
    }
    /// The input desktop changed: releases made there never reach the hook.
    /// Forgets Ctrl and Alt and releases held output. A key that is still
    /// held sets its bit again with its next auto-repeat.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub fn reset_modifiers(&mut self) -> Vec<Synth> {
        self.suspend()
    }
    /// Whether this physical key's press was swallowed and is still tracked,
    /// so the system key state cannot report it. A disabled engine swallows
    /// nothing, so it never claims a key.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub fn holds(&self, source: Source) -> bool {
        self.enabled
            && source
                .slot()
                .is_some_and(|index| matches!(self.slots[index], Slot::Held(_) | Slot::Orphan))
    }
    /// Before an Esc press is read as Ctrl+Alt+Esc, drops Ctrl and Alt bits
    /// whose key was passed to the system and that it now reports up: their
    /// release went to another desktop. Swallowed keys are always kept.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub fn sync_modifiers(&mut self, source: Source, down: bool, physical: impl Fn(u16) -> bool) {
        if !down || source != Source::Key(ESCAPE) || self.ctrl == 0 || self.alt == 0 {
            return;
        }
        for (id, bit) in [(0x1D, 1), (0x11D, 2), (0x38, 1), (0x138, 2)] {
            let bits = if id & 0xFF == 0x1D {
                self.ctrl
            } else {
                self.alt
            };
            if bits & bit == 0 || self.holds(Source::Key(id)) || physical(id) {
                continue;
            }
            if id & 0xFF == 0x1D {
                self.ctrl &= !bit;
            } else {
                self.alt &= !bit;
            }
        }
    }

    pub fn handle(&mut self, source: Source, down: bool, out: &mut Out) -> Verdict {
        if let Source::Key(id) = source {
            let (bits, bit) = match id {
                0x1D => (&mut self.ctrl, 1),
                0x11D => (&mut self.ctrl, 2),
                0x38 => (&mut self.alt, 1),
                0x138 => (&mut self.alt, 2),
                _ => (&mut 0u8, 0),
            };
            if down {
                *bits |= bit;
            } else {
                *bits &= !bit;
            }
            if down && id == ESCAPE && self.ctrl != 0 && self.alt != 0 {
                return Verdict::Emergency;
            }
        }
        let Some(index) = source.slot() else {
            return Verdict::Pass;
        };
        match (self.slots[index], down) {
            (Slot::Held(action), true) => {
                if let Action::Send { key: Some(key), .. } = action {
                    out.push(Synth { key, down: true });
                }
                Verdict::Swallow
            }
            (Slot::Held(action), false) => {
                release(action, out);
                self.slots[index] = Slot::Idle;
                self.held -= 1;
                Verdict::Swallow
            }
            (Slot::Passed, true) => Verdict::Pass,
            (Slot::Passed, false) | (Slot::Idle, false) => {
                self.slots[index] = Slot::Idle;
                Verdict::Pass
            }
            (Slot::Orphan, false) => {
                self.slots[index] = Slot::Idle;
                Verdict::Swallow
            }
            (Slot::Idle | Slot::Orphan, true) => {
                let action = if self.enabled {
                    self.table.slots[index]
                } else {
                    Action::Pass
                };
                if action == Action::Pass {
                    self.slots[index] = Slot::Passed;
                    return Verdict::Pass;
                }
                press(action, out);
                self.slots[index] = Slot::Held(action);
                self.held += 1;
                Verdict::Swallow
            }
        }
    }
}

static ACTIVE: AtomicBool = AtomicBool::new(false);
static LAST_FOREGROUND: AtomicUsize = AtomicUsize::new(0);
static COMPILED: Mutex<Option<Compiled>> = Mutex::new(None);
static INFO: Mutex<Option<ActiveInfo>> = Mutex::new(None);
static SIGNAL: OnceLock<tokio::sync::mpsc::UnboundedSender<()>> = OnceLock::new();
#[derive(Clone, Default)]
struct ActiveInfo {
    app: Option<String>,
    ids: Vec<String>,
    paused: bool,
}

fn engine() -> &'static Mutex<Engine> {
    static ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();
    ENGINE.get_or_init(Default::default)
}
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn try_engine() -> Option<MutexGuard<'static, Engine>> {
    for _ in 0..256 {
        match engine().try_lock() {
            Ok(guard) => return Some(guard),
            Err(TryLockError::Poisoned(e)) => return Some(e.into_inner()),
            Err(TryLockError::WouldBlock) => std::hint::spin_loop(),
        }
    }
    None
}

#[cfg(target_os = "windows")]
pub(crate) fn hook_key(scan: u32, extended: bool, down: bool, forwarding: bool) -> bool {
    if scan == 0 || scan > 0xFF {
        return false;
    }
    hook(
        Source::Key(scan as u16 | (u16::from(extended) << 8)),
        down,
        forwarding,
    )
}
#[cfg(target_os = "windows")]
pub(crate) fn hook_mouse(button: MouseButton, down: bool, forwarding: bool) -> bool {
    hook(Source::Mouse(button), down, forwarding)
}
#[cfg(target_os = "windows")]
fn hook(source: Source, down: bool, forwarding: bool) -> bool {
    if !ACTIVE.load(Ordering::Acquire) {
        return false;
    }
    let Some(mut engine) = try_engine() else {
        return false;
    };
    if forwarding {
        let released = engine.suspend();
        drop(engine);
        platform::send(released.iter().copied());
        return false;
    }
    engine.sync_modifiers(source, down, platform::key_down);
    let mut out = Out::default();
    let verdict = engine.handle(source, down, &mut out);
    let emergency = (verdict == Verdict::Emergency).then(|| engine.emergency());
    drop(engine);
    platform::send(out.iter());
    if let Some(released) = emergency {
        ACTIVE.store(false, Ordering::Release);
        platform::send(released.iter().copied());
        if let Some(tx) = SIGNAL.get() {
            let _ = tx.send(());
        }
    }
    verdict == Verdict::Swallow
}

/// Whether remapping swallowed this physical key's press and still tracks it.
/// Counts as held when the engine is busy, so callers never drop a real press.
/// While remapping is off nothing is swallowed and the key state is exact.
#[cfg(target_os = "windows")]
pub(crate) fn holds_key(id: u32) -> bool {
    if !ACTIVE.load(Ordering::Acquire) {
        return false;
    }
    let Ok(id) = u16::try_from(id) else {
        return false;
    };
    try_engine().is_none_or(|engine| engine.holds(Source::Key(id)))
}
#[cfg(target_os = "windows")]
pub(crate) fn desktop_switched() {
    if !ACTIVE.load(Ordering::Acquire) {
        return;
    }
    let released = lock(engine()).reset_modifiers();
    platform::send(released.iter().copied());
}

pub(crate) fn refresh_foreground(force: bool) {
    if !ACTIVE.load(Ordering::Acquire) {
        return;
    }
    let window = platform::foreground_window();
    if LAST_FOREGROUND.swap(window, Ordering::AcqRel) == window && !force {
        return;
    }
    let foreground = platform::describe(window);
    let resolved = {
        let compiled = lock(&COMPILED);
        let Some(compiled) = compiled.as_ref() else {
            return;
        };
        if foreground.own {
            compiled.paused()
        } else {
            compiled.resolve(foreground.exe.as_deref())
        }
    };
    let released = lock(engine()).set_table(resolved.table);
    platform::send(released.iter().copied());
    *lock(&INFO) = Some(ActiveInfo {
        app: foreground.exe,
        ids: resolved.ids,
        paused: foreground.own,
    });
}

pub fn apply(core: &Core) -> Result<(), String> {
    let (enabled, profiles) = {
        let data = core.data.lock().map_err(|_| "Settings lock unavailable.")?;
        (data.settings.remap_enabled, data.profiles.clone())
    };
    *lock(&COMPILED) = Some(Compiled::new(&profiles));
    if enabled && supported() {
        if !ACTIVE.load(Ordering::Acquire) {
            platform::acquire()?;
            let released = lock(engine()).set_enabled(true);
            platform::send(released.iter().copied());
            ACTIVE.store(true, Ordering::Release);
        }
        refresh_foreground(true);
    } else {
        stop();
    }
    Ok(())
}
fn stop() {
    ACTIVE.store(false, Ordering::Release);
    let released = lock(engine()).set_enabled(false);
    platform::send(released.iter().copied());
    platform::release();
    LAST_FOREGROUND.store(0, Ordering::Release);
    *lock(&INFO) = None;
}

pub fn release_held() {
    let released = lock(engine()).release_all();
    platform::send(released.iter().copied());
}

fn persist_enabled(core: &Core, enabled: bool) -> Result<(), String> {
    let mut data = core.data.lock().map_err(|_| "Settings lock unavailable.")?;
    if data.settings.remap_enabled == enabled {
        return Ok(());
    }
    let mut updated = data.clone();
    updated.settings.remap_enabled = enabled;
    storage::save(&core.directory, &updated)?;
    *data = updated;
    Ok(())
}

pub fn set_enabled(core: &Core, enabled: bool) -> Result<RemapStatus, String> {
    if enabled && !supported() {
        return Err(UNSUPPORTED.into());
    }
    persist_enabled(core, enabled)?;
    if let Err(error) = apply(core) {
        let _ = persist_enabled(core, false);
        stop();
        return Err(error);
    }
    Ok(status(core))
}

pub fn emergency_off(core: &Core) {
    stop();
    let _ = persist_enabled(core, false);
}

pub fn init(core: Arc<Core>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    if SIGNAL.set(tx).is_ok() {
        let listener = core.clone();
        tauri::async_runtime::spawn(async move {
            while rx.recv().await.is_some() {
                crate::emergency_stop_core(&listener);
            }
        });
    }
    if let Err(error) = apply(&core) {
        core.warning(format!(
            "Remapping could not be turned on: {error} It stays off."
        ));
        let _ = persist_enabled(&core, false);
        stop();
    }
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveProfile {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemapStatus {
    pub supported: bool,
    pub reason: Option<String>,
    pub enabled: bool,
    pub running: bool,
    pub hook_installed: bool,
    pub active_app: Option<String>,
    pub paused: bool,
    pub active_profiles: Vec<ActiveProfile>,
    pub profiles: Vec<ProfileState>,
}
pub fn status(core: &Core) -> RemapStatus {
    let (enabled, profiles) = core
        .data
        .lock()
        .map(|d| (d.settings.remap_enabled, d.profiles.clone()))
        .unwrap_or_default();
    let running = ACTIVE.load(Ordering::Acquire);
    let info = if running {
        lock(&INFO).clone().unwrap_or_default()
    } else {
        ActiveInfo::default()
    };
    let states = lock(&COMPILED)
        .as_ref()
        .map(|c| c.profiles.clone())
        .unwrap_or_else(|| Compiled::new(&profiles).profiles);
    RemapStatus {
        supported: supported(),
        reason: (!supported()).then(|| UNSUPPORTED.into()),
        enabled,
        running,
        hook_installed: running && platform::installed(),
        active_app: info.app,
        paused: info.paused,
        active_profiles: info
            .ids
            .iter()
            .filter_map(|id| {
                profiles
                    .iter()
                    .find(|p| &p.id == id)
                    .map(|p| ActiveProfile {
                        id: p.id.clone(),
                        name: p.name.clone(),
                    })
            })
            .collect(),
        profiles: states,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningApp {
    pub exe: String,
    pub title: String,
}

#[tauri::command]
pub fn get_remap_status(state: tauri::State<'_, Arc<Core>>) -> RemapStatus {
    status(state.inner())
}
#[tauri::command]
pub async fn set_remap_enabled(
    enabled: bool,
    state: tauri::State<'_, Arc<Core>>,
) -> Result<RemapStatus, String> {
    let core = state.inner().clone();
    tokio::task::spawn_blocking(move || set_enabled(&core, enabled))
        .await
        .map_err(|_| "Remapping task failed.".to_string())?
}
#[tauri::command]
pub async fn list_running_apps() -> Result<Vec<RunningApp>, String> {
    if !supported() {
        return Err(UNSUPPORTED.into());
    }
    tokio::task::spawn_blocking(platform::running_apps)
        .await
        .map_err(|_| "Application list failed.".to_string())
}

pub(crate) struct Foreground {
    pub exe: Option<String>,
    pub own: bool,
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::{Foreground, RunningApp, Synth};
    pub fn send(_: impl Iterator<Item = Synth>) {}
    pub fn foreground_window() -> usize {
        0
    }
    pub fn describe(_: usize) -> Foreground {
        Foreground {
            exe: None,
            own: false,
        }
    }
    pub fn acquire() -> Result<(), String> {
        Err(super::UNSUPPORTED.into())
    }
    pub fn release() {}
    pub fn installed() -> bool {
        false
    }
    pub fn running_apps() -> Vec<RunningApp> {
        vec![]
    }
}

#[cfg(target_os = "windows")]
pub(crate) mod platform {
    use super::{Foreground, RunningApp, Synth};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HWND, LPARAM},
        System::Threading::{
            GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::{
            Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK},
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
                KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, VK_LCONTROL, VK_LMENU,
                VK_RCONTROL, VK_RMENU,
            },
            WindowsAndMessaging::*,
        },
    };
    const EXTRA_INFO: usize = 0x5452_4C4B;

    pub fn send(events: impl Iterator<Item = Synth>) {
        let mut inputs = [INPUT::default(); 8];
        let mut count = 0;
        let flush = |inputs: &[INPUT]| {
            if !inputs.is_empty() {
                unsafe {
                    SendInput(
                        inputs.len() as u32,
                        inputs.as_ptr(),
                        std::mem::size_of::<INPUT>() as i32,
                    )
                };
            }
        };
        for synth in events {
            let mut flags = if synth.key.vk == 0 {
                KEYEVENTF_SCANCODE
            } else {
                0
            };
            if synth.key.extended() {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }
            if !synth.down {
                flags |= KEYEVENTF_KEYUP;
            }
            inputs[count] = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: u16::from(synth.key.vk),
                        wScan: synth.key.scan(),
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: EXTRA_INFO,
                    },
                },
            };
            count += 1;
            if count == inputs.len() {
                flush(&inputs);
                count = 0;
            }
        }
        flush(&inputs[..count]);
    }

    fn process_exe(pid: u32) -> Option<String> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let mut buffer = [0u16; 1024];
            let mut size = buffer.len() as u32;
            let ok = QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                buffer.as_mut_ptr(),
                &mut size,
            );
            CloseHandle(handle);
            if ok == 0 {
                return None;
            }
            let path = String::from_utf16_lossy(&buffer[..size as usize]);
            path.rsplit(['\\', '/'])
                .next()
                .map(|name| name.to_lowercase())
                .filter(|name| !name.is_empty())
        }
    }
    fn window_process(window: HWND) -> u32 {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(window, &mut pid) };
        pid
    }
    pub fn foreground_window() -> usize {
        unsafe { GetForegroundWindow() as usize }
    }
    /// Whether Windows reports this Ctrl or Alt key down. Other ids count as
    /// down, so they are never forgotten.
    pub fn key_down(id: u16) -> bool {
        let vk = match id {
            0x1D => VK_LCONTROL,
            0x11D => VK_RCONTROL,
            0x38 => VK_LMENU,
            0x138 => VK_RMENU,
            _ => return true,
        };
        unsafe { GetAsyncKeyState(i32::from(vk)) as u16 & 0x8000 != 0 }
    }
    pub fn describe(window: usize) -> Foreground {
        if window == 0 {
            return Foreground {
                exe: None,
                own: false,
            };
        }
        let pid = window_process(window as HWND);
        let own = pid != 0 && pid == unsafe { GetCurrentProcessId() };
        Foreground {
            exe: (pid != 0 && !own).then(|| process_exe(pid)).flatten(),
            own,
        }
    }

    pub fn acquire() -> Result<(), String> {
        crate::flow::hooks::acquire(crate::flow::hooks::USER_REMAP)
    }
    pub fn release() {
        crate::flow::hooks::release(crate::flow::hooks::USER_REMAP);
    }
    pub fn installed() -> bool {
        crate::flow::hooks::installed()
    }

    unsafe extern "system" fn foreground_proc(
        _: HWINEVENTHOOK,
        _: u32,
        _: HWND,
        _: i32,
        _: i32,
        _: u32,
        _: u32,
    ) {
        super::refresh_foreground(false);
    }
    pub fn install_foreground_hook() -> HWINEVENTHOOK {
        unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                std::ptr::null_mut(),
                Some(foreground_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        }
    }
    pub fn remove_foreground_hook(hook: HWINEVENTHOOK) {
        if !hook.is_null() {
            unsafe { UnhookWinEvent(hook) };
        }
    }

    struct Collect {
        own: u32,
        apps: Vec<RunningApp>,
    }
    unsafe extern "system" fn enum_proc(window: HWND, lparam: LPARAM) -> i32 {
        let collect = &mut *(lparam as *mut Collect);
        if collect.apps.len() >= 256
            || IsWindowVisible(window) == 0
            || !GetWindow(window, GW_OWNER).is_null()
            || GetWindowLongW(window, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0
        {
            return 1;
        }
        let length = GetWindowTextLengthW(window);
        if length <= 0 {
            return 1;
        }
        let pid = window_process(window);
        if pid == 0 || pid == collect.own {
            return 1;
        }
        let Some(exe) = process_exe(pid) else {
            return 1;
        };
        if collect.apps.iter().any(|a| a.exe == exe) {
            return 1;
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32);
        let title: String = String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
            .chars()
            .take(120)
            .collect();
        collect.apps.push(RunningApp { exe, title });
        1
    }
    pub fn running_apps() -> Vec<RunningApp> {
        let mut collect = Collect {
            own: unsafe { GetCurrentProcessId() },
            apps: vec![],
        };
        unsafe { EnumWindows(Some(enum_proc), &mut collect as *mut Collect as LPARAM) };
        collect.apps.sort_by(|a, b| a.exe.cmp(&b.exe));
        collect.apps
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::KeyMapping;

    fn profile(app: Option<&str>, mappings: &[(&str, &str)]) -> Profile {
        Profile {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Teste".into(),
            application: app.map(Into::into),
            device_id: None,
            mappings: mappings
                .iter()
                .map(|(from, to)| KeyMapping {
                    from: (*from).into(),
                    to: (*to).into(),
                })
                .collect(),
            created_at: crate::model::now(),
            updated_at: crate::model::now(),
        }
    }
    fn key(code: &str) -> KeyOut {
        key_by_code(code).unwrap()
    }
    fn src(code: &str) -> Source {
        Source::Key(key(code).id)
    }
    fn table(mappings: &[(&str, &str)]) -> Arc<Table> {
        Compiled::new(&[profile(None, mappings)])
            .resolve(None)
            .table
    }
    fn engine_with(mappings: &[(&str, &str)]) -> Engine {
        let mut engine = Engine::default();
        engine.set_enabled(true);
        engine.set_table(table(mappings));
        engine
    }
    fn run(engine: &mut Engine, source: Source, down: bool) -> (Verdict, Vec<Synth>) {
        let mut out = Out::default();
        let verdict = engine.handle(source, down, &mut out);
        (verdict, out.iter().collect())
    }
    fn d(code: &str) -> Synth {
        Synth {
            key: key(code),
            down: true,
        }
    }
    fn u(code: &str) -> Synth {
        Synth {
            key: key(code),
            down: false,
        }
    }

    #[test]
    fn parses_sources_targets_and_legacy_strings() {
        assert_eq!(parse_source("KeyA"), Some(src("KeyA")));
        assert_eq!(parse_source("f1"), Some(src("F1")));
        assert_eq!(parse_source("a"), Some(src("KeyA")));
        assert_eq!(parse_source("Win"), Some(src("MetaLeft")));
        assert_eq!(
            parse_source("MouseBack"),
            Some(Source::Mouse(MouseButton::Back))
        );
        assert_eq!(
            parse_source("mouse5"),
            Some(Source::Mouse(MouseButton::Forward))
        );
        assert_eq!(
            parse_source("Ctrl+F1"),
            None,
            "combinations are not sources"
        );
        assert_eq!(parse_source("abrir navegador"), None);
        assert_eq!(parse_target("disabled"), Some(Action::Disable));
        assert_eq!(parse_target("Desativar"), Some(Action::Disable));
        assert_eq!(
            parse_target("KeyB"),
            Some(Action::Send {
                mods: 0,
                key: Some(key("KeyB"))
            })
        );
        assert_eq!(
            parse_target("Ctrl+C"),
            Some(Action::Send {
                mods: MOD_CTRL,
                key: Some(key("KeyC"))
            })
        );
        assert_eq!(
            parse_target("ctrl + shift + Escape"),
            Some(Action::Send {
                mods: MOD_CTRL | MOD_SHIFT,
                key: Some(key("Escape"))
            })
        );
        assert_eq!(parse_target("KeyA+KeyB"), None);
        assert_eq!(parse_target("Ctrl+"), None);
        assert_eq!(parse_target(""), None);
    }

    #[test]
    fn exe_names_match_case_insensitively_and_ignore_folders() {
        let roblox = profile(Some("RobloxPlayerBeta.exe"), &[("KeyQ", "KeyE")]);
        let compiled = Compiled::new(&[roblox.clone()]);
        for exe in [
            r"C:\Users\x\AppData\Local\Roblox\Versions\version-1a2b3c\RobloxPlayerBeta.exe",
            r"C:\Users\x\AppData\Local\Roblox\Versions\version-9f8e7d\robloxplayerbeta.exe",
            "ROBLOXPLAYERBETA.EXE",
            "robloxplayerbeta",
        ] {
            let resolved = compiled.resolve(Some(exe));
            assert_eq!(resolved.ids, vec![roblox.id.clone()], "{exe}");
            assert_eq!(
                resolved.table.get(src("KeyQ")),
                parse_target("KeyE").unwrap()
            );
        }
        assert!(compiled.resolve(Some("notepad.exe")).ids.is_empty());
        assert_eq!(compiled.resolve(None).table.get(src("KeyQ")), Action::Pass);
        let by_path = profile(
            Some(r"C:\Roblox\Versions\version-old\RobloxPlayerBeta.exe"),
            &[],
        );
        let compiled = Compiled::new(&[by_path.clone()]);
        assert_eq!(
            compiled
                .resolve(Some(r"D:\Roblox\Versions\version-new\RobloxPlayerBeta.exe"))
                .ids,
            vec![by_path.id]
        );
        assert_eq!(normalize_app("  \"Blender\"  ").as_deref(), Some("blender"));
        assert_eq!(normalize_app(r"C:\x\"), None);
    }

    #[test]
    fn app_profile_overlays_default_and_device_profiles_are_not_applied() {
        let all = profile(None, &[("MetaLeft", "disabled"), ("KeyQ", "KeyW")]);
        let game = profile(Some("game.exe"), &[("KeyQ", "KeyE"), ("bogus", "KeyA")]);
        let mut device = profile(None, &[("KeyZ", "KeyX")]);
        device.device_id = Some("hid-1".into());
        let compiled = Compiled::new(&[all.clone(), game.clone(), device.clone()]);
        let resolved = compiled.resolve(Some("Game.exe"));
        assert_eq!(resolved.ids, vec![all.id.clone(), game.id.clone()]);
        assert_eq!(resolved.table.get(src("MetaLeft")), Action::Disable);
        assert_eq!(
            resolved.table.get(src("KeyQ")),
            parse_target("KeyE").unwrap()
        );
        assert_eq!(resolved.table.get(src("KeyZ")), Action::Pass);
        let other = compiled.resolve(Some("other.exe"));
        assert_eq!(other.table.get(src("KeyQ")), parse_target("KeyW").unwrap());
        let state = |id: &str| compiled.profiles.iter().find(|s| s.id == id).unwrap();
        assert_eq!(state(&game.id).ignored, vec![1]);
        assert!(!state(&device.id).applied);
        assert_eq!(state(&device.id).reason, Some("device_filter"));
        assert!(compiled.paused().table.get(src("MetaLeft")) == Action::Pass);
    }

    #[test]
    fn key_to_key_pairs_down_and_up() {
        let mut engine = engine_with(&[("KeyQ", "KeyE")]);
        assert_eq!(
            run(&mut engine, src("KeyQ"), true),
            (Verdict::Swallow, vec![d("KeyE")])
        );
        assert_eq!(
            run(&mut engine, src("KeyQ"), true),
            (Verdict::Swallow, vec![d("KeyE")])
        );
        assert_eq!(
            run(&mut engine, src("KeyQ"), false),
            (Verdict::Swallow, vec![u("KeyE")])
        );
        assert!(!engine.holding());
        assert_eq!(run(&mut engine, src("KeyA"), true), (Verdict::Pass, vec![]));
        assert_eq!(
            run(&mut engine, src("KeyA"), false),
            (Verdict::Pass, vec![])
        );
    }

    #[test]
    fn shortcut_presses_modifiers_first_and_releases_in_reverse() {
        let mut engine = engine_with(&[("MouseBack", "Ctrl+Shift+KeyC")]);
        let back = Source::Mouse(MouseButton::Back);
        assert_eq!(
            run(&mut engine, back, true),
            (
                Verdict::Swallow,
                vec![d("ControlLeft"), d("ShiftLeft"), d("KeyC")]
            )
        );
        assert_eq!(
            run(&mut engine, back, false),
            (
                Verdict::Swallow,
                vec![u("KeyC"), u("ShiftLeft"), u("ControlLeft")]
            )
        );
    }

    #[test]
    fn disabled_swallows_both_edges_without_output() {
        let mut engine = engine_with(&[("MetaLeft", "disabled")]);
        assert_eq!(
            run(&mut engine, src("MetaLeft"), true),
            (Verdict::Swallow, vec![])
        );
        assert_eq!(
            run(&mut engine, src("MetaLeft"), false),
            (Verdict::Swallow, vec![])
        );
    }

    #[test]
    fn profile_switch_releases_held_output_and_swallows_the_orphan_up() {
        let mut engine = engine_with(&[("KeyQ", "Ctrl+KeyE")]);
        run(&mut engine, src("KeyQ"), true);
        let released = engine.set_table(table(&[("KeyQ", "KeyR")]));
        assert_eq!(released, vec![u("KeyE"), u("ControlLeft")]);
        assert!(!engine.holding());
        assert_eq!(
            run(&mut engine, src("KeyQ"), false),
            (Verdict::Swallow, vec![])
        );
        let mut engine = engine_with(&[]);
        assert_eq!(run(&mut engine, src("KeyA"), true), (Verdict::Pass, vec![]));
        engine.set_table(table(&[("KeyA", "KeyB")]));
        assert_eq!(run(&mut engine, src("KeyA"), true), (Verdict::Pass, vec![]));
        assert_eq!(
            run(&mut engine, src("KeyA"), false),
            (Verdict::Pass, vec![])
        );
        assert_eq!(
            run(&mut engine, src("KeyA"), true),
            (Verdict::Swallow, vec![d("KeyB")])
        );
        let same = table(&[]);
        engine.set_table(same.clone());
        assert!(engine.set_table(same).is_empty());
    }

    #[test]
    fn master_off_releases_and_stops_remapping() {
        let mut engine = engine_with(&[("KeyQ", "KeyE")]);
        run(&mut engine, src("KeyQ"), true);
        assert_eq!(engine.set_enabled(false), vec![u("KeyE")]);
        assert_eq!(
            run(&mut engine, src("KeyQ"), false),
            (Verdict::Swallow, vec![])
        );
        assert_eq!(run(&mut engine, src("KeyQ"), true), (Verdict::Pass, vec![]));
    }

    #[test]
    fn ctrl_alt_esc_is_never_remapped_and_triggers_emergency() {
        let mut engine = engine_with(&[
            ("Escape", "KeyA"),
            ("ControlLeft", "disabled"),
            ("KeyQ", "KeyE"),
        ]);
        run(&mut engine, src("KeyQ"), true);
        assert_eq!(
            run(&mut engine, src("ControlLeft"), true).0,
            Verdict::Swallow
        );
        assert_eq!(run(&mut engine, src("AltLeft"), true).0, Verdict::Pass);
        assert_eq!(
            run(&mut engine, src("Escape"), true),
            (Verdict::Emergency, vec![])
        );
        let released = engine.emergency();
        assert!(released.contains(&u("KeyE")));
        assert!(!engine.enabled());
        let mut engine = engine_with(&[("Escape", "KeyA")]);
        assert_eq!(
            run(&mut engine, src("Escape"), true),
            (Verdict::Swallow, vec![d("KeyA")])
        );
    }

    #[test]
    fn stale_modifiers_from_another_desktop_do_not_trigger_emergency() {
        let mut engine = engine_with(&[("KeyQ", "KeyE")]);
        run(&mut engine, src("ControlLeft"), true);
        run(&mut engine, src("AltRight"), true);
        // Ctrl+Alt+Del: both releases happen on the secure desktop.
        engine.sync_modifiers(src("Escape"), true, |_| false);
        assert_eq!(run(&mut engine, src("Escape"), true).0, Verdict::Pass);
        assert!(engine.enabled());
    }

    #[test]
    fn ctrl_alt_esc_still_triggers_after_the_key_state_check() {
        let mut engine = engine_with(&[]);
        run(&mut engine, src("ControlRight"), true);
        run(&mut engine, src("AltLeft"), true);
        engine.sync_modifiers(src("Escape"), true, |_| true);
        assert_eq!(run(&mut engine, src("Escape"), true).0, Verdict::Emergency);
        // A remapped modifier never reaches the key state, so it must count.
        let mut engine = engine_with(&[("ControlLeft", "disabled"), ("AltLeft", "KeyB")]);
        run(&mut engine, src("ControlLeft"), true);
        run(&mut engine, src("AltLeft"), true);
        engine.sync_modifiers(src("Escape"), true, |_| false);
        assert_eq!(run(&mut engine, src("Escape"), true).0, Verdict::Emergency);
        // Only one side is stale: the other Ctrl is really held.
        let mut engine = engine_with(&[]);
        run(&mut engine, src("ControlLeft"), true);
        run(&mut engine, src("ControlRight"), true);
        run(&mut engine, src("AltLeft"), true);
        engine.sync_modifiers(src("Escape"), true, |id| id != 0x1D);
        assert_eq!(run(&mut engine, src("Escape"), true).0, Verdict::Emergency);
    }

    #[test]
    fn desktop_switch_forgets_modifiers_and_releases_held_output() {
        let mut engine = engine_with(&[("KeyQ", "KeyE")]);
        run(&mut engine, src("ControlLeft"), true);
        run(&mut engine, src("AltLeft"), true);
        run(&mut engine, src("KeyQ"), true);
        assert!(engine.holds(src("KeyQ")) && !engine.holds(src("ControlLeft")));
        assert_eq!(engine.reset_modifiers(), vec![u("KeyE")]);
        assert_eq!(run(&mut engine, src("Escape"), true).0, Verdict::Pass);
        assert_eq!(
            run(&mut engine, src("KeyQ"), false),
            (Verdict::Swallow, vec![])
        );
        // Still held: the next auto-repeat restores the modifier.
        run(&mut engine, src("ControlLeft"), true);
        run(&mut engine, src("AltLeft"), true);
        assert_eq!(run(&mut engine, src("Escape"), true).0, Verdict::Emergency);
    }

    #[test]
    fn a_disabled_engine_never_claims_a_held_key() {
        let mut engine = engine_with(&[("ControlLeft", "disabled"), ("AltLeft", "KeyB")]);
        run(&mut engine, src("ControlLeft"), true);
        run(&mut engine, src("AltLeft"), true);
        assert!(engine.holds(src("ControlLeft")) && engine.holds(src("AltLeft")));
        // The emergency stop turns both into orphans, but a disabled engine
        // gets no more events, so the system key state is authoritative.
        assert_eq!(run(&mut engine, src("Escape"), true).0, Verdict::Emergency);
        engine.emergency();
        assert!(!engine.holds(src("ControlLeft")) && !engine.holds(src("AltLeft")));
        // Turning remapping back on starts from a clean state.
        engine.set_enabled(true);
        assert!(!engine.holds(src("ControlLeft")) && !engine.holds(src("AltLeft")));
        assert_eq!(
            run(&mut engine, src("AltLeft"), false),
            (Verdict::Pass, vec![])
        );
    }

    #[test]
    fn suspend_for_flow_releases_and_forgets_modifiers() {
        let mut engine = engine_with(&[("KeyQ", "KeyE")]);
        run(&mut engine, src("ControlLeft"), true);
        run(&mut engine, src("AltLeft"), true);
        run(&mut engine, src("KeyQ"), true);
        assert_eq!(engine.suspend(), vec![u("KeyE")]);
        assert_eq!(run(&mut engine, src("Escape"), true).0, Verdict::Pass);
    }

    #[test]
    fn invalid_slots_pass_through() {
        let mut engine = engine_with(&[("KeyQ", "KeyE")]);
        assert_eq!(
            run(&mut engine, Source::Key(0), true),
            (Verdict::Pass, vec![])
        );
        assert_eq!(
            run(&mut engine, Source::Key(0x2FF), true),
            (Verdict::Pass, vec![])
        );
    }

    #[test]
    fn every_code_is_unique_and_round_trips() {
        let mut ids = std::collections::HashSet::new();
        for def in KEYS {
            assert!(ids.insert(def.id), "{}", def.code);
            assert_eq!(key_by_code(def.code).map(|k| k.id), Some(def.id));
            assert!((def.id as usize) < KEY_SLOTS);
        }
    }

    #[test]
    fn legacy_settings_and_profiles_deserialize() {
        let json = r#"{"deviceName":"PC","authServerUrl":"https://traplink.traphub.xyz","clipboardEnabled":false,"fileSharingEnabled":false,"inputEnabled":false,"startOnLogin":false,"theme":"dark","downloadDirectory":"C:/Downloads","port":48372}"#;
        let settings: crate::model::Settings = serde_json::from_str(json).unwrap();
        assert!(!settings.remap_enabled, "remapping is off by default");
        let profile: Profile = serde_json::from_str(
            r#"{"id":"3f0e7c1e-2c1a-4f7a-9d55-8b1a7d0c1a11","name":"Antigo","application":"Blender","deviceId":null,"mappings":[{"from":"F1","to":"Ctrl+C"},{"from":"gesto","to":"abrir menu"}],"createdAt":"2024-01-01T00:00:00Z","updatedAt":"2024-01-01T00:00:00Z"}"#,
        )
        .unwrap();
        profile.validate().unwrap();
        let compiled = Compiled::new(&[profile.clone()]);
        assert_eq!(compiled.profiles[0].ignored, vec![1]);
        assert_eq!(
            compiled.resolve(Some("blender.exe")).table.get(src("F1")),
            parse_target("Ctrl+KeyC").unwrap()
        );
        let mut round = serde_json::to_value(&settings).unwrap();
        assert_eq!(round["remapEnabled"], false);
        round["remapEnabled"] = true.into();
        let back: crate::model::Settings = serde_json::from_value(round).unwrap();
        assert!(back.remap_enabled);
    }
}
