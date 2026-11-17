use crate::{
    flow::{FlowState, ReturnWatch},
    model::{FlowEdge, InputEvent},
    screen::Layout,
    state::Core,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[cfg(all(windows, not(test)))]
use crate::screen::win as windows_pointer;
#[cfg(all(windows, test))]
#[path = "input/test_display.rs"]
mod windows_pointer;

const SESSION_TIMEOUT: Duration = Duration::from_secs(3);
pub const INACTIVE: &str = "Input control is not active.";

#[derive(Default)]
pub struct InputState {
    pub sending_to: Option<String>,
    pub pending_control: Option<Uuid>,
    receiving: Option<Receiver>,
    // Enigo's macOS destructor waits for posted native events. Reuse one idle
    // injector so that drain never stalls a handoff or the encrypted LAN reader.
    #[cfg(target_os = "macos")]
    idle_injector: Option<enigo::Enigo>,
    pub flow: FlowState,
    pub last_return: Option<crate::flow::FlowTiming>,
}
impl InputState {
    pub fn busy(&self) -> bool {
        self.sending_to.is_some() || self.receiving.is_some()
    }
    fn finish_receiving(&mut self) {
        if let Some(mut receiver) = self.receiving.take() {
            receiver.release_all();
            #[cfg(target_os = "macos")]
            if receiver.codes.is_empty() && receiver.buttons.is_empty() {
                self.idle_injector = Some(receiver.enigo);
            }
        }
    }
}
impl Drop for InputState {
    fn drop(&mut self) {
        self.finish_receiving();
    }
}

struct ReceiverFlow {
    session: Option<Uuid>,
    watch: ReturnWatch,
    clipboard: bool,
    returned_at: Option<Instant>,
}
pub struct FlowReturnInfo {
    pub entry: f64,
    pub clipboard: bool,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
struct ClickSeries {
    last_down: Option<(Instant, (i32, i32))>,
    count: i64,
}
#[cfg(any(target_os = "macos", test))]
impl ClickSeries {
    fn update(&mut self, down: bool, point: (i32, i32), now: Instant, interval: Duration) -> i64 {
        if down {
            self.count = if self.last_down.is_some_and(|(last, (x, y))| {
                now.saturating_duration_since(last) < interval
                    && x.abs_diff(point.0) <= 4
                    && y.abs_diff(point.1) <= 4
            }) {
                self.count.saturating_add(1)
            } else {
                1
            };
            self.last_down = Some((now, point));
        }
        self.count.max(1)
    }
}

struct Receiver {
    peer: String,
    last_seen: Instant,
    flow: Option<ReceiverFlow>,
    layout: Option<(Layout, Instant)>,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    cursor: Option<((i32, i32), Instant)>,
    #[cfg(target_os = "macos")]
    clicks: [ClickSeries; 3],
    #[cfg(target_os = "macos")]
    double_click_interval: Duration,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    enigo: enigo::Enigo,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    codes: HashMap<String, enigo::Key>,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    buttons: HashSet<enigo::Button>,
}
impl Receiver {
    fn accept_activity(&mut self, now: Instant) -> Result<bool, String> {
        if now.saturating_duration_since(self.last_seen) > SESSION_TIMEOUT {
            return Err("Input control timed out; start it again.".into());
        }
        if self.flow.as_ref().is_some_and(|f| f.returned_at.is_some()) {
            return Ok(false);
        }
        self.last_seen = now;
        Ok(true)
    }
    fn matches(&self, peer: &str, session: Option<Uuid>) -> bool {
        self.peer == peer && self.flow.as_ref().and_then(|flow| flow.session) == session
    }
    fn release_all(&mut self) {
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        {
            use enigo::{Direction, Keyboard};
            let keys: HashSet<enigo::Key> = self.codes.values().copied().collect();
            for key in keys {
                if self.enigo.key(key, Direction::Release).is_ok() {
                    self.codes.retain(|_, held| *held != key);
                }
            }
            let buttons: Vec<_> = self.buttons.iter().copied().collect();
            for button in buttons {
                if self.button(button, false).is_ok() {
                    self.buttons.remove(&button);
                }
            }
        }
    }
}
pub fn enabled(core: &Core, peer: &str) -> Result<(), String> {
    let data = core.data.lock().map_err(|_| "Settings lock unavailable.")?;
    check_permission(&data, peer)
}
pub(super) fn check_permission(data: &crate::model::Persisted, peer: &str) -> Result<(), String> {
    if !data.settings.input_enabled
        || !data
            .peers
            .iter()
            .any(|p| p.peer.id == peer && p.peer.permissions.input)
    {
        return Err(
            "Input requires the global switch and this peer's input permission on both computers."
                .into(),
        );
    }
    Ok(())
}
pub fn reserve_input<'a>(
    core: &'a Core,
    peer: &str,
) -> Result<std::sync::MutexGuard<'a, InputState>, String> {
    let data = core.data.lock().map_err(|_| "Settings lock unavailable.")?;
    check_permission(&data, peer)?;
    core.input
        .lock()
        .map_err(|_| "Input lock unavailable.".into())
}
pub fn start_receiving(core: &Core, peer: &str) -> Result<(), String> {
    start_session(core, peer, None, None)
}
pub fn start_flow_receiving(
    core: &Core,
    peer: &str,
    session: Option<Uuid>,
    sender_edge: FlowEdge,
    entry: f64,
    clipboard: bool,
) -> Result<(), String> {
    crate::flow::validate_entry(entry)?;
    start_session(
        core,
        peer,
        Some(ReceiverFlow {
            session,
            watch: ReturnWatch::new(sender_edge),
            clipboard,
            returned_at: None,
        }),
        Some((sender_edge.opposite(), entry)),
    )
}
#[cfg(any(target_os = "windows", target_os = "macos"))]
impl Receiver {
    fn layout(&mut self) -> Result<&Layout, String> {
        let fresh = self
            .layout
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(2));
        if !fresh {
            self.layout = Some((platform_layout(&self.enigo)?, Instant::now()));
        }
        self.layout
            .as_ref()
            .map(|(layout, _)| layout)
            .ok_or_else(|| "Display layout is unavailable.".to_string())
    }
    fn position(&self) -> Result<(i32, i32), String> {
        // Both platforms queue injected events. Integrate bursts from the last
        // accepted point instead of reading a cursor still one event behind.
        if let Some((point, at)) = self.cursor {
            if at.elapsed() < INJECTED_CURSOR_TTL {
                return Ok(point);
            }
        }
        #[cfg(target_os = "windows")]
        return windows_pointer::cursor().ok_or_else(|| "Pointer position is unavailable.".into());
        #[cfg(target_os = "macos")]
        {
            let (x, y) = crate::screen::mac::cursor().ok_or("Pointer position is unavailable.")?;
            Ok((x.floor() as i32, y.floor() as i32))
        }
    }
    fn move_to(&mut self, point: (i32, i32)) -> Result<(), String> {
        #[cfg(target_os = "windows")]
        {
            let layout = self.layout()?;
            windows_pointer::move_absolute(point, layout)?;
            self.cursor = Some((point, Instant::now()));
            Ok(())
        }
        #[cfg(target_os = "macos")]
        {
            let from = self.position().unwrap_or(point);
            let drag = [
                (enigo::Button::Left, 0),
                (enigo::Button::Right, 1),
                (enigo::Button::Middle, 2),
            ]
            .into_iter()
            .find(|(button, _)| self.buttons.contains(button))
            .map(|(_, number)| number);
            if !crate::macos::post_mouse_move(point, (point.0 - from.0, point.1 - from.1), drag) {
                self.cursor = None;
                return Err("Native input injection failed: the pointer move was blocked.".into());
            }
            self.cursor = Some((point, Instant::now()));
            Ok(())
        }
    }
    fn button(&mut self, button: enigo::Button, down: bool) -> enigo::InputResult<()> {
        #[cfg(target_os = "windows")]
        {
            use enigo::{Direction, Mouse};
            self.enigo.button(
                button,
                if down {
                    Direction::Press
                } else {
                    Direction::Release
                },
            )
        }
        #[cfg(target_os = "macos")]
        {
            let number = match button {
                enigo::Button::Left => 0,
                enigo::Button::Right => 1,
                enigo::Button::Middle => 2,
                _ => return Err(enigo::InputError::Simulate("unsupported mouse button")),
            };
            // Even if a display disappears between the press and the release,
            // send the up at the last posted point to balance the held button.
            let point = self
                .position()
                .ok()
                .or_else(|| {
                    if down {
                        None
                    } else {
                        self.cursor.map(|(point, _)| point)
                    }
                })
                .ok_or(enigo::InputError::Simulate(
                    "could not read the pointer position",
                ))?;
            let now = Instant::now();
            let clicks =
                self.clicks[number as usize].update(down, point, now, self.double_click_interval);
            if !crate::macos::post_mouse_button(point, number, down, clicks) {
                return Err(enigo::InputError::Simulate(
                    "could not post the mouse button",
                ));
            }
            self.cursor = Some((point, now));
            Ok(())
        }
    }
}
#[cfg(any(target_os = "windows", target_os = "macos"))]
const INJECTED_CURSOR_TTL: Duration = Duration::from_millis(250);
#[cfg(target_os = "windows")]
fn platform_layout(_: &enigo::Enigo) -> Result<Layout, String> {
    windows_pointer::layout().ok_or_else(|| "Display layout is unavailable.".to_string())
}
#[cfg(target_os = "macos")]
fn platform_layout(_: &enigo::Enigo) -> Result<Layout, String> {
    crate::screen::mac::layout().ok_or_else(|| "Display layout is unavailable.".to_string())
}
#[cfg_attr(
    not(any(target_os = "windows", target_os = "macos")),
    allow(unused_variables, unused_mut)
)]
fn start_session(
    core: &Core,
    peer: &str,
    flow: Option<ReceiverFlow>,
    entry: Option<(FlowEdge, f64)>,
) -> Result<(), String> {
    let mut state = reserve_input(core, peer)?;
    if state.sending_to.is_some() {
        return Err(
            "This computer is already controlling another peer; stop it before receiving input."
                .into(),
        );
    }
    // A new, tagged start from the same peer can follow its return before the
    // old stop arrives. A late stop for that previous visit is harmless.
    if state.receiving.as_ref().is_some_and(|r| {
        r.peer == peer
            && r.flow.as_ref().is_some_and(|old| {
                old.returned_at.is_some()
                    && old.session.is_some()
                    && flow
                        .as_ref()
                        .is_some_and(|new| new.session.is_some() && new.session != old.session)
            })
    }) {
        state.finish_receiving();
    }
    if state.receiving.is_some() {
        return Err("An input session is already active on this computer.".into());
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        let settings = enigo::Settings {
            open_prompt_to_get_permissions: false,
            release_keys_when_dropped: true,
            #[cfg(target_os = "macos")]
            event_source_user_data: Some(crate::macos::INJECTED_MARKER),
            ..Default::default()
        };
        let new_injector = || {
            enigo::Enigo::new(&settings).map_err(|_| "Input access is unavailable. On macOS, allow TrapLink in System Settings > Privacy & Security > Accessibility, then retry.".to_string())
        };
        #[cfg(target_os = "macos")]
        let enigo = {
            if !crate::macos::accessibility_trusted(false) {
                return Err(
                    "Input access is unavailable. Allow TrapLink in macOS Accessibility settings."
                        .into(),
                );
            }
            state.idle_injector.take().map_or_else(new_injector, Ok)?
        };
        #[cfg(target_os = "windows")]
        let enigo = new_injector()?;
        let mut receiver = Receiver {
            peer: peer.into(),
            last_seen: Instant::now(),
            flow,
            layout: None,
            cursor: None,
            #[cfg(target_os = "macos")]
            clicks: std::array::from_fn(|_| ClickSeries::default()),
            #[cfg(target_os = "macos")]
            double_click_interval: crate::macos::double_click_interval(),
            enigo,
            codes: HashMap::new(),
            buttons: HashSet::new(),
        };
        if let Some((edge, fraction)) = entry {
            let point = receiver
                .layout()?
                .entry_point(edge, fraction, crate::flow::EDGE_INSET_PX);
            receiver.move_to(point)?;
        }
        state.receiving = Some(receiver);
        Ok(())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    Err("Input receiving is supported on Windows and macOS only.".into())
}
pub fn active_with(core: &Core, peer: &str) -> bool {
    core.input
        .lock()
        .is_ok_and(|state| state.active_peer() == Some(peer))
}
pub fn stop_peer(core: &Core, peer: &str) -> bool {
    if let Ok(mut state) = core.input.lock() {
        let active = state.active_peer() == Some(peer);
        if state.receiving.as_ref().is_some_and(|r| r.peer == peer) {
            state.finish_receiving();
        }
        if state.sending_to.as_deref() == Some(peer) {
            state.sending_to = None;
            state.pending_control = None;
        }
        crate::flow::session_ended(&mut state.flow, peer);
        return active;
    }
    false
}
pub fn stop_receiving(core: &Core, peer: &str, session: Option<Uuid>) {
    if let Ok(mut state) = core.input.lock() {
        if state
            .receiving
            .as_ref()
            .is_some_and(|r| r.matches(peer, session))
        {
            state.finish_receiving();
        }
    }
}
pub fn stop_focused(core: &Core, peer: &str) {
    if let Ok(mut state) = core.input.lock() {
        if state
            .receiving
            .as_ref()
            .is_some_and(|r| r.peer == peer && r.flow.is_none())
        {
            state.finish_receiving();
        }
        if state.sending_to.as_deref() == Some(peer) && state.flow.switch.peer().is_none() {
            state.sending_to = None;
            state.pending_control = None;
        }
    }
}
impl InputState {
    fn active_peer(&self) -> Option<&str> {
        self.sending_to
            .as_deref()
            .or_else(|| self.receiving.as_ref().map(|r| r.peer.as_str()))
            .or_else(|| self.flow.switch.peer())
    }
    pub fn acknowledge_stop(&mut self, peer: &str, token: Uuid) {
        if let Some(start) = self
            .receiving
            .as_ref()
            .filter(|r| r.matches(peer, Some(token)))
            .and_then(|r| r.flow.as_ref()?.returned_at)
        {
            self.last_return = Some(crate::flow::FlowTiming {
                peer: peer.to_owned(),
                elapsed_ms: start.elapsed().as_millis() as u64,
            });
        }
        self.stop_flow(peer, token);
    }
    pub fn stop_flow(&mut self, peer: &str, token: Uuid) -> bool {
        if self
            .receiving
            .as_ref()
            .is_some_and(|r| r.matches(peer, Some(token)))
        {
            self.finish_receiving();
            return true;
        }
        if self.flow.switch.peer() == Some(peer) && self.flow.switch_id() == Some(token) {
            self.sending_to = None;
            crate::flow::session_ended(&mut self.flow, peer);
            return true;
        }
        false
    }
}
pub fn stop_all(core: &Core) -> Option<String> {
    if let Ok(mut state) = core.input.lock() {
        let peer = state.active_peer().map(str::to_owned);
        state.finish_receiving();
        state.sending_to = None;
        state.pending_control = None;
        crate::flow::disarm_all(&mut state.flow);
        return peer;
    }
    None
}

pub fn keepalive(core: &Core, peer: &str, session: Option<Uuid>) -> Result<(), String> {
    enabled(core, peer)?;
    let mut state = core.input.lock().map_err(|_| "Input lock unavailable.")?;
    let receiver = state
        .receiving
        .as_mut()
        .filter(|r| r.matches(peer, session))
        .ok_or(INACTIVE)?;
    receiver.accept_activity(Instant::now()).map(|_| ())
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn key(code: &str) -> Option<enigo::Key> {
    physical_key(code).or_else(|| legacy_key(code))
}
#[cfg(target_os = "macos")]
fn physical_key(code: &str) -> Option<enigo::Key> {
    crate::keymap::mac_for_code(code, crate::macos::iso_keyboard(None))
        .map(|kvk| enigo::Key::Other(u32::from(kvk)))
}
#[cfg(target_os = "windows")]
fn physical_key(code: &str) -> Option<enigo::Key> {
    use enigo::Key;
    Some(match code {
        "Numpad0" => Key::Numpad0,
        "Numpad1" => Key::Numpad1,
        "Numpad2" => Key::Numpad2,
        "Numpad3" => Key::Numpad3,
        "Numpad4" => Key::Numpad4,
        "Numpad5" => Key::Numpad5,
        "Numpad6" => Key::Numpad6,
        "Numpad7" => Key::Numpad7,
        "Numpad8" => Key::Numpad8,
        "Numpad9" => Key::Numpad9,
        "NumpadDecimal" => Key::Decimal,
        "NumpadAdd" => Key::Add,
        "NumpadSubtract" => Key::Subtract,
        "NumpadMultiply" => Key::Multiply,
        "NumpadDivide" => Key::Divide,
        "NumpadEnter" => Key::Return,
        _ => {
            let (scan, extended) = crate::keymap::scan_for_code(code)?;
            let vk = crate::screen::win::vk_for_scan(scan, extended)?;
            Key::Other(u32::from(vk))
        }
    })
}
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn known_key(code: &str) -> bool {
    key(code).is_some()
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn known_key(_: &str) -> bool {
    true
}
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn legacy_key(code: &str) -> Option<enigo::Key> {
    use enigo::Key;
    if code.len() == 4 && code.starts_with("Key") {
        let c = code.as_bytes()[3];
        if c.is_ascii_uppercase() {
            return Some(Key::Unicode((c as char).to_ascii_lowercase()));
        }
    }
    if code.len() == 6 && code.starts_with("Digit") {
        let c = code.as_bytes()[5];
        if c.is_ascii_digit() {
            return Some(Key::Unicode(c as char));
        }
    }
    Some(match code {
        "Enter" | "NumpadEnter" => Key::Return,
        "Space" => Key::Space,
        "Tab" => Key::Tab,
        "Backspace" => Key::Backspace,
        "Delete" => Key::Delete,
        "Escape" => Key::Escape,
        "CapsLock" => Key::CapsLock,
        "ShiftLeft" | "ShiftRight" => Key::Shift,
        "ControlLeft" | "ControlRight" => Key::Control,
        "AltLeft" | "AltRight" => Key::Alt,
        "MetaLeft" | "MetaRight" => Key::Meta,
        "ArrowLeft" => Key::LeftArrow,
        "ArrowRight" => Key::RightArrow,
        "ArrowUp" => Key::UpArrow,
        "ArrowDown" => Key::DownArrow,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "F1" => Key::F1,
        "F2" => Key::F2,
        "F3" => Key::F3,
        "F4" => Key::F4,
        "F5" => Key::F5,
        "F6" => Key::F6,
        "F7" => Key::F7,
        "F8" => Key::F8,
        "F9" => Key::F9,
        "F10" => Key::F10,
        "F11" => Key::F11,
        "F12" => Key::F12,
        "Minus" => Key::Unicode('-'),
        "Equal" => Key::Unicode('='),
        "BracketLeft" => Key::Unicode('['),
        "BracketRight" => Key::Unicode(']'),
        "Backslash" => Key::Unicode('\\'),
        "Semicolon" => Key::Unicode(';'),
        "Quote" => Key::Unicode('\''),
        "Comma" => Key::Unicode(','),
        "Period" => Key::Unicode('.'),
        "Slash" => Key::Unicode('/'),
        "Backquote" => Key::Unicode('`'),
        _ => return None,
    })
}
pub fn validate(event: &InputEvent) -> Result<(), String> {
    match event {
        InputEvent::Key { key, .. } if key.len() <= 32 => {
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            if self::key(key).is_none() {
                return Err("This key is unsupported in focused control mode.".into());
            }
            Ok(())
        }
        InputEvent::Move { x, y }
            if x.is_finite()
                && y.is_finite()
                && (0.0..=1.0).contains(x)
                && (0.0..=1.0).contains(y) =>
        {
            Ok(())
        }
        InputEvent::Button { button, .. } if *button <= 2 => Ok(()),
        InputEvent::Scroll { x, y } if (-10..=10).contains(x) && (-10..=10).contains(y) => Ok(()),
        InputEvent::MoveBy { dx, dy }
            if (-crate::flow::MAX_DELTA..=crate::flow::MAX_DELTA).contains(dx)
                && (-crate::flow::MAX_DELTA..=crate::flow::MAX_DELTA).contains(dy) =>
        {
            Ok(())
        }
        _ => Err("Invalid input event.".into()),
    }
}
pub fn receive(
    core: &Core,
    peer: &str,
    session: Option<Uuid>,
    event: InputEvent,
) -> Result<Option<FlowReturnInfo>, String> {
    enabled(core, peer)?;
    let unknown_key =
        matches!(&event, InputEvent::Key { key, .. } if key.len() <= 32 && !known_key(key));
    if !unknown_key {
        validate(&event)?;
    }
    let mut state = core.input.lock().map_err(|_| "Input lock unavailable.")?;
    let receiver = state
        .receiving
        .as_mut()
        .filter(|r| r.matches(peer, session))
        .ok_or(INACTIVE)?;
    if !receiver.accept_activity(Instant::now())? || unknown_key {
        return Ok(None);
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        use enigo::{Axis, Button, Direction, Keyboard, Mouse};
        let mut flow_return = None;
        let result = match event {
            InputEvent::Key { key: code, down } => {
                if down {
                    let key = receiver
                        .codes
                        .get(&code)
                        .copied()
                        .or_else(|| key(&code))
                        .ok_or("Unsupported key.")?;
                    receiver.enigo.key(key, Direction::Press).map(|()| {
                        receiver.codes.insert(code, key);
                    })
                } else if let Some(&key) = receiver.codes.get(&code) {
                    let still_held = receiver
                        .codes
                        .iter()
                        .any(|(other, value)| other != &code && *value == key);
                    let release = if still_held {
                        Ok(())
                    } else {
                        receiver.enigo.key(key, Direction::Release)
                    };
                    release.map(|()| {
                        receiver.codes.remove(&code);
                    })
                } else {
                    Ok(())
                }
            }
            InputEvent::Move { x, y } => {
                let point = receiver.layout()?.primary_point(x, y);
                receiver.move_to(point)?;
                Ok(())
            }
            InputEvent::MoveBy { dx, dy } => {
                let position = receiver.position()?;
                receiver.layout()?;
                let layout = &receiver
                    .layout
                    .as_ref()
                    .ok_or("Display layout is unavailable.")?
                    .0;
                let (target, crossed) = match receiver.flow.as_mut() {
                    Some(flow) => flow.watch.step_layout(position, dx, dy, layout),
                    None => (
                        layout
                            .clamp((position.0.saturating_add(dx), position.1.saturating_add(dy))),
                        None,
                    ),
                };
                if let (Some(entry), Some(flow)) = (crossed, receiver.flow.as_mut()) {
                    flow.returned_at = Some(Instant::now());
                    flow_return = Some(FlowReturnInfo {
                        entry,
                        clipboard: flow.clipboard,
                    });
                }
                receiver.move_to(target)?;
                Ok(())
            }
            InputEvent::Button { button, down } => {
                let button = match button {
                    0 => Button::Left,
                    1 => Button::Middle,
                    _ => Button::Right,
                };
                if receiver.buttons.contains(&button) == down {
                    return Ok(None);
                }
                receiver.button(button, down).map(|()| {
                    if down {
                        receiver.buttons.insert(button);
                    } else {
                        receiver.buttons.remove(&button);
                    }
                })
            }
            InputEvent::Scroll { x, y } => {
                let horizontal = if x == 0 {
                    Ok(())
                } else {
                    receiver.enigo.scroll(x, Axis::Horizontal)
                };
                horizontal.and_then(|_| {
                    if y == 0 {
                        Ok(())
                    } else {
                        receiver.enigo.scroll(y, Axis::Vertical)
                    }
                })
            }
        };
        if flow_return.is_some() {
            receiver.release_all();
        }
        result
            .map(|_| flow_return)
            .map_err(|e| format!("Native input injection failed: {e}"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    Err("Native input is unavailable on this platform.".into())
}

pub async fn watchdog(core: Arc<Core>, changed: impl Fn(InputStatus)) {
    let mut timer = tokio::time::interval(Duration::from_millis(200));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut previous = InputStatus::default();
    loop {
        timer.tick().await;
        let peer = core
            .input
            .lock()
            .ok()
            .and_then(|s| s.receiving.as_ref().map(|r| r.peer.clone()));
        if let Some(peer) = peer {
            let allowed = enabled(&core, &peer).is_ok();
            if let Ok(mut state) = core.input.lock() {
                if state.receiving.as_ref().is_some_and(|r| {
                    r.peer == peer && (!allowed || r.last_seen.elapsed() > SESSION_TIMEOUT)
                }) {
                    state.finish_receiving();
                }
            }
        }
        let current = status(&core.input);
        if current != previous {
            previous = current.clone();
            changed(current);
        }
    }
}
#[derive(Clone, Default, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputStatus {
    sending_to: Option<String>,
    receiving_from: Option<String>,
    connecting_to: Option<String>,
    flow_session: Option<Uuid>,
}

pub fn status(state: &Mutex<InputState>) -> InputStatus {
    let Ok(state) = state.lock() else {
        return InputStatus::default();
    };
    let receiver = state
        .receiving
        .as_ref()
        .filter(|r| !r.flow.as_ref().is_some_and(|f| f.returned_at.is_some()));
    let connecting = state
        .flow
        .connecting()
        .or_else(|| state.pending_control.and(state.sending_to.as_deref()));
    InputStatus {
        sending_to: state
            .sending_to
            .clone()
            .filter(|peer| connecting != Some(peer)),
        receiving_from: receiver.map(|r| r.peer.clone()),
        connecting_to: connecting.map(str::to_owned),
        flow_session: state
            .flow
            .switch_id()
            .or_else(|| receiver.and_then(|r| r.flow.as_ref()?.session)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_clicks_preserve_count_on_release_and_expire() {
        let mut clicks = ClickSeries::default();
        let now = Instant::now();
        let interval = Duration::from_millis(400);
        let point = (-30, 90);
        assert_eq!(clicks.update(true, point, now, interval), 1);
        assert_eq!(
            clicks.update(false, point, now + Duration::from_millis(20), interval),
            1
        );
        assert_eq!(
            clicks.update(true, (-29, 90), now + Duration::from_millis(100), interval),
            2
        );
        assert_eq!(
            clicks.update(false, point, now + Duration::from_millis(120), interval),
            2
        );
        assert_eq!(
            clicks.update(true, point, now + Duration::from_millis(501), interval),
            1
        );
        assert_eq!(
            clicks.update(true, (100, 90), now + Duration::from_millis(502), interval),
            1
        );
        assert_eq!(
            clicks.update(
                true,
                point,
                now + Duration::from_millis(503),
                Duration::ZERO
            ),
            1
        );
    }
    #[test]
    fn input_boundaries() {
        assert!(validate(&InputEvent::Move {
            x: f64::NAN,
            y: 0.0
        })
        .is_err());
        assert!(validate(&InputEvent::Move { x: 1.1, y: 0.0 }).is_err());
        assert!(validate(&InputEvent::Button {
            button: 3,
            down: true
        })
        .is_err());
        assert!(validate(&InputEvent::Scroll { x: 0, y: 1000 }).is_err());
        assert!(validate(&InputEvent::Key {
            key: "KeyA".into(),
            down: true
        })
        .is_ok());
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    #[test]
    fn every_physical_key_resolves() {
        for code in crate::keymap::codes() {
            if cfg!(target_os = "macos") && code == "ContextMenu" {
                assert!(physical_key(code).is_none());
                continue;
            }
            assert!(key(code).is_some(), "{code}");
            assert!(known_key(code), "{code}");
        }
        assert!(!known_key("LaunchMail"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_receiver_swaps_command_and_control_back() {
        use enigo::Key;
        assert_eq!(physical_key("ControlLeft"), Some(Key::Other(0x37)));
        assert_eq!(physical_key("MetaLeft"), Some(Key::Other(0x3B)));
        assert_eq!(physical_key("ArrowLeft"), Some(Key::Other(0x7B)));
        assert_eq!(physical_key("KeyA"), Some(Key::Other(0x00)));
    }
}
