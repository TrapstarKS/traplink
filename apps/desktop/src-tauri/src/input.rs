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

#[derive(Default)]
pub struct InputState {
    pub sending_to: Option<String>,
    receiving: Option<Receiver>,
    pub flow: FlowState,
}
impl InputState {
    pub fn busy(&self) -> bool {
        self.sending_to.is_some() || self.receiving.is_some()
    }
}

struct ReceiverFlow {
    watch: ReturnWatch,
    clipboard: bool,
    returned: bool,
}
pub struct FlowReturnInfo {
    pub entry: f64,
    pub clipboard: bool,
}

struct Receiver {
    peer: String,
    last_seen: Instant,
    flow: Option<ReceiverFlow>,
    layout: Option<(Layout, Instant)>,
    #[cfg(target_os = "macos")]
    cursor: Option<((i32, i32), Instant)>,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    enigo: enigo::Enigo,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    codes: HashMap<String, enigo::Key>,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    buttons: HashSet<enigo::Button>,
}
impl Receiver {
    fn release_all(&mut self) {
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        {
            use enigo::{Direction, Keyboard, Mouse};
            let keys: HashSet<enigo::Key> = self.codes.drain().map(|(_, key)| key).collect();
            for key in keys {
                let _ = self.enigo.key(key, Direction::Release);
            }
            for button in self.buttons.drain() {
                let _ = self.enigo.button(button, Direction::Release);
            }
        }
    }
}
impl Drop for Receiver {
    fn drop(&mut self) {
        self.release_all();
    }
}
pub fn enabled(core: &Core, peer: &str) -> Result<(), String> {
    let data = core.data.lock().map_err(|_| "Settings lock unavailable.")?;
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
pub fn start_receiving(core: &Core, peer: &str) -> Result<(), String> {
    start_session(core, peer, None)
}
pub fn start_flow_receiving(
    core: &Core,
    peer: &str,
    sender_edge: FlowEdge,
    entry: f64,
    clipboard: bool,
) -> Result<(), String> {
    crate::flow::validate_entry(entry)?;
    start_session(
        core,
        peer,
        Some(ReceiverFlow {
            watch: ReturnWatch::new(sender_edge),
            clipboard,
            returned: false,
        }),
    )?;
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        let mut state = core.input.lock().map_err(|_| "Input lock unavailable.")?;
        if let Some(receiver) = state.receiving.as_mut().filter(|r| r.peer == peer) {
            let point = receiver.layout()?.entry_point(
                sender_edge.opposite(),
                entry,
                crate::flow::EDGE_INSET_PX,
            );
            if let Err(error) = receiver.move_to(point) {
                state.receiving = None;
                return Err(error);
            }
        }
    }
    Ok(())
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
        #[cfg(target_os = "windows")]
        return crate::screen::win::cursor()
            .ok_or_else(|| "Pointer position is unavailable.".into());
        #[cfg(target_os = "macos")]
        {
            if let Some((point, at)) = self.cursor {
                if at.elapsed() < INJECTED_CURSOR_TTL {
                    return Ok(point);
                }
            }
            let (x, y) =
                crate::screen::mac::cursor().ok_or("Pointer position is unavailable.")?;
            Ok((x.floor() as i32, y.floor() as i32))
        }
    }
    fn move_to(&mut self, point: (i32, i32)) -> Result<(), String> {
        #[cfg(target_os = "windows")]
        {
            let layout = self.layout()?;
            crate::screen::win::move_absolute(point, layout)
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
}
#[cfg(target_os = "macos")]
const INJECTED_CURSOR_TTL: Duration = Duration::from_millis(250);
#[cfg(target_os = "windows")]
fn platform_layout(_: &enigo::Enigo) -> Result<Layout, String> {
    crate::screen::win::layout().ok_or_else(|| "Display layout is unavailable.".to_string())
}
#[cfg(target_os = "macos")]
fn platform_layout(_: &enigo::Enigo) -> Result<Layout, String> {
    crate::screen::mac::layout().ok_or_else(|| "Display layout is unavailable.".to_string())
}
#[cfg_attr(
    not(any(target_os = "windows", target_os = "macos")),
    allow(unused_variables, unused_mut)
)]
fn start_session(core: &Core, peer: &str, flow: Option<ReceiverFlow>) -> Result<(), String> {
    enabled(core, peer)?;
    let mut state = core.input.lock().map_err(|_| "Input lock unavailable.")?;
    if state.sending_to.is_some() {
        return Err(
            "This computer is already controlling another peer; stop it before receiving input."
                .into(),
        );
    }
    if state.receiving.is_some() {
        return Err("An input session is already active on this computer.".into());
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        let settings = enigo::Settings {
            open_prompt_to_get_permissions: false,
            release_keys_when_dropped: true,
            ..Default::default()
        };
        let enigo = enigo::Enigo::new(&settings).map_err(|_| "Input access is unavailable. On macOS, allow TrapLink in System Settings > Privacy & Security > Accessibility, then retry.".to_string())?;
        state.receiving = Some(Receiver {
            peer: peer.into(),
            last_seen: Instant::now(),
            flow,
            layout: None,
            #[cfg(target_os = "macos")]
            cursor: None,
            enigo,
            codes: HashMap::new(),
            buttons: HashSet::new(),
        });
        Ok(())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    Err("Input receiving is supported on Windows and macOS only.".into())
}
pub fn active_with(core: &Core, peer: &str) -> bool {
    core.input.lock().is_ok_and(|state| {
        state.sending_to.as_deref() == Some(peer)
            || state.receiving.as_ref().is_some_and(|r| r.peer == peer)
            || state.flow.switch.peer() == Some(peer)
    })
}
pub fn stop_peer(core: &Core, peer: &str) {
    if let Ok(mut state) = core.input.lock() {
        if state.receiving.as_ref().is_some_and(|r| r.peer == peer) {
            state.receiving = None;
        }
        if state.sending_to.as_deref() == Some(peer) {
            state.sending_to = None;
        }
        crate::flow::session_ended(&mut state.flow, peer);
    }
}
pub fn stop_all(core: &Core) {
    if let Ok(mut state) = core.input.lock() {
        state.receiving = None;
        state.sending_to = None;
        crate::flow::disarm_all(&mut state.flow);
    }
}

pub fn keepalive(core: &Core, peer: &str) -> Result<(), String> {
    enabled(core, peer)?;
    let mut state = core.input.lock().map_err(|_| "Input lock unavailable.")?;
    let receiver = state
        .receiving
        .as_mut()
        .filter(|r| r.peer == peer)
        .ok_or("Input control is not active.")?;
    receiver.last_seen = Instant::now();
    Ok(())
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
        .filter(|r| r.peer == peer)
        .ok_or("Input control is not active.")?;
    if receiver.last_seen.elapsed() > Duration::from_secs(3) {
        state.receiving = None;
        return Err("Input control timed out; start it again.".into());
    }
    receiver.last_seen = Instant::now();
    if unknown_key || receiver.flow.as_ref().is_some_and(|f| f.returned) {
        return Ok(None);
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        use enigo::{Axis, Button, Coordinate, Direction, Keyboard, Mouse};
        let mut flow_return = None;
        let result = match event {
            InputEvent::Key { key: code, down } => {
                let key = if down {
                    let key = key(&code).ok_or("Unsupported key.")?;
                    receiver.codes.insert(code, key);
                    key
                } else {
                    let Some(key) = receiver.codes.remove(&code) else {
                        return Ok(None);
                    };
                    if receiver.codes.values().any(|other| *other == key) {
                        return Ok(None);
                    }
                    key
                };
                receiver.enigo.key(
                    key,
                    if down {
                        Direction::Press
                    } else {
                        Direction::Release
                    },
                )
            }
            InputEvent::Move { x, y } => {
                let (w, h) = receiver.enigo.main_display().map_err(|e| e.to_string())?;
                receiver.enigo.move_mouse(
                    (x * f64::from(w - 1)) as i32,
                    (y * f64::from(h - 1)) as i32,
                    Coordinate::Abs,
                )
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
                    flow.returned = true;
                    flow_return = Some(FlowReturnInfo {
                        entry,
                        clipboard: flow.clipboard,
                    });
                }
                if let Err(error) = receiver.move_to(target) {
                    return Err(error);
                }
                Ok(())
            }
            InputEvent::Button { button, down } => {
                let button = match button {
                    0 => Button::Left,
                    1 => Button::Middle,
                    _ => Button::Right,
                };
                if down {
                    receiver.buttons.insert(button);
                } else {
                    receiver.buttons.remove(&button);
                }
                receiver.enigo.button(
                    button,
                    if down {
                        Direction::Press
                    } else {
                        Direction::Release
                    },
                )
            }
            InputEvent::Scroll { x, y } => receiver
                .enigo
                .scroll(x, Axis::Horizontal)
                .and_then(|_| receiver.enigo.scroll(y, Axis::Vertical)),
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

pub async fn watchdog(core: Arc<Core>) {
    let mut timer = tokio::time::interval(Duration::from_millis(200));
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
                if state
                    .receiving
                    .as_ref()
                    .is_some_and(|r| !allowed || r.last_seen.elapsed() > Duration::from_secs(3))
                {
                    state.receiving = None;
                }
            }
        }
    }
}
pub fn status(state: &Mutex<InputState>) -> serde_json::Value {
    if let Ok(state) = state.lock() {
        serde_json::json!({"sendingTo":state.sending_to,"receivingFrom":state.receiving.as_ref().map(|r|r.peer.clone())})
    } else {
        serde_json::json!({"sendingTo":null,"receivingFrom":null})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
