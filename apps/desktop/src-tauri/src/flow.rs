use crate::{
    link,
    model::{FlowEdge, FlowMode, InputEvent, SavedFlow},
    network::Wire,
    screen::Layout,
    state::Core,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

mod pressed;
use pressed::{ButtonTracker, KeyTracker, Route, MODIFIER_IDS};

pub const MAX_DELAY_MS: u64 = 1000;
pub const REARM_PX: i32 = 24;
pub const RETURN_PUSH_PX: i32 = 6;
pub const EDGE_INSET_PX: i32 = 2;
pub const MAX_DELTA: i32 = 4096;
pub const UNSUPPORTED: &str = "Flow is not supported on this platform yet.";

pub fn supported() -> bool {
    cfg!(any(target_os = "windows", target_os = "macos"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
impl Bounds {
    pub fn new(left: i32, top: i32, width: i32, height: i32) -> Option<Self> {
        (width > 0 && height > 0).then(|| Self {
            left,
            top,
            right: left + width - 1,
            bottom: top + height - 1,
        })
    }
    pub(crate) fn distance(&self, edge: FlowEdge, (x, y): (i32, i32)) -> i32 {
        match edge {
            FlowEdge::Left => x - self.left,
            FlowEdge::Right => self.right - x,
            FlowEdge::Top => y - self.top,
            FlowEdge::Bottom => self.bottom - y,
        }
    }
    pub fn entry(&self, edge: FlowEdge, (x, y): (i32, i32)) -> f64 {
        let (value, start, end) = match edge {
            FlowEdge::Left | FlowEdge::Right => (y, self.top, self.bottom),
            FlowEdge::Top | FlowEdge::Bottom => (x, self.left, self.right),
        };
        let span = (end - start).max(1);
        (f64::from(value - start) / f64::from(span)).clamp(0.0, 1.0)
    }
    pub fn point_on(&self, edge: FlowEdge, entry: f64, inset: i32) -> (i32, i32) {
        let entry = if entry.is_finite() {
            entry.clamp(0.0, 1.0)
        } else {
            0.5
        };
        let along = |start: i32, end: i32| start + (entry * f64::from(end - start)).round() as i32;
        let point = match edge {
            FlowEdge::Left => (self.left + inset, along(self.top, self.bottom)),
            FlowEdge::Right => (self.right - inset, along(self.top, self.bottom)),
            FlowEdge::Top => (along(self.left, self.right), self.top + inset),
            FlowEdge::Bottom => (along(self.left, self.right), self.bottom - inset),
        };
        self.clamp(point)
    }
    pub fn clamp(&self, (x, y): (i32, i32)) -> (i32, i32) {
        (
            x.clamp(self.left, self.right),
            y.clamp(self.top, self.bottom),
        )
    }
    pub fn center(&self) -> (i32, i32) {
        (
            self.left + (self.right - self.left) / 2,
            self.top + (self.bottom - self.top) / 2,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlowConfig {
    pub edge: FlowEdge,
    pub mode: FlowMode,
    pub delay: Duration,
    pub block_while_button: bool,
    pub clipboard: bool,
}
impl FlowConfig {
    pub fn new(
        edge: FlowEdge,
        mode: FlowMode,
        delay_ms: u64,
        block_while_button: bool,
        clipboard: bool,
    ) -> Result<Self, String> {
        if delay_ms > MAX_DELAY_MS {
            return Err(format!(
                "The accidental-switch protection must be between 0 and {MAX_DELAY_MS} ms."
            ));
        }
        Ok(Self {
            edge,
            mode,
            delay: Duration::from_millis(delay_ms),
            block_while_button,
            clipboard,
        })
    }
}

#[derive(Clone, Debug)]
pub struct EdgeTracker {
    pub config: FlowConfig,
    since: Option<Instant>,
    pushes: u32,
    last: Option<(i32, i32)>,
    inside: Option<(i32, i32)>,
    armed: bool,
}
impl EdgeTracker {
    pub fn new(config: FlowConfig) -> Self {
        Self {
            config,
            since: None,
            pushes: 0,
            last: None,
            inside: None,
            armed: true,
        }
    }
    pub fn disarm_until_leave(&mut self) {
        self.armed = false;
        self.since = None;
        self.pushes = 0;
    }
    #[cfg(test)]
    pub fn update(
        &mut self,
        position: (i32, i32),
        bounds: Bounds,
        buttons_held: bool,
        ctrl_held: bool,
        now: Instant,
    ) -> Option<f64> {
        self.update_layout(
            position,
            None,
            &Layout::single(bounds),
            buttons_held,
            ctrl_held,
            now,
        )
    }
    pub fn update_layout(
        &mut self,
        position: (i32, i32),
        cursor: Option<(i32, i32)>,
        layout: &Layout,
        buttons_held: bool,
        ctrl_held: bool,
        now: Instant,
    ) -> Option<f64> {
        let edge = self.config.edge;
        let previous = self.last.replace(position);
        let from = if layout.clamp(position) == position {
            self.inside = Some(position);
            position
        } else if let Some(cursor) = cursor {
            let cursor = layout.clamp(cursor);
            self.inside = Some(cursor);
            cursor
        } else {
            self.inside.unwrap_or(position)
        };
        if !self.armed {
            if layout.distance_from_edge(edge, position) >= REARM_PX {
                self.armed = true;
            }
            return None;
        }
        let blocked = (self.config.block_while_button && buttons_held)
            || (self.config.mode == FlowMode::CtrlEdge && !ctrl_held);
        let overshoot = layout.outer_edge_overshoot_from(edge, position, from);
        let Some(overshoot) = overshoot.filter(|_| !blocked) else {
            self.since = None;
            self.pushes = 0;
            return None;
        };
        let along = |(x, y): (i32, i32)| match edge {
            FlowEdge::Left | FlowEdge::Right => y,
            FlowEdge::Top | FlowEdge::Bottom => x,
        };
        let cornered = edge
            .perpendicular()
            .into_iter()
            .any(|side| layout.outer_edge_overshoot_from(side, position, from).is_some());
        let push = overshoot > 0
            || (!cornered
                && (self.since.is_none() || previous.is_none_or(|p| along(p) == along(position))));
        if !push && self.since.is_none() {
            return None;
        }
        let start = *self.since.get_or_insert(now);
        if push {
            self.pushes = self.pushes.saturating_add(1);
        }
        let delay = match self.config.mode {
            FlowMode::Edge => self.config.delay,
            FlowMode::CtrlEdge => Duration::ZERO,
        };
        let needed = if delay.is_zero() { 1 } else { 2 };
        if push && now.saturating_duration_since(start) >= delay && self.pushes >= needed {
            self.disarm_until_leave();
            Some(layout.entry(edge, position))
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
pub struct ReturnWatch {
    pub edge: FlowEdge,
    push: i32,
}
impl ReturnWatch {
    pub fn new(sender_edge: FlowEdge) -> Self {
        Self {
            edge: sender_edge.opposite(),
            push: 0,
        }
    }
    #[cfg(test)]
    pub fn step(
        &mut self,
        position: (i32, i32),
        dx: i32,
        dy: i32,
        bounds: Bounds,
    ) -> ((i32, i32), Option<f64>) {
        self.step_layout(position, dx, dy, &Layout::single(bounds))
    }
    pub fn step_layout(
        &mut self,
        position: (i32, i32),
        dx: i32,
        dy: i32,
        layout: &Layout,
    ) -> ((i32, i32), Option<f64>) {
        let target = (position.0.saturating_add(dx), position.1.saturating_add(dy));
        let clamped = layout.clamp_from(target, position);
        match layout.outer_edge_overshoot_from(self.edge, target, position) {
            Some(overshoot) if overshoot > 0 => {
                self.push = self.push.saturating_add(overshoot);
                if self.push >= RETURN_PUSH_PX {
                    self.push = 0;
                    return (clamped, Some(layout.entry(self.edge, clamped)));
                }
            }
            Some(_) => {}
            None => self.push = 0,
        }
        (clamped, None)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Switch {
    #[default]
    Local,
    Starting {
        peer: String,
        deadline: Instant,
    },
    Forwarding {
        peer: String,
    },
}
#[derive(Clone, Debug)]
pub enum SwitchEvent {
    Begin { peer: String, now: Instant },
    Started { peer: String, ok: bool },
    Returned { peer: String },
    Lost { peer: String },
    Emergency,
    HookLost,
    Tick { now: Instant },
}
pub const START_DEADLINE: Duration = Duration::from_millis(3500);
impl Switch {
    pub fn peer(&self) -> Option<&str> {
        match self {
            Self::Local => None,
            Self::Starting { peer, .. } | Self::Forwarding { peer } => Some(peer),
        }
    }
    pub fn forwarding(&self) -> Option<&str> {
        match self {
            Self::Forwarding { peer } => Some(peer),
            _ => None,
        }
    }
    pub fn starting(&self) -> Option<&str> {
        match self {
            Self::Starting { peer, .. } => Some(peer),
            _ => None,
        }
    }
    pub fn apply(&mut self, event: SwitchEvent) -> bool {
        let next = match (&*self, event) {
            (Self::Local, SwitchEvent::Begin { peer, now }) => Self::Starting {
                peer,
                deadline: now + START_DEADLINE,
            },
            (Self::Starting { peer, .. }, SwitchEvent::Started { peer: p, ok }) if *peer == p => {
                if ok {
                    Self::Forwarding { peer: p }
                } else {
                    Self::Local
                }
            }
            (
                Self::Starting { peer, .. } | Self::Forwarding { peer },
                SwitchEvent::Lost { peer: p },
            ) if *peer == p => Self::Local,
            (Self::Forwarding { peer }, SwitchEvent::Returned { peer: p }) if *peer == p => {
                Self::Local
            }
            (Self::Starting { .. } | Self::Forwarding { .. }, SwitchEvent::Emergency)
            | (Self::Starting { .. } | Self::Forwarding { .. }, SwitchEvent::HookLost) => {
                Self::Local
            }
            (Self::Starting { deadline, .. }, SwitchEvent::Tick { now }) if now >= *deadline => {
                Self::Local
            }
            _ => return false,
        };
        *self = next;
        true
    }
}

#[derive(Clone, Debug, Default)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct HookHealth {
    suspicious: u8,
    last_reinstall: Option<Instant>,
}
#[cfg_attr(not(windows), allow(dead_code))]
pub const REINSTALL_COOLDOWN: Duration = Duration::from_secs(5);
#[cfg_attr(not(windows), allow(dead_code))]
impl HookHealth {
    pub fn observe(&mut self, cursor_moved: bool, events: u32, now: Instant) -> bool {
        if !cursor_moved || events > 0 {
            self.suspicious = 0;
            return false;
        }
        self.suspicious = self.suspicious.saturating_add(1);
        let cooled = self
            .last_reinstall
            .is_none_or(|at| now.saturating_duration_since(at) >= REINSTALL_COOLDOWN);
        if self.suspicious >= 2 && cooled {
            self.suspicious = 0;
            self.last_reinstall = Some(now);
            return true;
        }
        false
    }
}

pub fn validate_entry(entry: f64) -> Result<(), String> {
    if entry.is_finite() && (0.0..=1.0).contains(&entry) {
        Ok(())
    } else {
        Err("Invalid edge position.".into())
    }
}

pub fn scan_code(scan: u32, extended: bool) -> Option<&'static str> {
    crate::keymap::code_for_scan(scan, extended)
}
pub fn key_id(scan: u32, extended: bool) -> u32 {
    scan | (u32::from(extended) << 8)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RawEvent {
    Move {
        x: i32,
        y: i32,
        from: Option<(i32, i32)>,
    },
    Button {
        index: u8,
        down: bool,
    },
    Wheel {
        delta: i32,
        horizontal: bool,
    },
    Key {
        scan: u32,
        extended: bool,
        down: bool,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Swallow,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Captured {
    EdgeHit {
        peer: String,
        entry: f64,
    },
    Event(InputEvent),
    Move {
        dx: i32,
        dy: i32,
    },
    Emergency,
    Returned {
        peer: String,
        entry: f64,
    },
    HookReinstalled {
        removed: bool,
    },
    HookFailed,
    Link {
        peer: String,
        result: Result<(), String>,
    },
    Shutdown,
}

pub struct Arm {
    pub peer: String,
    pub tracker: EdgeTracker,
}

#[derive(Default)]
pub struct CaptureLogic {
    pub arms: Vec<Arm>,
    anchor: (i32, i32),
    buttons: ButtonTracker,
    keys: KeyTracker,
    wheel: (i32, i32),
    pub swap_command_control: bool,
}
impl CaptureLogic {
    fn ctrl(&self) -> bool {
        self.keys.held(0x1D) || self.keys.held(0x11D)
    }
    fn alt(&self) -> bool {
        self.keys.held(0x38) || self.keys.held(0x138)
    }
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn key_held(&self, scan: u32, extended: bool) -> bool {
        self.keys.held(key_id(scan, extended))
    }
    #[cfg_attr(not(any(target_os = "windows", target_os = "macos")), allow(dead_code))]
    pub fn adopt_local_keys(&mut self, ids: impl IntoIterator<Item = u32>) {
        for id in ids {
            self.keys.adopt_local(id);
        }
    }
    fn key_event(&self, id: u32, down: bool) -> Option<InputEvent> {
        if id >= 0x200 {
            return None;
        }
        let code = scan_code(id & 0xFF, id & 0x100 != 0)?;
        let code = if self.swap_command_control {
            crate::keymap::swap_command_control(code)
        } else {
            code
        };
        Some(InputEvent::Key {
            key: code.into(),
            down,
        })
    }
    pub fn set_arm(&mut self, peer: &str, config: FlowConfig) {
        self.arms.retain(|a| a.peer != peer);
        self.arms.push(Arm {
            peer: peer.into(),
            tracker: EdgeTracker::new(config),
        });
    }
    pub fn remove_arm(&mut self, peer: &str) {
        self.arms.retain(|a| a.peer != peer);
    }
    pub fn disarm_until_leave(&mut self, peer: &str) {
        if let Some(arm) = self.arms.iter_mut().find(|a| a.peer == peer) {
            arm.tracker.disarm_until_leave();
        }
    }
    pub fn start_forwarding(&mut self, anchor: (i32, i32)) {
        self.anchor = anchor;
        self.keys.begin();
        self.buttons.begin();
        self.wheel = (0, 0);
    }
    pub fn end_forwarding(&mut self, peer: &str) -> Vec<InputEvent> {
        let buttons = self.buttons.finish();
        let keys = self.keys.finish();
        let mut releases: Vec<InputEvent> = buttons
            .iter()
            .filter(|button| *button <= 2)
            .map(|button| InputEvent::Button {
                button,
                down: false,
            })
            .collect();
        let (modifiers, others): (Vec<u32>, Vec<u32>) =
            keys.iter().partition(|id| MODIFIER_IDS.contains(id));
        releases.extend(
            others
                .into_iter()
                .chain(modifiers)
                .filter_map(|id| self.key_event(id, false)),
        );
        self.wheel = (0, 0);
        self.disarm_until_leave(peer);
        releases
    }

    pub fn handle(
        &mut self,
        event: RawEvent,
        injected: bool,
        forwarding: bool,
        layout: &Layout,
        now: Instant,
    ) -> (Verdict, Option<Captured>) {
        if injected {
            let hold = forwarding && matches!(event, RawEvent::Move { .. });
            return (
                if hold {
                    Verdict::Swallow
                } else {
                    Verdict::Pass
                },
                None,
            );
        }
        match event {
            RawEvent::Key {
                scan,
                extended,
                down,
            } => {
                let id = key_id(scan, extended);
                if down && scan == 0x01 && !extended && self.ctrl() && self.alt() {
                    self.keys.route(id, true, false);
                    return (Verdict::Pass, Some(Captured::Emergency));
                }
                match self.keys.route(id, down, forwarding) {
                    Route::Local => (Verdict::Pass, None),
                    Route::Drop => (Verdict::Swallow, None),
                    Route::Forward => (
                        Verdict::Swallow,
                        self.key_event(id, down).map(Captured::Event),
                    ),
                }
            }
            RawEvent::Button { index, down } => {
                match self.buttons.route(u32::from(index), down, forwarding) {
                    Route::Local => (Verdict::Pass, None),
                    Route::Drop => (Verdict::Swallow, None),
                    Route::Forward => (
                        Verdict::Swallow,
                        (index <= 2).then_some(Captured::Event(InputEvent::Button {
                            button: index,
                            down,
                        })),
                    ),
                }
            }
            RawEvent::Wheel { delta, horizontal } => {
                if !forwarding {
                    return (Verdict::Pass, None);
                }
                let acc = if horizontal {
                    &mut self.wheel.0
                } else {
                    &mut self.wheel.1
                };
                *acc = acc.saturating_add(delta);
                let notches = *acc / 120;
                *acc -= notches * 120;
                let notches = notches.clamp(-10, 10);
                if notches == 0 {
                    return (Verdict::Swallow, None);
                }
                let event = if horizontal {
                    InputEvent::Scroll { x: notches, y: 0 }
                } else {
                    InputEvent::Scroll { x: 0, y: -notches }
                };
                (Verdict::Swallow, Some(Captured::Event(event)))
            }
            RawEvent::Move { x, y, from } => {
                if forwarding {
                    let (dx, dy) = (x - self.anchor.0, y - self.anchor.1);
                    let event = (dx != 0 || dy != 0).then_some(Captured::Move {
                        dx: dx.clamp(-MAX_DELTA, MAX_DELTA),
                        dy: dy.clamp(-MAX_DELTA, MAX_DELTA),
                    });
                    return (Verdict::Swallow, event);
                }
                let (buttons, ctrl) = (self.buttons.any_held(), self.ctrl());
                for arm in &mut self.arms {
                    if let Some(entry) =
                        arm.tracker
                            .update_layout((x, y), from, layout, buttons, ctrl, now)
                    {
                        return (
                            Verdict::Pass,
                            Some(Captured::EdgeHit {
                                peer: arm.peer.clone(),
                                entry,
                            }),
                        );
                    }
                }
                (Verdict::Pass, None)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowError {
    pub code: &'static str,
    pub peer: Option<String>,
    pub detail: String,
}
impl FlowError {
    pub fn new(code: &'static str, peer: &str, detail: impl Into<String>) -> Self {
        Self {
            code,
            peer: Some(peer.into()),
            detail: detail.into(),
        }
    }
}
const CONNECTION_CODES: [&str; 3] = ["connection_lost", "reconnect_failed", "remote_closed"];

pub fn classify_start_error(message: &str) -> &'static str {
    let lower = message.to_lowercase();
    if lower.contains("accessibility") || lower.contains("input access is unavailable") {
        "peer_accessibility"
    } else if lower.contains("input requires") || lower.contains("permission") {
        "peer_denied"
    } else if lower.contains("already") {
        "peer_busy"
    } else if lower.contains("not connected") || lower.contains("connection") {
        "connection_lost"
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "timeout"
    } else {
        "start_failed"
    }
}

#[derive(Default)]
pub struct FlowState {
    pub peers: HashMap<String, FlowConfig>,
    pub switch: Switch,
    pub last_error: Option<FlowError>,
    links: HashMap<String, link::Reconnect>,
    suspended: HashSet<String>,
    remote_closed: HashSet<String>,
    controller: Option<Controller>,
}
impl FlowState {
    pub fn connecting(&self) -> Option<&str> {
        self.switch.starting()
    }
    pub fn forwarding(&self) -> Option<&str> {
        self.switch.forwarding()
    }
}
struct Controller {
    tx: UnboundedSender<Captured>,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    hook: platform::HookThread,
}

fn shutdown(flow: &mut FlowState) {
    platform::set_forwarding(false);
    if let Some(controller) = flow.controller.take() {
        let _ = controller.tx.send(Captured::Shutdown);
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        controller.hook.stop();
    }
    flow.peers.clear();
    flow.switch = Switch::Local;
    flow.links.clear();
    flow.suspended.clear();
    flow.remote_closed.clear();
}
pub fn disarm_all(flow: &mut FlowState) {
    shutdown(flow);
}
fn disarm_peer(flow: &mut FlowState, peer: &str) {
    flow.peers.remove(peer);
    flow.links.remove(peer);
    flow.suspended.remove(peer);
    flow.remote_closed.remove(peer);
    platform::with_logic(|logic| logic.remove_arm(peer));
    if flow.switch.peer() == Some(peer) {
        local_end(flow, peer);
    }
    if flow.peers.is_empty() {
        shutdown(flow);
    }
}
pub fn session_ended(flow: &mut FlowState, peer: &str) {
    if flow.switch.peer() == Some(peer) {
        local_end(flow, peer);
    }
}
fn local_end(flow: &mut FlowState, peer: &str) -> Vec<InputEvent> {
    platform::set_forwarding(false);
    let releases = platform::with_logic(|logic| logic.end_forwarding(peer)).unwrap_or_default();
    flow.switch.apply(SwitchEvent::Lost { peer: peer.into() });
    releases
}
fn release_remote(core: &Core, peer: &str, releases: Vec<InputEvent>) {
    for event in releases {
        crate::network::try_send(core, peer, Wire::Input { event });
    }
}

pub fn remote_closed(core: &Core, peer: &str) {
    if let Ok(mut state) = core.input.lock() {
        if state.flow.peers.contains_key(peer) {
            state.flow.remote_closed.insert(peer.into());
            state.flow.last_error = Some(FlowError::new(
                "remote_closed",
                peer,
                "The other computer closed the connection.",
            ));
        }
    }
}
pub fn user_disconnected(core: &Core, peer: &str) {
    if let Ok(mut guard) = core.input.lock() {
        let state = &mut *guard;
        if state.flow.peers.contains_key(peer) {
            if state.flow.switch.peer() == Some(peer) && state.sending_to.as_deref() == Some(peer) {
                state.sending_to = None;
            }
            disarm_peer(&mut state.flow, peer);
        }
    }
    if let Err(error) = forget_saved(core, peer) {
        core.warning(error);
    }
}

pub fn returned(core: &Core, peer: &str, entry: f64) -> Result<(), String> {
    validate_entry(entry)?;
    let state = core.input.lock().map_err(|_| "Input lock unavailable.")?;
    if state.flow.forwarding() != Some(peer) {
        return Ok(());
    }
    if let Some(controller) = &state.flow.controller {
        let _ = controller.tx.send(Captured::Returned {
            peer: peer.into(),
            entry,
        });
    }
    Ok(())
}

fn ticker(period: Duration) -> tokio::time::Interval {
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval
}

async fn controller(core: Arc<Core>, mut rx: UnboundedReceiver<Captured>) {
    let mut flush = ticker(Duration::from_millis(4));
    let mut keepalive = ticker(Duration::from_secs(1));
    let mut health = ticker(Duration::from_millis(250));
    let mut supervisor = Supervisor::new(&core);
    loop {
        tokio::select! {
            message = rx.recv() => match message {
                None | Some(Captured::Shutdown) => break,
                Some(Captured::EdgeHit { peer, entry }) => {
                    platform::take_moves();
                    tokio::spawn(begin(core.clone(), peer, entry));
                }
                Some(Captured::Move { .. }) => {}
                Some(Captured::Event(event)) => {
                    if let Some(peer) = forwarding(&core) {
                        flush_moves(&core, &peer).await;
                        if let Err(error) = send(&core, &peer, Wire::Input { event }).await {
                            end(&core, &peer, None, Some(FlowError::new("connection_lost", &peer, error))).await;
                        }
                    }
                }
                Some(Captured::Emergency) => {
                    platform::set_forwarding(false);
                    if let Ok(mut state) = core.input.lock() {
                        state.flow.switch.apply(SwitchEvent::Emergency);
                    }
                    crate::emergency_stop_core(&core);
                    break;
                }
                Some(Captured::Returned { peer, entry }) => {
                    if forwarding(&core).as_deref() == Some(peer.as_str()) {
                        platform::take_moves();
                        end(&core, &peer, Some(entry), None).await;
                    }
                }
                Some(Captured::HookReinstalled { removed }) => {
                    if removed {
                        if let Some(peer) = forwarding(&core) {
                            end(&core, &peer, None, Some(FlowError::new(
                                "hook_removed",
                                &peer,
                                platform::REMOVED_DETAIL,
                            ))).await;
                        }
                    }
                }
                Some(Captured::HookFailed) => {
                    let detail = "The low-level input hooks could not be reinstalled.";
                    if let Some(peer) = forwarding(&core) {
                        end(&core, &peer, None, Some(FlowError::new("hook_failed", &peer, detail))).await;
                    }
                    if let Ok(mut state) = core.input.lock() {
                        state.flow.last_error = Some(FlowError {
                            code: "hook_failed",
                            peer: None,
                            detail: detail.into(),
                        });
                    }
                }
                Some(Captured::Link { peer, result }) => supervisor.link_result(&core, &peer, result),
            },
            _ = flush.tick() => {
                platform::heartbeat();
                match forwarding(&core) {
                    Some(peer) => flush_moves(&core, &peer).await,
                    None => {
                        platform::take_moves();
                    }
                }
            }
            _ = keepalive.tick() => {
                if let Some(peer) = forwarding(&core) {
                    if let Err(error) = send(&core, &peer, Wire::InputKeepalive).await {
                        end(&core, &peer, None, Some(FlowError::new("connection_lost", &peer, error))).await;
                    }
                }
            }
            _ = health.tick() => {
                if !supervisor.check(&core) { break; }
            }
        }
    }
    platform::set_forwarding(false);
}

fn forwarding(core: &Core) -> Option<String> {
    core.input
        .lock()
        .ok()
        .and_then(|s| s.flow.forwarding().map(str::to_owned))
}

async fn send(core: &Core, peer: &str, message: Wire) -> Result<(), String> {
    crate::network::send_message(core, peer, message).await
}

async fn flush_moves(core: &Core, peer: &str) {
    let mut pending = platform::take_moves();
    while pending != (0, 0) {
        let dx = pending.0.clamp(-MAX_DELTA, MAX_DELTA);
        let dy = pending.1.clamp(-MAX_DELTA, MAX_DELTA);
        pending.0 -= dx;
        pending.1 -= dy;
        let message = Wire::Input {
            event: InputEvent::MoveBy { dx, dy },
        };
        if let Err(error) = send(core, peer, message).await {
            platform::take_moves();
            end(
                core,
                peer,
                None,
                Some(FlowError::new("connection_lost", peer, error)),
            )
            .await;
            return;
        }
    }
}

struct PeerProbe {
    peer: String,
    connected: bool,
    permitted: Result<(), String>,
    address: Option<String>,
}

struct Supervisor {
    backoff: link::Backoff,
    last_tick: Instant,
    last_wall: SystemTime,
    addresses: Vec<String>,
    addresses_at: Instant,
}
impl Supervisor {
    fn new(core: &Core) -> Self {
        Self {
            backoff: link::Backoff::default(),
            last_tick: Instant::now(),
            last_wall: SystemTime::now(),
            addresses: core.addresses(),
            addresses_at: Instant::now(),
        }
    }

    fn check(&mut self, core: &Arc<Core>) -> bool {
        let now = Instant::now();
        let wall = SystemTime::now();
        let resumed = link::resumed(
            self.last_wall,
            wall,
            now.saturating_duration_since(self.last_tick),
            link::RESUME_GAP,
        );
        self.last_tick = now;
        self.last_wall = wall;
        let mut network_changed = false;
        if now.saturating_duration_since(self.addresses_at) >= Duration::from_secs(2) {
            let addresses = core.addresses();
            network_changed = addresses != self.addresses;
            self.addresses = addresses;
            self.addresses_at = now;
        }
        let peers: Vec<String> = match core.input.lock() {
            Ok(state) if state.flow.controller.is_some() => {
                state.flow.peers.keys().cloned().collect()
            }
            _ => return false,
        };
        let my_id = core
            .data
            .lock()
            .map(|d| d.device_id.clone())
            .unwrap_or_default();
        let probes: Vec<PeerProbe> = peers
            .into_iter()
            .map(|peer| PeerProbe {
                connected: crate::network::is_connected(core, &peer),
                permitted: crate::input::enabled(core, &peer),
                address: crate::network::peer_address(core, &peer),
                peer,
            })
            .collect();
        let mut attempts = Vec::new();
        let Ok(mut guard) = core.input.lock() else {
            return false;
        };
        let state = &mut *guard;
        let starting = state.flow.connecting().map(str::to_owned);
        if state.flow.switch.apply(SwitchEvent::Tick { now }) {
            if let Some(peer) = starting {
                if state.sending_to.as_deref() == Some(peer.as_str()) {
                    state.sending_to = None;
                }
                platform::with_logic(|logic| logic.disarm_until_leave(&peer));
                state.flow.last_error = Some(FlowError::new(
                    "timeout",
                    &peer,
                    "The start handshake did not finish.",
                ));
            }
        }
        if let Some(peer) = state.flow.forwarding().map(str::to_owned) {
            if state.sending_to.as_deref() != Some(peer.as_str()) {
                local_end(&mut state.flow, &peer);
            }
        }
        for probe in probes {
            let peer = probe.peer;
            if !state.flow.peers.contains_key(&peer) {
                continue;
            }
            if let Err(error) = probe.permitted {
                if state.sending_to.as_deref() == Some(peer.as_str()) {
                    state.sending_to = None;
                }
                disarm_peer(&mut state.flow, &peer);
                state.flow.last_error = Some(FlowError::new("permission_changed", &peer, error));
                continue;
            }
            let flow = &mut state.flow;
            if probe.connected {
                if flow.suspended.remove(&peer) {
                    if let Some(config) = flow.peers.get(&peer).copied() {
                        platform::with_logic(|logic| {
                            logic.set_arm(&peer, config);
                            logic.disarm_until_leave(&peer);
                        });
                    }
                }
                flow.links.entry(peer.clone()).or_default().connected();
                flow.remote_closed.remove(&peer);
                if flow.last_error.as_ref().is_some_and(|e| {
                    e.peer.as_deref() == Some(peer.as_str()) && CONNECTION_CODES.contains(&e.code)
                }) {
                    flow.last_error = None;
                }
                continue;
            }
            if flow.switch.peer() == Some(peer.as_str()) {
                if state.sending_to.as_deref() == Some(peer.as_str()) {
                    state.sending_to = None;
                }
                local_end(&mut state.flow, &peer);
                state.flow.last_error = Some(FlowError::new(
                    "connection_lost",
                    &peer,
                    "The connection dropped while Flow was active.",
                ));
            }
            let flow = &mut state.flow;
            if flow.suspended.insert(peer.clone()) {
                platform::with_logic(|logic| logic.remove_arm(&peer));
            }
            if flow.remote_closed.contains(&peer) {
                continue;
            }
            let Some(address) = probe.address else {
                continue;
            };
            let extra = if my_id.as_str() > peer.as_str() {
                link::TIE_BREAK
            } else {
                Duration::ZERO
            };
            let link = flow.links.entry(peer.clone()).or_default();
            if resumed || network_changed {
                link.kick(now, extra);
            } else {
                link.lost(&self.backoff, now, link::jitter(), extra);
            }
            if link.take_due(now) {
                attempts.push((peer, address));
            }
        }
        let tx = state.flow.controller.as_ref().map(|c| c.tx.clone());
        drop(guard);
        let Some(tx) = tx else {
            return false;
        };
        for (peer, address) in attempts {
            let (core, tx) = (core.clone(), tx.clone());
            tokio::spawn(async move {
                let result = reconnect(&core, &peer, address).await;
                let _ = tx.send(Captured::Link { peer, result });
            });
        }
        true
    }

    fn link_result(&self, core: &Core, peer: &str, result: Result<(), String>) {
        record_link(core, peer, result, &self.backoff);
    }
}

fn record_link(core: &Core, peer: &str, result: Result<(), String>, backoff: &link::Backoff) {
    let Ok(mut state) = core.input.lock() else {
        return;
    };
    let flow = &mut state.flow;
    let Some(link) = flow.links.get_mut(peer) else {
        return;
    };
    match result {
        Ok(()) => link.connected(),
        Err(error) => {
            link.failed(backoff, Instant::now(), link::jitter(), error.clone());
            if link.gave_up {
                flow.last_error = Some(FlowError::new("reconnect_failed", peer, error));
            }
        }
    }
}

async fn reconnect(core: &Arc<Core>, peer: &str, address: String) -> Result<(), String> {
    if crate::network::is_connected(core, peer) {
        return Ok(());
    }
    tokio::time::timeout(
        Duration::from_secs(15),
        crate::network::connect(core.clone(), address, None, Some(peer.into())),
    )
    .await
    .map_err(|_| "Connection attempt timed out.".to_string())?
    .map(|_| ())
}

async fn begin(core: Arc<Core>, peer: String, entry: f64) {
    let peer = peer.as_str();
    let config = {
        let Ok(state) = core.input.lock() else {
            return;
        };
        let Some(config) = state.flow.peers.get(peer).copied() else {
            return;
        };
        if state.busy() || state.flow.switch != Switch::Local {
            return;
        }
        config
    };
    if let Err(error) = crate::input::enabled(&core, peer) {
        set_error(
            &core,
            peer,
            FlowError::new("permission_changed", peer, error),
        );
        return;
    }
    {
        let Ok(mut state) = core.input.lock() else {
            return;
        };
        if state.busy()
            || !state.flow.peers.contains_key(peer)
            || !state.flow.switch.apply(SwitchEvent::Begin {
                peer: peer.into(),
                now: Instant::now(),
            })
        {
            return;
        }
        state.sending_to = Some(peer.into());
    }
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        crate::network::flow_start(&core, peer, config.edge, entry, config.clipboard),
    )
    .await
    .unwrap_or_else(|_| Err("timeout: the other computer did not confirm Flow.".into()))
    .map_err(|error| FlowError::new(classify_start_error(&error), peer, error));
    let started = {
        let Ok(mut guard) = core.input.lock() else {
            return;
        };
        let state = &mut *guard;
        let ok = result.is_ok()
            && state.sending_to.as_deref() == Some(peer)
            && state.flow.peers.contains_key(peer)
            && state.flow.controller.is_some()
            && state.flow.connecting() == Some(peer);
        state.flow.switch.apply(SwitchEvent::Started {
            peer: peer.into(),
            ok,
        });
        if ok {
            state.flow.last_error = None;
            platform::begin_forwarding();
        } else {
            if state.sending_to.as_deref() == Some(peer) && state.flow.switch.peer() != Some(peer) {
                state.sending_to = None;
            }
            platform::with_logic(|logic| logic.disarm_until_leave(peer));
            state.flow.last_error = Some(result.clone().err().unwrap_or_else(|| {
                FlowError::new(
                    "start_failed",
                    peer,
                    "Flow was interrupted during the start.",
                )
            }));
        }
        ok
    };
    if !started {
        if result.is_ok() {
            let _ = send(&core, peer, Wire::ControlStop).await;
        }
        return;
    }
    if config.clipboard && crate::network::clipboard_permitted(&core, peer) {
        let (core, peer) = (core.clone(), peer.to_string());
        tokio::spawn(async move {
            let _ = crate::network::send_clipboard(core, &peer).await;
        });
    }
}

async fn end(core: &Core, peer: &str, entry: Option<f64>, error: Option<FlowError>) {
    let (edge, releases) = {
        let Ok(mut state) = core.input.lock() else {
            platform::set_forwarding(false);
            return;
        };
        let state = &mut *state;
        let edge = state.flow.peers.get(peer).map(|c| c.edge);
        let reason = match (&entry, &error) {
            (Some(_), _) => SwitchEvent::Returned { peer: peer.into() },
            (None, Some(e)) if e.code.starts_with("hook_") => SwitchEvent::HookLost,
            _ => SwitchEvent::Lost { peer: peer.into() },
        };
        state.flow.switch.apply(reason);
        let releases = local_end(&mut state.flow, peer);
        if state.sending_to.as_deref() == Some(peer) {
            state.sending_to = None;
        }
        if error.is_some() {
            state.flow.last_error = error;
        }
        (edge, releases)
    };
    if let (Some(entry), Some(edge)) = (entry, edge) {
        platform::place_cursor(edge, entry);
    }
    for event in releases {
        if send(core, peer, Wire::Input { event }).await.is_err() {
            break;
        }
    }
    let _ = send(core, peer, Wire::ControlStop).await;
}

fn set_error(core: &Core, peer: &str, error: FlowError) {
    if let Ok(mut state) = core.input.lock() {
        state.flow.last_error = Some(error);
    }
    platform::with_logic(|logic| logic.disarm_until_leave(peer));
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowPeerStatus {
    pub edge: FlowEdge,
    pub mode: FlowMode,
    pub delay_ms: u64,
    pub block_while_button: bool,
    pub clipboard: bool,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowLinkStatus {
    pub state: &'static str,
    pub attempt: u32,
    pub max_attempts: u32,
    pub next_retry_ms: Option<u64>,
    pub rtt_ms: Option<u32>,
    pub last_error: Option<String>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    pub installed: bool,
    pub reinstalls: u32,
    pub last_reinstall_ago_ms: Option<u64>,
    pub reason: Option<&'static str>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowStatus {
    pub supported: bool,
    pub reason: Option<String>,
    pub peers: HashMap<String, FlowPeerStatus>,
    pub connecting: Option<String>,
    pub forwarding: Option<String>,
    pub last_error: Option<FlowError>,
    pub links: HashMap<String, FlowLinkStatus>,
    pub capture: Option<CaptureStatus>,
    pub saved: HashMap<String, FlowPeerStatus>,
}
pub fn status(core: &Core) -> FlowStatus {
    let saved: HashMap<String, FlowPeerStatus> = core
        .data
        .lock()
        .map(|d| {
            d.flow_peers
                .iter()
                .map(|f| {
                    (
                        f.peer.clone(),
                        FlowPeerStatus {
                            edge: f.edge,
                            mode: f.mode,
                            delay_ms: f.delay_ms,
                            block_while_button: f.block_while_button,
                            clipboard: f.clipboard,
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let peers: Vec<String> = core
        .input
        .lock()
        .map(|s| s.flow.peers.keys().cloned().collect())
        .unwrap_or_default();
    let network: HashMap<String, (bool, Option<u32>)> = peers
        .into_iter()
        .map(|peer| {
            let info = (
                crate::network::is_connected(core, &peer),
                crate::network::rtt(core, &peer),
            );
            (peer, info)
        })
        .collect();
    let now = Instant::now();
    let backoff = link::Backoff::default();
    let state = core.input.lock();
    let flow = state.as_ref().ok().map(|s| &s.flow);
    let capture = flow
        .filter(|f| f.controller.is_some())
        .and_then(|_| platform::capture_status());
    FlowStatus {
        supported: supported(),
        reason: (!supported()).then(|| UNSUPPORTED.into()),
        peers: flow
            .map(|f| {
                f.peers
                    .iter()
                    .map(|(id, c)| {
                        (
                            id.clone(),
                            FlowPeerStatus {
                                edge: c.edge,
                                mode: c.mode,
                                delay_ms: c.delay.as_millis() as u64,
                                block_while_button: c.block_while_button,
                                clipboard: c.clipboard,
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        connecting: flow.and_then(|f| f.connecting().map(str::to_owned)),
        forwarding: flow.and_then(|f| f.forwarding().map(str::to_owned)),
        last_error: flow.and_then(|f| f.last_error.clone()),
        links: flow
            .map(|f| {
                f.peers
                    .keys()
                    .map(|id| {
                        let (connected, rtt) = network.get(id).copied().unwrap_or((false, None));
                        let link = f.links.get(id).cloned().unwrap_or_default();
                        let state = if connected {
                            "connected"
                        } else if f.remote_closed.contains(id) {
                            "remote_closed"
                        } else if link.gave_up {
                            "gave_up"
                        } else if link.in_flight {
                            "reconnecting"
                        } else if link.next_at.is_some() {
                            "waiting"
                        } else {
                            "offline"
                        };
                        (
                            id.clone(),
                            FlowLinkStatus {
                                state,
                                attempt: link.attempt,
                                max_attempts: backoff.max_attempts,
                                next_retry_ms: link.retry_in(now).map(|d| d.as_millis() as u64),
                                rtt_ms: rtt.filter(|_| connected),
                                last_error: link.last_error,
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        capture,
        saved,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn configure(
    core: &Arc<Core>,
    peer: &str,
    enabled: bool,
    edge: FlowEdge,
    mode: FlowMode,
    delay_ms: u64,
    block_while_button: bool,
    clipboard: bool,
) -> Result<FlowStatus, String> {
    if !enabled {
        {
            let mut guard = core.input.lock().map_err(|_| "Input lock unavailable.")?;
            let state = &mut *guard;
            if state.flow.forwarding() == Some(peer) && state.sending_to.as_deref() == Some(peer) {
                state.sending_to = None;
                let releases = local_end(&mut state.flow, peer);
                release_remote(core, peer, releases);
                crate::network::try_send(core, peer, Wire::ControlStop);
            }
            disarm_peer(&mut state.flow, peer);
            state.flow.last_error = None;
        }
        if let Err(error) = forget_saved(core, peer) {
            core.warning(error);
        }
        return Ok(status(core));
    }
    let config = FlowConfig::new(edge, mode, delay_ms, block_while_button, clipboard)?;
    arm(core, peer, config, false)?;
    if let Err(error) = save(core, peer, config) {
        core.warning(format!(
            "Flow is on, but it could not be saved for the next launch: {error}"
        ));
    }
    Ok(status(core))
}

fn arm(core: &Arc<Core>, peer: &str, config: FlowConfig, restoring: bool) -> Result<(), String> {
    if !supported() {
        return Err(UNSUPPORTED.into());
    }
    crate::input::enabled(core, peer)?;
    let connected = crate::network::is_connected(core, peer);
    if !connected && !restoring {
        return Err("Connect to this computer before turning on Flow.".into());
    }
    if config.clipboard && !crate::network::clipboard_permitted(core, peer) {
        return Err("To copy and paste between computers, turn on clipboard sharing in Settings and allow it for this computer.".into());
    }
    let mut state = core.input.lock().map_err(|_| "Input lock unavailable.")?;
    if state
        .flow
        .peers
        .iter()
        .any(|(id, c)| id != peer && c.edge == config.edge)
    {
        return Err("This edge is already assigned to another computer.".into());
    }
    if state.flow.controller.is_none() {
        let (tx, rx) = unbounded_channel();
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        let hook = platform::HookThread::start(tx.clone())?;
        state.flow.controller = Some(Controller {
            tx,
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            hook,
        });
        tokio::spawn(controller(core.clone(), rx));
    }
    state.flow.peers.insert(peer.into(), config);
    state.flow.links.remove(peer);
    state.flow.remote_closed.remove(peer);
    if connected {
        state.flow.suspended.remove(peer);
        platform::with_logic(|logic| logic.set_arm(peer, config));
    } else {
        state.flow.suspended.insert(peer.into());
        platform::with_logic(|logic| logic.remove_arm(peer));
    }
    if !restoring
        || state
            .flow
            .last_error
            .as_ref()
            .is_some_and(|e| e.peer.as_deref() == Some(peer))
    {
        state.flow.last_error = None;
    }
    Ok(())
}

fn saved_entry(peer: &str, config: FlowConfig) -> SavedFlow {
    SavedFlow {
        peer: peer.into(),
        edge: config.edge,
        mode: config.mode,
        delay_ms: config.delay.as_millis() as u64,
        block_while_button: config.block_while_button,
        clipboard: config.clipboard,
    }
}

fn save(core: &Core, peer: &str, config: FlowConfig) -> Result<(), String> {
    let mut data = core.data.lock().map_err(|_| "Settings lock unavailable.")?;
    let entry = saved_entry(peer, config);
    let mut updated = data.clone();
    updated
        .flow_peers
        .retain(|f| f.peer != peer && f.edge != entry.edge);
    updated.flow_peers.push(entry);
    if updated.flow_peers == data.flow_peers {
        return Ok(());
    }
    crate::storage::save(&core.directory, &updated)?;
    *data = updated;
    Ok(())
}

pub fn forget_saved(core: &Core, peer: &str) -> Result<(), String> {
    let mut data = core.data.lock().map_err(|_| "Settings lock unavailable.")?;
    let mut updated = data.clone();
    if !updated.forget_flow(peer) {
        return Ok(());
    }
    crate::storage::save(&core.directory, &updated)?;
    *data = updated;
    Ok(())
}

pub fn restore_saved(core: &Arc<Core>) {
    let saved: Vec<SavedFlow> = core
        .data
        .lock()
        .map(|d| d.flow_peers.clone())
        .unwrap_or_default();
    for entry in saved {
        let armed = core
            .input
            .lock()
            .map(|s| s.flow.peers.contains_key(&entry.peer))
            .unwrap_or(true);
        if armed {
            continue;
        }
        let result = FlowConfig::new(
            entry.edge,
            entry.mode,
            entry.delay_ms,
            entry.block_while_button,
            entry.clipboard,
        )
        .and_then(|config| arm(core, &entry.peer, config, true));
        if let Err(error) = result {
            if let Ok(mut state) = core.input.lock() {
                state.flow.last_error = Some(FlowError::new(
                    "restore_failed",
                    &entry.peer,
                    format!("Flow could not be turned back on: {error}"),
                ));
            }
        }
    }
}

pub async fn repair(core: &Arc<Core>, peer: &str) -> Result<FlowStatus, String> {
    {
        let mut guard = core.input.lock().map_err(|_| "Input lock unavailable.")?;
        let state = &mut *guard;
        if !state.flow.peers.contains_key(peer) {
            return Err("Turn on Flow for this computer before repairing.".into());
        }
        if state.flow.switch.peer() == Some(peer) {
            if state.sending_to.as_deref() == Some(peer) {
                state.sending_to = None;
            }
            let releases = local_end(&mut state.flow, peer);
            release_remote(core, peer, releases);
            crate::network::try_send(core, peer, Wire::ControlStop);
        }
        let now = Instant::now();
        let link = state.flow.links.entry(peer.into()).or_default();
        if link.in_flight {
            return Err("A reconnection attempt is already in progress.".into());
        }
        link.kick(now, Duration::ZERO);
        link.take_due(now);
        state.flow.remote_closed.remove(peer);
        if state
            .flow
            .last_error
            .as_ref()
            .is_some_and(|e| e.peer.as_deref() != Some(peer) || e.code != "permission_changed")
        {
            state.flow.last_error = None;
        }
    }
    platform::reinstall();
    let result = match crate::network::peer_address(core, peer) {
        Some(address) => {
            crate::network::drop_session(core, peer);
            reconnect(core, peer, address).await
        }
        None => Err("No known address for this computer yet.".into()),
    };
    record_link(core, peer, result.clone(), &link::Backoff::default());
    if let Err(error) = result {
        if let Ok(mut state) = core.input.lock() {
            state.flow.last_error = Some(FlowError::new("reconnect_failed", peer, error));
        }
    }
    Ok(status(core))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn set_flow(
    peer_id: String,
    enabled: bool,
    edge: FlowEdge,
    mode: Option<FlowMode>,
    delay_ms: u64,
    block_while_button: bool,
    clipboard: Option<bool>,
    state: tauri::State<'_, Arc<Core>>,
) -> Result<FlowStatus, String> {
    configure(
        state.inner(),
        &peer_id,
        enabled,
        edge,
        mode.unwrap_or_default(),
        delay_ms,
        block_while_button,
        clipboard.unwrap_or(false),
    )
}
#[tauri::command]
pub fn get_flow_status(state: tauri::State<'_, Arc<Core>>) -> FlowStatus {
    status(state.inner())
}
#[tauri::command]
pub async fn repair_flow(
    peer_id: String,
    state: tauri::State<'_, Arc<Core>>,
) -> Result<FlowStatus, String> {
    repair(state.inner(), &peer_id).await
}

#[cfg(target_os = "windows")]
use win as platform;
#[cfg(target_os = "macos")]
use mac as platform;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "windows")]
pub(crate) mod hooks {
    pub use super::win::{acquire, installed, release, USER_REMAP};
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod platform {
    use super::{CaptureLogic, CaptureStatus, FlowEdge};
    pub const REMOVED_DETAIL: &str = "The input capture was removed.";
    pub fn set_forwarding(_: bool) {}
    pub fn begin_forwarding() {}
    pub fn heartbeat() {}
    pub fn place_cursor(_: FlowEdge, _: f64) {}
    pub fn with_logic<R>(_: impl FnOnce(&mut CaptureLogic) -> R) -> Option<R> {
        None
    }
    pub fn take_moves() -> (i32, i32) {
        (0, 0)
    }
    pub fn reinstall() {}
    pub fn capture_status() -> Option<CaptureStatus> {
        None
    }
}

#[cfg(target_os = "windows")]
mod win {
    use super::{
        Bounds, CaptureLogic, CaptureStatus, Captured, FlowEdge, HookHealth, RawEvent, Verdict,
        EDGE_INSET_PX,
    };
    use crate::screen::{self, Layout};
    use std::{
        sync::{
            atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicU8, Ordering},
            Mutex, OnceLock,
        },
        time::{Duration, Instant},
    };
    use tokio::sync::mpsc::UnboundedSender;
    use windows_sys::Win32::{
        Foundation::{LPARAM, LRESULT, WPARAM},
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
        UI::WindowsAndMessaging::*,
    };

    struct HookCtx {
        logic: CaptureLogic,
        tx: UnboundedSender<Captured>,
        layout: Layout,
    }
    static CTX: Mutex<Option<HookCtx>> = Mutex::new(None);
    static FORWARDING: AtomicBool = AtomicBool::new(false);
    static HEARTBEAT: AtomicU64 = AtomicU64::new(0);
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    static PENDING_DX: AtomicI32 = AtomicI32::new(0);
    static PENDING_DY: AtomicI32 = AtomicI32::new(0);
    static EVENTS: AtomicU32 = AtomicU32::new(0);
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    static REINSTALLS: AtomicU32 = AtomicU32::new(0);
    static LAST_REINSTALL_MS: AtomicU64 = AtomicU64::new(0);
    static REASON: AtomicU8 = AtomicU8::new(0);
    static THREAD_ID: AtomicU32 = AtomicU32::new(0);
    const REASON_REMOVED: u8 = 1;
    const REASON_REPAIR: u8 = 2;
    pub const REMOVED_DETAIL: &str = "Windows removed the low-level input hooks.";
    const WM_REPAIR: u32 = WM_APP + 1;
    const TIMER_MS: u32 = 500;
    const LAYOUT_REFRESH: Duration = Duration::from_secs(5);
    const STALE: Duration = Duration::from_secs(2);

    fn now_ms() -> u64 {
        EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64
    }
    pub fn heartbeat() {
        HEARTBEAT.store(now_ms(), Ordering::Relaxed);
    }
    static HIRES_TIMER: AtomicBool = AtomicBool::new(false);
    pub fn set_forwarding(value: bool) {
        FORWARDING.store(value, Ordering::SeqCst);
        if HIRES_TIMER.swap(value, Ordering::SeqCst) != value {
            unsafe {
                if value {
                    windows_sys::Win32::Media::timeBeginPeriod(1);
                } else {
                    windows_sys::Win32::Media::timeEndPeriod(1);
                }
            }
        }
    }
    pub fn with_logic<R>(f: impl FnOnce(&mut CaptureLogic) -> R) -> Option<R> {
        let mut guard = CTX.lock().ok()?;
        guard.as_mut().map(|ctx| f(&mut ctx.logic))
    }
    pub fn take_moves() -> (i32, i32) {
        (
            PENDING_DX.swap(0, Ordering::Relaxed),
            PENDING_DY.swap(0, Ordering::Relaxed),
        )
    }
    fn send(message: Captured) {
        if let Ok(guard) = CTX.lock() {
            if let Some(ctx) = guard.as_ref() {
                let _ = ctx.tx.send(message);
            }
        }
    }
    fn current_layout() -> Option<Layout> {
        screen::win::layout().or_else(|| {
            let [x, y, cx, cy, _] = screen::win::signature();
            Bounds::new(x, y, cx, cy).map(Layout::single)
        })
    }
    const MODIFIER_VKS: [(u32, i32); 8] = [
        (0x2A, 0xA0),
        (0x36, 0xA1),
        (0x1D, 0xA2),
        (0x11D, 0xA3),
        (0x38, 0xA4),
        (0x138, 0xA5),
        (0x15B, 0x5B),
        (0x15C, 0x5C),
    ];
    fn modifiers_down() -> impl Iterator<Item = u32> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        MODIFIER_VKS
            .into_iter()
            .filter(|(_, vk)| unsafe { GetAsyncKeyState(*vk) } as u16 & 0x8000 != 0)
            .map(|(id, _)| id)
    }
    pub fn begin_forwarding() {
        take_moves();
        let anchor = CTX.lock().ok().and_then(|mut guard| {
            guard.as_mut().map(|ctx| {
                let anchor = ctx.layout.monitor_for((0, 0)).center();
                ctx.logic.start_forwarding(anchor);
                ctx.logic.adopt_local_keys(modifiers_down());
                anchor
            })
        });
        let Some(anchor) = anchor else {
            return;
        };
        heartbeat();
        screen::win::set_cursor(anchor);
        set_forwarding(true);
    }
    pub fn place_cursor(edge: FlowEdge, entry: f64) {
        let point = CTX.lock().ok().and_then(|guard| {
            guard
                .as_ref()
                .map(|ctx| ctx.layout.entry_point(edge, entry, EDGE_INSET_PX))
        });
        if let Some(point) = point {
            screen::win::set_cursor(point);
        }
    }
    pub fn reinstall() {
        let thread = THREAD_ID.load(Ordering::SeqCst);
        if thread != 0 {
            unsafe { PostThreadMessageW(thread, WM_REPAIR, 0, 0) };
        }
    }
    pub fn capture_status() -> Option<CaptureStatus> {
        if THREAD_ID.load(Ordering::SeqCst) == 0 {
            return None;
        }
        let last = LAST_REINSTALL_MS.load(Ordering::Relaxed);
        Some(CaptureStatus {
            installed: INSTALLED.load(Ordering::SeqCst),
            reinstalls: REINSTALLS.load(Ordering::Relaxed),
            last_reinstall_ago_ms: (last > 0).then(|| now_ms().saturating_sub(last)),
            reason: match REASON.load(Ordering::Relaxed) {
                REASON_REMOVED => Some("removed"),
                REASON_REPAIR => Some("repair"),
                _ => None,
            },
        })
    }

    fn handle(event: RawEvent, injected: bool) -> bool {
        EVENTS.fetch_add(1, Ordering::Relaxed);
        let mut forwarding = FORWARDING.load(Ordering::SeqCst);
        if forwarding
            && now_ms().saturating_sub(HEARTBEAT.load(Ordering::Relaxed)) > STALE.as_millis() as u64
        {
            set_forwarding(false);
            forwarding = false;
        }
        let Ok(mut guard) = CTX.lock() else {
            return false;
        };
        let Some(HookCtx { logic, tx, layout }) = guard.as_mut() else {
            return false;
        };
        let (verdict, output) = logic.handle(event, injected, forwarding, layout, Instant::now());
        match output {
            Some(Captured::Move { dx, dy }) => {
                PENDING_DX.fetch_add(dx, Ordering::Relaxed);
                PENDING_DY.fetch_add(dy, Ordering::Relaxed);
            }
            Some(Captured::Emergency) => {
                set_forwarding(false);
                let _ = tx.send(Captured::Emergency);
            }
            Some(other) => {
                let _ = tx.send(other);
            }
            None => {}
        }
        verdict == Verdict::Swallow
    }

    unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 && lparam != 0 {
            let info = &*(lparam as *const MSLLHOOKSTRUCT);
            let injected = info.flags & LLMHF_INJECTED != 0;
            let wheel = ((info.mouseData >> 16) as u16 as i16) as i32;
            let event = match wparam as u32 {
                WM_MOUSEMOVE => Some(RawEvent::Move {
                    x: info.pt.x,
                    y: info.pt.y,
                    from: screen::win::cursor(),
                }),
                WM_LBUTTONDOWN | WM_LBUTTONUP => Some(RawEvent::Button {
                    index: 0,
                    down: wparam as u32 == WM_LBUTTONDOWN,
                }),
                WM_MBUTTONDOWN | WM_MBUTTONUP => Some(RawEvent::Button {
                    index: 1,
                    down: wparam as u32 == WM_MBUTTONDOWN,
                }),
                WM_RBUTTONDOWN | WM_RBUTTONUP => Some(RawEvent::Button {
                    index: 2,
                    down: wparam as u32 == WM_RBUTTONDOWN,
                }),
                WM_XBUTTONDOWN | WM_XBUTTONUP => Some(RawEvent::Button {
                    index: 3,
                    down: wparam as u32 == WM_XBUTTONDOWN,
                }),
                WM_MOUSEWHEEL => Some(RawEvent::Wheel {
                    delta: wheel,
                    horizontal: false,
                }),
                WM_MOUSEHWHEEL => Some(RawEvent::Wheel {
                    delta: wheel,
                    horizontal: true,
                }),
                _ => None,
            };
            if let Some(event) = event {
                let flow = handle(event, injected);
                let remap = !injected && remap_mouse(wparam as u32, info.mouseData);
                if flow || remap {
                    return 1;
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }
    fn remap_mouse(message: u32, data: u32) -> bool {
        use crate::remap::MouseButton;
        let (button, down) = match message {
            WM_MBUTTONDOWN | WM_MBUTTONUP => (MouseButton::Middle, message == WM_MBUTTONDOWN),
            WM_XBUTTONDOWN | WM_XBUTTONUP => {
                let button = match (data >> 16) as u16 {
                    XBUTTON1 => MouseButton::Back,
                    XBUTTON2 => MouseButton::Forward,
                    _ => return false,
                };
                (button, message == WM_XBUTTONDOWN)
            }
            _ => return false,
        };
        crate::remap::hook_mouse(button, down, FORWARDING.load(Ordering::SeqCst))
    }
    unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 && lparam != 0 {
            let info = &*(lparam as *const KBDLLHOOKSTRUCT);
            let message = wparam as u32;
            if matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP) {
                let extended = info.flags & LLKHF_EXTENDED != 0;
                let down = info.flags & LLKHF_UP == 0;
                let injected = info.flags & LLKHF_INJECTED != 0;
                let event = RawEvent::Key {
                    scan: info.scanCode & 0xFF,
                    extended,
                    down,
                };
                let flow = handle(event, injected);
                let remap = !injected
                    && crate::remap::hook_key(
                        info.scanCode,
                        extended,
                        down,
                        FORWARDING.load(Ordering::SeqCst),
                    );
                if flow || remap {
                    return 1;
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    struct Hooks {
        mouse: HHOOK,
        keyboard: HHOOK,
    }
    unsafe fn install() -> Option<Hooks> {
        let module = GetModuleHandleW(std::ptr::null());
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0);
        let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0);
        if mouse.is_null() || keyboard.is_null() {
            if !mouse.is_null() {
                UnhookWindowsHookEx(mouse);
            }
            if !keyboard.is_null() {
                UnhookWindowsHookEx(keyboard);
            }
            return None;
        }
        Some(Hooks { mouse, keyboard })
    }
    unsafe fn uninstall(hooks: Hooks) {
        UnhookWindowsHookEx(hooks.mouse);
        UnhookWindowsHookEx(hooks.keyboard);
    }
    unsafe fn reinstall_hooks(hooks: &mut Option<Hooks>, reason: u8) {
        if let Some(old) = hooks.take() {
            uninstall(old);
        }
        match install() {
            Some(new) => {
                *hooks = Some(new);
                INSTALLED.store(true, Ordering::SeqCst);
                REINSTALLS.fetch_add(1, Ordering::Relaxed);
                LAST_REINSTALL_MS.store(now_ms().max(1), Ordering::Relaxed);
                REASON.store(reason, Ordering::Relaxed);
                send(Captured::HookReinstalled {
                    removed: reason == REASON_REMOVED,
                });
            }
            None => {
                INSTALLED.store(false, Ordering::SeqCst);
                set_forwarding(false);
                send(Captured::HookFailed);
            }
        }
    }
    fn refresh_layout() {
        if let Some(layout) = current_layout() {
            if let Ok(mut guard) = CTX.lock() {
                if let Some(ctx) = guard.as_mut() {
                    ctx.layout = layout;
                }
            }
        }
    }

    pub const USER_FLOW: u8 = 1;
    pub const USER_REMAP: u8 = 2;
    struct Running {
        users: u8,
        thread_id: u32,
        handle: Option<std::thread::JoinHandle<()>>,
    }
    static RUNNING: Mutex<Running> = Mutex::new(Running {
        users: 0,
        thread_id: 0,
        handle: None,
    });
    fn running() -> std::sync::MutexGuard<'static, Running> {
        RUNNING.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn installed() -> bool {
        INSTALLED.load(Ordering::SeqCst)
    }
    pub fn acquire(user: u8) -> Result<(), String> {
        let mut running = running();
        if running.handle.as_ref().is_none_or(|h| h.is_finished()) {
            if let Some(old) = running.handle.take() {
                let _ = old.join();
            }
            let (handle, thread_id) = spawn()?;
            running.handle = Some(handle);
            running.thread_id = thread_id;
        }
        running.users |= user;
        Ok(())
    }
    pub fn release(user: u8) {
        let mut running = running();
        running.users &= !user;
        if running.users != 0 {
            return;
        }
        if let Some(handle) = running.handle.take() {
            unsafe { PostThreadMessageW(running.thread_id, WM_QUIT, 0, 0) };
            let _ = handle.join();
        }
        running.thread_id = 0;
    }

    fn spawn() -> Result<(std::thread::JoinHandle<()>, u32), String> {
        let (ready, wait) = std::sync::mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("traplink-input-hooks".into())
            .spawn(move || unsafe {
                screen::win::dpi_aware();
                let mut msg: MSG = std::mem::zeroed();
                PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
                let mut hooks = install();
                if hooks.is_none() {
                    let _ = ready.send(Err(
                        "Could not install the global mouse and keyboard capture."
                            .to_string(),
                    ));
                    return;
                }
                let id = GetCurrentThreadId();
                INSTALLED.store(true, Ordering::SeqCst);
                THREAD_ID.store(id, Ordering::SeqCst);
                let _ = ready.send(Ok(id));
                let foreground = crate::remap::platform::install_foreground_hook();
                let timer = SetTimer(std::ptr::null_mut(), 0, TIMER_MS, None);
                let mut health = HookHealth::default();
                let mut last_cursor = screen::win::cursor();
                let mut signature = screen::win::signature();
                let mut layout_at = Instant::now();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    match msg.message {
                        WM_TIMER => {
                            let now = Instant::now();
                            let current = screen::win::signature();
                            if current != signature
                                || now.saturating_duration_since(layout_at) >= LAYOUT_REFRESH
                            {
                                signature = current;
                                layout_at = now;
                                refresh_layout();
                            }
                            let cursor = screen::win::cursor();
                            let moved = cursor.is_some() && cursor != last_cursor;
                            last_cursor = cursor;
                            let events = EVENTS.swap(0, Ordering::Relaxed);
                            if health.observe(moved, events, now) {
                                reinstall_hooks(&mut hooks, REASON_REMOVED);
                            }
                            crate::remap::refresh_foreground(false);
                        }
                        WM_REPAIR => {
                            refresh_layout();
                            reinstall_hooks(&mut hooks, REASON_REPAIR);
                            health = HookHealth::default();
                        }
                        _ => {
                            DispatchMessageW(&msg);
                        }
                    }
                }
                KillTimer(std::ptr::null_mut(), timer);
                crate::remap::platform::remove_foreground_hook(foreground);
                set_forwarding(false);
                if let Some(hooks) = hooks.take() {
                    uninstall(hooks);
                }
                INSTALLED.store(false, Ordering::SeqCst);
                let _ = THREAD_ID.compare_exchange(id, 0, Ordering::SeqCst, Ordering::SeqCst);
            })
            .map_err(|_| "Could not start mouse and keyboard capture.".to_string())?;
        match wait
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| "Mouse and keyboard capture did not respond.".to_string())
            .and_then(|r| r)
        {
            Ok(thread_id) => Ok((handle, thread_id)),
            Err(error) => {
                if handle.is_finished() {
                    let _ = handle.join();
                }
                Err(error)
            }
        }
    }

    pub struct HookThread;
    impl HookThread {
        pub fn start(tx: UnboundedSender<Captured>) -> Result<Self, String> {
            let layout = current_layout()
                .ok_or_else(|| "Could not read the display layout.".to_string())?;
            *CTX.lock().map_err(|_| "Flow lock unavailable.")? = Some(HookCtx {
                logic: CaptureLogic::default(),
                tx,
                layout,
            });
            set_forwarding(false);
            take_moves();
            REINSTALLS.store(0, Ordering::Relaxed);
            LAST_REINSTALL_MS.store(0, Ordering::Relaxed);
            REASON.store(0, Ordering::Relaxed);
            if let Err(error) = acquire(USER_FLOW) {
                if let Ok(mut ctx) = CTX.lock() {
                    *ctx = None;
                }
                return Err(error);
            }
            Ok(Self)
        }
        pub fn stop(&self) {
            set_forwarding(false);
            if let Ok(mut ctx) = CTX.lock() {
                *ctx = None;
            }
            release(USER_FLOW);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Bounds {
        Bounds::new(0, 0, 1920, 1080).unwrap()
    }
    fn config(edge: FlowEdge, mode: FlowMode, delay: u64, block: bool) -> FlowConfig {
        FlowConfig::new(edge, mode, delay, block, false).unwrap()
    }

    #[test]
    fn dead_zone_requires_dwell_at_edge() {
        let mut tracker = EdgeTracker::new(config(FlowEdge::Right, FlowMode::Edge, 250, true));
        let t0 = Instant::now();
        assert_eq!(
            tracker.update((1000, 540), screen(), false, false, t0),
            None
        );
        assert_eq!(
            tracker.update((1919, 540), screen(), false, false, t0),
            None
        );
        let early = t0 + Duration::from_millis(200);
        assert_eq!(
            tracker.update((1919, 540), screen(), false, false, early),
            None
        );
        assert_eq!(
            tracker.update((1900, 540), screen(), false, false, early),
            None
        );
        let t1 = t0 + Duration::from_millis(300);
        assert_eq!(
            tracker.update((1919, 540), screen(), false, false, t1),
            None
        );
        let entry = tracker
            .update(
                (1919, 540),
                screen(),
                false,
                false,
                t1 + Duration::from_millis(250),
            )
            .unwrap();
        assert!((entry - 540.0 / 1079.0).abs() < 1e-9);
    }

    #[test]
    fn rearm_after_trigger_requires_leaving_edge() {
        let mut tracker = EdgeTracker::new(config(FlowEdge::Left, FlowMode::Edge, 0, true));
        let now = Instant::now();
        assert!(tracker
            .update((0, 10), screen(), false, false, now)
            .is_some());
        assert!(tracker
            .update((0, 10), screen(), false, false, now)
            .is_none());
        assert!(tracker
            .update((10, 10), screen(), false, false, now)
            .is_none());
        assert!(tracker
            .update((0, 10), screen(), false, false, now)
            .is_none());
        assert!(tracker
            .update((REARM_PX, 10), screen(), false, false, now)
            .is_none());
        assert!(tracker
            .update((0, 10), screen(), false, false, now)
            .is_some());
    }

    #[test]
    fn mouse_button_blocks_switch_when_configured() {
        let now = Instant::now();
        let mut blocked = EdgeTracker::new(config(FlowEdge::Top, FlowMode::Edge, 0, true));
        assert!(blocked
            .update((50, 0), screen(), true, false, now)
            .is_none());
        assert!(blocked
            .update((50, 0), screen(), false, false, now)
            .is_some());
        let mut allowed = EdgeTracker::new(config(FlowEdge::Top, FlowMode::Edge, 0, false));
        assert!(allowed
            .update((50, 0), screen(), true, false, now)
            .is_some());
    }

    #[test]
    fn ctrl_mode_switches_only_with_ctrl() {
        let now = Instant::now();
        let mut tracker =
            EdgeTracker::new(config(FlowEdge::Bottom, FlowMode::CtrlEdge, 1000, true));
        assert!(tracker
            .update((5, 1079), screen(), false, false, now)
            .is_none());
        assert!(tracker
            .update(
                (5, 1079),
                screen(),
                false,
                false,
                now + Duration::from_secs(5)
            )
            .is_none());
        assert!(tracker
            .update(
                (5, 1079),
                screen(),
                false,
                true,
                now + Duration::from_secs(5)
            )
            .is_some());
    }

    #[test]
    fn delay_is_bounded() {
        assert!(FlowConfig::new(FlowEdge::Left, FlowMode::Edge, 1001, true, false).is_err());
        assert!(FlowConfig::new(FlowEdge::Left, FlowMode::Edge, 1000, true, false).is_ok());
    }

    #[test]
    fn negative_virtual_screen_edges() {
        let bounds = Bounds::new(-1280, -200, 3200, 1280).unwrap();
        let mut tracker = EdgeTracker::new(config(FlowEdge::Left, FlowMode::Edge, 0, true));
        assert!(tracker
            .update((-1279, 0), bounds, false, false, Instant::now())
            .is_none());
        assert_eq!(
            tracker.update((-1280, -200), bounds, false, false, Instant::now()),
            None
        );
        assert_eq!(
            tracker.update((-1280, -206), bounds, false, false, Instant::now()),
            None
        );
        assert_eq!(
            tracker.update((-1281, -200), bounds, false, false, Instant::now()),
            Some(0.0)
        );
        assert_eq!(bounds.point_on(FlowEdge::Right, 1.0, 2), (1917, 1079));
    }

    #[test]
    fn return_watch_requires_push_past_opposite_edge() {
        let mut watch = ReturnWatch::new(FlowEdge::Right);
        assert_eq!(watch.edge, FlowEdge::Left);
        let bounds = screen();
        let (target, ret) = watch.step((100, 300), -50, 0, bounds);
        assert_eq!((target, ret), ((50, 300), None));
        let (target, ret) = watch.step((2, 300), -4, 0, bounds);
        assert_eq!((target, ret), ((0, 300), None));
        let (_, ret) = watch.step((0, 300), -3, 0, bounds);
        assert!(ret.is_none(), "below the push threshold");
        let (_, ret) = watch.step((0, 300), -1, 0, bounds);
        assert!(ret.is_some());
        let mut watch = ReturnWatch::new(FlowEdge::Right);
        assert_eq!(watch.step((1919, 5), 500, 0, bounds).1, None);
        assert_eq!(watch.step((0, 0), 0, -500, bounds).1, None);
    }

    #[test]
    fn protocol_messages_parse_and_validate() {
        let start: Wire = serde_json::from_str(
            r#"{"kind":"flow_start","request":"r1","edge":"right","entry":0.25,"clipboard":false}"#,
        )
        .unwrap();
        assert!(matches!(
            start,
            Wire::FlowStart { edge: FlowEdge::Right, entry, clipboard: false, .. } if entry == 0.25
        ));
        assert!(serde_json::from_str::<Wire>(
            r#"{"kind":"flow_start","request":"r1","edge":"diagonal","entry":0.25,"clipboard":false}"#
        )
        .is_err());
        let back: Wire = serde_json::from_str(r#"{"kind":"flow_return","entry":1.5}"#).unwrap();
        let Wire::FlowReturn { entry } = back else {
            panic!("wrong variant")
        };
        assert!(validate_entry(entry).is_err());
        assert!(validate_entry(f64::NAN).is_err());
        assert!(validate_entry(0.0).is_ok() && validate_entry(1.0).is_ok());
        let delta: InputEvent =
            serde_json::from_str(r#"{"type":"move_by","dx":5,"dy":-3}"#).unwrap();
        assert!(crate::input::validate(&delta).is_ok());
        assert!(crate::input::validate(&InputEvent::MoveBy {
            dx: MAX_DELTA + 1,
            dy: 0
        })
        .is_err());
    }

    #[test]
    fn scan_codes_map_to_physical_codes() {
        assert_eq!(scan_code(0x1E, false), Some("KeyA"));
        assert_eq!(scan_code(0x10, false), Some("KeyQ"));
        assert_eq!(scan_code(0x0B, false), Some("Digit0"));
        assert_eq!(scan_code(0x1C, true), Some("NumpadEnter"));
        assert_eq!(scan_code(0x4B, true), Some("ArrowLeft"));
        assert_eq!(scan_code(0x4B, false), Some("Numpad4"));
        assert_eq!(scan_code(0x45, false), None);
        for scan in 0..=0xFF {
            for extended in [false, true] {
                if let Some(code) = scan_code(scan, extended) {
                    let event = InputEvent::Key {
                        key: code.into(),
                        down: true,
                    };
                    let typeable = !(cfg!(target_os = "macos") && code == "ContextMenu");
                    assert_eq!(crate::input::validate(&event).is_ok(), typeable, "{code}");
                }
            }
        }
    }

    fn armed_logic(edge: FlowEdge) -> CaptureLogic {
        let mut logic = CaptureLogic::default();
        logic.set_arm("peer", config(edge, FlowMode::Edge, 0, true));
        logic
    }

    #[test]
    fn capture_switches_forwards_and_swallows() {
        let mut logic = armed_logic(FlowEdge::Right);
        let now = Instant::now();
        let s = &Layout::single(screen());
        assert_eq!(
            logic.handle(RawEvent::Move { x: 1919, y: 0, from: None }, true, false, s, now),
            (Verdict::Pass, None)
        );
        let (verdict, out) = logic.handle(RawEvent::Move { x: 1921, y: 0, from: None }, false, false, s, now);
        assert_eq!(verdict, Verdict::Pass);
        assert_eq!(
            out,
            Some(Captured::EdgeHit {
                peer: "peer".into(),
                entry: 0.0
            })
        );
        logic.start_forwarding((960, 540));
        assert_eq!(
            logic.handle(RawEvent::Move { x: 970, y: 535, from: None }, false, true, s, now),
            (Verdict::Swallow, Some(Captured::Move { dx: 10, dy: -5 }))
        );
        let key = RawEvent::Key {
            scan: 0x1E,
            extended: false,
            down: true,
        };
        assert_eq!(
            logic.handle(key, false, true, s, now),
            (
                Verdict::Swallow,
                Some(Captured::Event(InputEvent::Key {
                    key: "KeyA".into(),
                    down: true
                }))
            )
        );
        assert_eq!(
            logic.handle(key, false, true, s, now),
            (Verdict::Swallow, None)
        );
        assert_eq!(
            logic.handle(
                RawEvent::Wheel {
                    delta: 60,
                    horizontal: false
                },
                false,
                true,
                s,
                now
            ),
            (Verdict::Swallow, None)
        );
        assert_eq!(
            logic.handle(
                RawEvent::Wheel {
                    delta: 60,
                    horizontal: false
                },
                false,
                true,
                s,
                now
            ),
            (
                Verdict::Swallow,
                Some(Captured::Event(InputEvent::Scroll { x: 0, y: -1 }))
            )
        );
        assert_eq!(
            logic.handle(
                RawEvent::Button {
                    index: 3,
                    down: true
                },
                false,
                true,
                s,
                now
            ),
            (Verdict::Swallow, None)
        );
    }

    #[test]
    fn keys_held_before_switch_release_locally() {
        let mut logic = armed_logic(FlowEdge::Right);
        let s = &Layout::single(screen());
        let now = Instant::now();
        let shift = |down| RawEvent::Key {
            scan: 0x2A,
            extended: false,
            down,
        };
        assert_eq!(
            logic.handle(shift(true), false, false, s, now).0,
            Verdict::Pass
        );
        logic.start_forwarding((960, 540));
        assert_eq!(
            logic.handle(shift(false), false, true, s, now),
            (Verdict::Pass, None)
        );
        let left = |down| RawEvent::Button { index: 0, down };
        logic.end_forwarding("peer");
        assert_eq!(
            logic.handle(left(true), false, false, s, now).0,
            Verdict::Pass
        );
        logic.start_forwarding((960, 540));
        assert_eq!(
            logic.handle(left(false), false, true, s, now),
            (Verdict::Pass, None)
        );
    }

    fn key(scan: u32, extended: bool, down: bool) -> RawEvent {
        RawEvent::Key {
            scan,
            extended,
            down,
        }
    }
    fn key_out(code: &str, down: bool) -> Option<Captured> {
        Some(Captured::Event(InputEvent::Key {
            key: code.into(),
            down,
        }))
    }

    #[test]
    fn alt_held_across_the_edge_is_released_locally_and_never_reaches_the_remote() {
        let mut logic = armed_logic(FlowEdge::Right);
        let s = &Layout::single(screen());
        let now = Instant::now();
        assert_eq!(logic.handle(key(0x38, false, true), false, false, s, now).0, Verdict::Pass);
        assert_eq!(logic.handle(key(0x0F, false, true), false, false, s, now).0, Verdict::Pass);
        assert_eq!(logic.handle(key(0x0F, false, false), false, false, s, now).0, Verdict::Pass);
        logic.start_forwarding((960, 540));
        assert_eq!(
            logic.handle(key(0x38, false, true), false, true, s, now),
            (Verdict::Swallow, None),
            "auto-repeat of a local Alt is not forwarded"
        );
        assert_eq!(
            logic.handle(key(0x38, false, false), false, true, s, now),
            (Verdict::Pass, None)
        );
        assert_eq!(
            logic.handle(key(0x38, false, true), false, true, s, now),
            (Verdict::Swallow, key_out("AltLeft", true))
        );
        assert_eq!(
            logic.end_forwarding("peer"),
            vec![InputEvent::Key {
                key: "AltLeft".into(),
                down: false
            }]
        );
    }

    #[test]
    fn returning_releases_the_remote_and_swallows_the_late_local_release() {
        let mut logic = armed_logic(FlowEdge::Right);
        let s = &Layout::single(screen());
        let now = Instant::now();
        logic.start_forwarding((960, 540));
        assert_eq!(
            logic.handle(key(0x1D, false, true), false, true, s, now),
            (Verdict::Swallow, key_out("ControlLeft", true))
        );
        assert_eq!(
            logic.handle(key(0x2C, false, true), false, true, s, now),
            (Verdict::Swallow, key_out("KeyZ", true))
        );
        let left = |down| RawEvent::Button { index: 0, down };
        assert_eq!(
            logic.handle(left(true), false, true, s, now),
            (
                Verdict::Swallow,
                Some(Captured::Event(InputEvent::Button {
                    button: 0,
                    down: true
                }))
            )
        );
        assert_eq!(
            logic.end_forwarding("peer"),
            vec![
                InputEvent::Button {
                    button: 0,
                    down: false
                },
                InputEvent::Key {
                    key: "KeyZ".into(),
                    down: false
                },
                InputEvent::Key {
                    key: "ControlLeft".into(),
                    down: false
                },
            ]
        );
        assert!(logic.end_forwarding("peer").is_empty());
        assert_eq!(
            logic.handle(key(0x1D, false, false), false, false, s, now),
            (Verdict::Swallow, None),
            "the local system never saw this Ctrl go down"
        );
        assert_eq!(
            logic.handle(left(false), false, false, s, now),
            (Verdict::Swallow, None)
        );
        assert_eq!(
            logic.handle(key(0x2C, false, true), false, false, s, now),
            (Verdict::Pass, None)
        );
        assert_eq!(
            logic.handle(key(0x2C, false, false), false, false, s, now),
            (Verdict::Pass, None)
        );
        assert_eq!(
            logic.handle(key(0x1D, false, false), false, false, s, now),
            (Verdict::Pass, None)
        );
    }

    #[test]
    fn modifiers_reported_down_by_the_system_release_locally() {
        let mut logic = armed_logic(FlowEdge::Right);
        let s = &Layout::single(screen());
        let now = Instant::now();
        logic.start_forwarding((960, 540));
        logic.adopt_local_keys([0x38, 0x15B]);
        assert_eq!(
            logic.handle(key(0x5B, true, false), false, true, s, now),
            (Verdict::Pass, None)
        );
        assert_eq!(
            logic.handle(key(0x38, false, false), false, true, s, now),
            (Verdict::Pass, None)
        );
        assert!(logic.end_forwarding("peer").is_empty());
    }

    #[test]
    fn swapped_command_and_control_release_with_the_forwarded_code() {
        let mut logic = armed_logic(FlowEdge::Right);
        logic.swap_command_control = true;
        let s = &Layout::single(screen());
        let now = Instant::now();
        logic.start_forwarding((960, 540));
        assert_eq!(
            logic.handle(key(0x5B, true, true), false, true, s, now),
            (Verdict::Swallow, key_out("ControlLeft", true))
        );
        assert!(logic.key_held(0x5B, true));
        assert_eq!(
            logic.end_forwarding("peer"),
            vec![InputEvent::Key {
                key: "ControlLeft".into(),
                down: false
            }]
        );
    }

    #[test]
    fn emergency_escape_stays_local() {
        let mut logic = armed_logic(FlowEdge::Right);
        let s = &Layout::single(screen());
        let now = Instant::now();
        logic.start_forwarding((960, 540));
        logic.handle(key(0x1D, false, true), false, true, s, now);
        logic.handle(key(0x38, false, true), false, true, s, now);
        assert_eq!(
            logic.handle(key(0x01, false, true), false, true, s, now),
            (Verdict::Pass, Some(Captured::Emergency))
        );
        let releases = logic.end_forwarding("peer");
        assert_eq!(releases.len(), 2);
        assert_eq!(
            logic.handle(key(0x01, false, false), false, false, s, now),
            (Verdict::Pass, None)
        );
    }

    #[test]
    fn ctrl_alt_escape_is_always_an_emergency() {
        let s = &Layout::single(screen());
        let now = Instant::now();
        for forwarding in [false, true] {
            let mut logic = armed_logic(FlowEdge::Left);
            logic.start_forwarding((960, 540));
            for (scan, extended) in [(0x1D, false), (0x38, true)] {
                logic.handle(
                    RawEvent::Key {
                        scan,
                        extended,
                        down: true,
                    },
                    false,
                    forwarding,
                    s,
                    now,
                );
            }
            assert_eq!(
                logic.handle(
                    RawEvent::Key {
                        scan: 0x01,
                        extended: false,
                        down: true
                    },
                    false,
                    forwarding,
                    s,
                    now
                ),
                (Verdict::Pass, Some(Captured::Emergency))
            );
        }
    }

    #[test]
    fn capture_respects_ctrl_mode_and_buttons() {
        let mut logic = CaptureLogic::default();
        logic.set_arm(
            "peer",
            config(FlowEdge::Left, FlowMode::CtrlEdge, 250, true),
        );
        let s = &Layout::single(screen());
        let now = Instant::now();
        assert_eq!(
            logic
                .handle(RawEvent::Move { x: 0, y: 5, from: None }, false, false, s, now)
                .1,
            None
        );
        logic.handle(
            RawEvent::Key {
                scan: 0x1D,
                extended: true,
                down: true,
            },
            false,
            false,
            s,
            now,
        );
        logic.handle(
            RawEvent::Button {
                index: 0,
                down: true,
            },
            false,
            false,
            s,
            now,
        );
        assert_eq!(
            logic
                .handle(RawEvent::Move { x: 0, y: 5, from: None }, false, false, s, now)
                .1,
            None
        );
        logic.handle(
            RawEvent::Button {
                index: 0,
                down: false,
            },
            false,
            false,
            s,
            now,
        );
        assert!(matches!(
            logic
                .handle(RawEvent::Move { x: 0, y: 5, from: None }, false, false, s, now)
                .1,
            Some(Captured::EdgeHit { .. })
        ));
    }

    fn mixed() -> Layout {
        Layout::new(vec![
            Bounds::new(0, 0, 1920, 1080).unwrap(),
            Bounds::new(-2560, -360, 2560, 1440).unwrap(),
        ])
        .unwrap()
    }

    #[test]
    fn edge_pressure_ignores_sliding_along_the_edge() {
        let mut tracker = EdgeTracker::new(config(FlowEdge::Right, FlowMode::Edge, 250, true));
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        assert_eq!(
            tracker.update((1919, 100), screen(), false, false, at(0)),
            None
        );
        for (i, ms) in [300u64, 400, 500, 600].into_iter().enumerate() {
            let y = 110 + i as i32 * 10;
            assert_eq!(
                tracker.update((1919, y), screen(), false, false, at(ms)),
                None
            );
        }
        assert!(tracker
            .update((1919, 140), screen(), false, false, at(650))
            .is_some());

        let mut tracker = EdgeTracker::new(config(FlowEdge::Right, FlowMode::Edge, 250, true));
        assert_eq!(
            tracker.update((1919, 100), screen(), false, false, at(0)),
            None
        );
        assert!(tracker
            .update((1925, 160), screen(), false, false, at(260))
            .is_some());
    }

    #[test]
    fn only_outer_edges_of_a_multi_monitor_layout_switch() {
        let layout = mixed();
        let now = Instant::now();
        let mut tracker = EdgeTracker::new(config(FlowEdge::Left, FlowMode::Edge, 0, true));
        assert_eq!(
            tracker.update_layout((0, 500), None, &layout, false, false, now),
            None
        );
        let entry = tracker.update_layout((-2561, -360), Some((-2560, -360)), &layout, false, false, now)
            .unwrap();
        assert!(entry.abs() < 1e-9, "top of the virtual desktop is entry 0");
        let top = || EdgeTracker::new(config(FlowEdge::Top, FlowMode::Edge, 0, true));
        assert!(top().update_layout((500, 0), None, &layout, false, false, now)
            .is_some());
        assert_eq!(
            top().update_layout((-500, 0), None, &layout, false, false, now),
            None
        );
        assert!(top().update_layout((-500, -360), None, &layout, false, false, now)
            .is_some());
    }

    #[test]
    fn return_watch_uses_the_receivers_whole_layout() {
        let layout = mixed();
        let mut watch = ReturnWatch::new(FlowEdge::Right);
        let (target, crossed) = watch.step_layout((0, 500), -40, 0, &layout);
        assert_eq!((target, crossed), ((-40, 500), None));
        let (target, crossed) = watch.step_layout((-2558, 500), -4, 0, &layout);
        assert_eq!((target, crossed), ((-2560, 500), None));
        let (_, crossed) = watch.step_layout((-2560, 500), -RETURN_PUSH_PX, 0, &layout);
        assert!(crossed.is_some());
    }

    #[test]
    fn capture_logic_switches_only_at_outer_edges() {
        let mut logic = CaptureLogic::default();
        logic.set_arm("peer", config(FlowEdge::Left, FlowMode::Edge, 0, true));
        let layout = mixed();
        let now = Instant::now();
        let seam = logic.handle(RawEvent::Move { x: 0, y: 200, from: None }, false, false, &layout, now);
        assert_eq!(seam, (Verdict::Pass, None));
        let outer = logic.handle(
            RawEvent::Move { x: -2560, y: 200, from: None },
            false,
            false,
            &layout,
            now,
        );
        assert!(matches!(outer.1, Some(Captured::EdgeHit { .. })));
    }

    #[test]
    fn hook_health_detects_silent_removal_with_cooldown() {
        let t0 = Instant::now();
        let mut health = HookHealth::default();
        assert!(!health.observe(false, 0, t0));
        assert!(!health.observe(true, 12, t0));
        assert!(!health.observe(true, 0, t0));
        assert!(!health.observe(true, 3, t0), "events reset the count");
        assert!(!health.observe(true, 0, t0));
        assert!(health.observe(true, 0, t0), "two in a row reinstall");
        let soon = t0 + Duration::from_secs(1);
        assert!(!health.observe(true, 0, soon));
        assert!(!health.observe(true, 0, soon));
        let later = t0 + REINSTALL_COOLDOWN;
        assert!(health.observe(true, 0, later));
        assert!(
            !health.observe(true, 0, later),
            "count restarts after a reinstall"
        );
    }

    #[test]
    fn start_errors_are_classified_for_the_interface() {
        let cases = [
            ("Input requires the global switch and this peer's input permission on both computers.", "peer_denied"),
            ("This computer is already controlling another peer; stop it before receiving input.", "peer_busy"),
            ("An input session is already active on this computer.", "peer_busy"),
            ("Input access is unavailable. On macOS, allow TrapLink in System Settings > Privacy & Security > Accessibility, then retry.", "peer_accessibility"),
            ("Peer is not connected.", "connection_lost"),
            ("timeout: the other computer did not confirm Flow.", "timeout"),
            ("Native input injection failed: blocked", "start_failed"),
        ];
        for (message, code) in cases {
            assert_eq!(classify_start_error(message), code, "{message}");
        }
    }

    #[test]
    fn switch_always_returns_local() {
        let peers = ["a", "b"];
        let mut seed: u64 = 0x5EED_F10E;
        let mut next = move |n: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        let t0 = Instant::now();
        for _ in 0..500 {
            let mut switch = Switch::default();
            let mut clock = t0;
            for _ in 0..40 {
                clock += Duration::from_millis(next(1500));
                let peer = peers[next(2) as usize].to_string();
                let event = match next(7) {
                    0 => SwitchEvent::Begin { peer, now: clock },
                    1 => SwitchEvent::Started {
                        peer,
                        ok: next(2) == 0,
                    },
                    2 => SwitchEvent::Returned { peer },
                    3 => SwitchEvent::Lost { peer },
                    4 => SwitchEvent::Emergency,
                    5 => SwitchEvent::HookLost,
                    _ => SwitchEvent::Tick { now: clock },
                };
                let before = switch.clone();
                switch.apply(event.clone());
                if let Switch::Forwarding { peer } = &switch {
                    let allowed = matches!(&before, Switch::Forwarding { peer: p } if p == peer)
                        || matches!((&before, &event),
                            (Switch::Starting { peer: p, .. }, SwitchEvent::Started { ok: true, .. }) if p == peer);
                    assert!(allowed, "{before:?} --{event:?}--> {switch:?}");
                }
                if let Switch::Starting { deadline, .. } = &switch {
                    assert!(*deadline <= clock + START_DEADLINE);
                }
                let mut exits = vec![
                    SwitchEvent::Emergency,
                    SwitchEvent::HookLost,
                    SwitchEvent::Tick {
                        now: clock + START_DEADLINE,
                    },
                ];
                if let Some(peer) = switch.peer() {
                    exits.push(SwitchEvent::Lost { peer: peer.into() });
                }
                for exit in exits {
                    let mut probe = switch.clone();
                    probe.apply(exit.clone());
                    let timer_only = matches!(exit, SwitchEvent::Tick { .. })
                        && matches!(switch, Switch::Forwarding { .. });
                    if !timer_only {
                        assert_eq!(probe, Switch::Local, "{switch:?} --{exit:?}-->");
                    }
                }
            }
        }
    }

    fn saved_fixture() -> (tempfile::TempDir, Arc<Core>, String) {
        use crate::model::{Peer, PeerPermissions, TrustedPeer};
        let directory = tempfile::tempdir().unwrap();
        let core = Core::for_test(directory.path().to_path_buf(), 48372);
        let id = uuid::Uuid::new_v4().to_string();
        let mut data = core.data.lock().unwrap();
        data.peers.push(TrustedPeer {
            peer: Peer {
                id: id.clone(),
                name: "Test peer".into(),
                address: "127.0.0.1:48373".into(),
                platform: "windows".into(),
                fingerprint: "test".into(),
                connected: false,
                trusted: true,
                last_seen: None,
                permissions: PeerPermissions {
                    files: false,
                    clipboard: false,
                    input: true,
                },
            },
            public_key: String::new(),
        });
        data.flow_peers.push(SavedFlow {
            peer: id.clone(),
            edge: FlowEdge::Right,
            mode: FlowMode::CtrlEdge,
            delay_ms: 250,
            block_while_button: true,
            clipboard: false,
        });
        crate::storage::save(&core.directory, &data).unwrap();
        drop(data);
        (directory, core, id)
    }

    #[test]
    fn saved_flow_survives_a_restart() {
        let (directory, core, id) = saved_fixture();
        let reloaded = crate::storage::load(directory.path(), "ignored".into()).unwrap();
        assert_eq!(reloaded.flow_peers, core.data.lock().unwrap().flow_peers);
        assert_eq!(reloaded.flow_peers[0].peer, id);
        assert_eq!(reloaded.flow_peers[0].mode, FlowMode::CtrlEdge);
        let status = status(&core);
        assert!(status.saved.contains_key(&id));
        assert!(status.peers.is_empty());
    }

    #[test]
    fn turning_flow_off_removes_the_saved_choice() {
        let (directory, core, id) = saved_fixture();
        let status = configure(&core, &id, false, FlowEdge::Right, FlowMode::Edge, 250, true, false)
            .unwrap();
        assert!(status.saved.is_empty());
        let reloaded = crate::storage::load(directory.path(), "ignored".into()).unwrap();
        assert!(reloaded.flow_peers.is_empty());
    }

    #[test]
    fn disconnecting_removes_the_saved_choice() {
        let (directory, core, id) = saved_fixture();
        user_disconnected(&core, &id);
        let reloaded = crate::storage::load(directory.path(), "ignored".into()).unwrap();
        assert!(reloaded.flow_peers.is_empty());
    }

    #[test]
    fn a_failed_restore_keeps_the_choice_and_reports_why() {
        let (directory, core, id) = saved_fixture();
        assert!(!core.data.lock().unwrap().settings.input_enabled);
        restore_saved(&core);
        let status = status(&core);
        assert!(status.peers.is_empty(), "nothing may look armed");
        let error = status.last_error.expect("restore error");
        assert_eq!(error.code, "restore_failed");
        assert_eq!(error.peer.as_deref(), Some(id.as_str()));
        assert!(status.saved.contains_key(&id));
        let reloaded = crate::storage::load(directory.path(), "ignored".into()).unwrap();
        assert_eq!(reloaded.flow_peers.len(), 1);
    }

    #[test]
    fn emergency_stop_keeps_saved_flow() {
        let (directory, core, id) = saved_fixture();
        crate::stop_all_input(&core, true);
        let reloaded = crate::storage::load(directory.path(), "ignored".into()).unwrap();
        assert_eq!(reloaded.flow_peers.len(), 1);
        assert_eq!(reloaded.flow_peers[0].peer, id);
        assert!(!reloaded.settings.input_enabled);
    }

    fn edge_layouts() -> Vec<(&'static str, Layout)> {
        let b = |l, t, w, h| Bounds::new(l, t, w, h).unwrap();
        vec![
            ("mac", Layout::single(b(0, 0, 1512, 982))),
            (
                "mac with external",
                Layout::new(vec![b(0, 0, 1512, 982), b(1512, -200, 2560, 1440)]).unwrap(),
            ),
            ("pc 1080p", Layout::single(b(0, 0, 1920, 1080))),
            (
                "pc 1440p and 1080p",
                Layout::new(vec![b(0, 0, 2560, 1440), b(2560, 0, 1920, 1080)]).unwrap(),
            ),
            (
                "pc left monitor",
                Layout::new(vec![b(-1920, 0, 1920, 1080), b(0, 0, 1920, 1080)]).unwrap(),
            ),
            ("pc mixed", mixed()),
        ]
    }
    fn non_armed_samples(layout: &Layout) -> Vec<((f64, f64), (f64, f64))> {
        let v = layout.virtual_bounds();
        let mut samples = Vec::new();
        for m in [
            *layout.monitor_for((v.left, v.top)),
            *layout.monitor_for((v.right, v.top)),
            *layout.monitor_for((v.left, v.bottom)),
            *layout.monitor_for((v.right, v.bottom)),
            *layout.monitor_for((0, 0)),
        ] {
            let (l, t, r, b) = (
                f64::from(m.left),
                f64::from(m.top),
                f64::from(m.right) + 0.9,
                f64::from(m.bottom) + 0.9,
            );
            for i in 0..=20 {
                let k = f64::from(i) / 20.0;
                let x = l + (r - l) * k;
                let y = t + (b - t) * k;
                samples.push(((r, y), (5.0, 3.0)));
                samples.push(((r, y), (5.0, -3.0)));
                samples.push(((x, t), (2.0, -6.0)));
                samples.push(((x, t), (0.0, -6.0)));
                samples.push(((x, b), (2.0, 6.0)));
                samples.push(((x, b), (0.0, 6.0)));
            }
            for (corner, vertical) in [((l, t), -1.0), ((r, t), -1.0), ((l, b), 1.0), ((r, b), 1.0)] {
                for d in [(0.0, 30.0 * vertical), (8.0, 8.0 * vertical), (3.0, 30.0 * vertical)] {
                    for _ in 0..8 {
                        samples.push((corner, d));
                    }
                }
            }
        }
        samples
    }

    #[test]
    fn only_the_armed_edge_switches_and_other_edges_and_corners_pass_through() {
        for (name, layout) in edge_layouts() {
            for delay in [0, 250] {
                let mut logic = CaptureLogic::default();
                logic.set_arm("peer", config(FlowEdge::Left, FlowMode::Edge, delay, true));
                let t0 = Instant::now();
                for (i, (location, delta)) in non_armed_samples(&layout).into_iter().enumerate() {
                    let now = t0 + Duration::from_millis(i as u64 * 50);
                    let cursor = crate::screen::pointer_location(&layout, location);
                    let mac = crate::screen::pointer_sample(&layout, location, delta);
                    let raw = (
                        location.0.floor() as i32 + delta.0 as i32,
                        location.1.floor() as i32 + delta.1 as i32,
                    );
                    for (x, y) in [mac, raw] {
                        let out = logic.handle(RawEvent::Move { x, y, from: Some(cursor) }, false, false, &layout, now);
                        assert_eq!(out, (Verdict::Pass, None), "{name} delay {delay} at {x},{y}");
                    }
                }
                let v = layout.virtual_bounds();
                let outer = *layout.monitor_for((v.left, v.top + (v.bottom - v.top) / 2));
                let y = outer.center().1;
                let hits = (0..10u64)
                    .filter_map(|i| {
                        let now = t0 + Duration::from_secs(60) + Duration::from_millis(i * 60);
                        logic
                            .handle(
                                RawEvent::Move {
                                    x: outer.left - 3,
                                    y,
                                    from: Some((outer.left, y)),
                                },
                                false,
                                false,
                                &layout,
                                now,
                            )
                            .1
                    })
                    .count();
                assert_eq!(hits, 1, "{name} delay {delay}: the armed edge switches once");
            }
        }
    }

    #[test]
    fn receiver_returns_only_at_the_return_edge_and_moves_freely_elsewhere() {
        for (name, layout) in edge_layouts() {
            let mut watch = ReturnWatch::new(FlowEdge::Left);
            assert_eq!(watch.edge, FlowEdge::Right);
            let v = layout.virtual_bounds();
            let start = layout.monitor_for((0, 0)).center();
            let mut p = start;
            for (dx, dy) in [
                (-4000, 0),
                (0, -4000),
                (0, 37),
                (0, 4000),
                (0, -51),
                (-3, 9),
                (-7, -7),
            ] {
                for _ in 0..6 {
                    let (next, crossed) = watch.step_layout(p, dx, dy, &layout);
                    assert_eq!(crossed, None, "{name}: {p:?} by {dx},{dy}");
                    assert_eq!(layout.clamp(next), next, "{name}: stays on a monitor");
                    p = next;
                }
            }
            let mut sweep = |p: &mut (i32, i32), dx: i32, dy: i32, steps: usize| {
                for _ in 0..steps {
                    let (next, crossed) = watch.step_layout(*p, dx, dy, &layout);
                    assert_eq!(crossed, None, "{name}: {p:?} by {dx},{dy}");
                    *p = next;
                }
            };
            let left = *layout.monitor_for((v.left, v.top + (v.bottom - v.top) / 2));
            let mut corner = (left.left, left.top);
            sweep(&mut corner, -9, -9, 20);
            assert_eq!(corner, (left.left, left.top), "{name}: rests in the corner");
            sweep(&mut corner, 0, 25, 4);
            assert_eq!(corner, (left.left, left.top + 100), "{name}: slides along the edge");
            sweep(&mut corner, -9, 0, 4);
            assert_eq!(corner.0, left.left, "{name}: the left edge only stops the pointer");
            sweep(&mut corner, 30, 0, 3);
            assert_eq!(corner.0, left.left + 90, "{name}: leaves the edge freely");
            let mut bottom = (left.left + 200, left.bottom);
            sweep(&mut bottom, -2, 9, 10);
            assert_eq!(bottom, (left.left + 180, left.bottom), "{name}: slides along the bottom");
            sweep(&mut bottom, 0, -40, 2);
            assert_eq!(bottom.1, left.bottom - 80, "{name}: leaves the bottom freely");
            let mut right = start;
            let mut returned = None;
            for _ in 0..4000 {
                let (next, crossed) = watch.step_layout(right, 3, 1, &layout);
                right = next;
                if crossed.is_some() {
                    returned = crossed;
                    break;
                }
            }
            assert!(returned.is_some(), "{name}: the return edge returns");
            assert_eq!(layout.clamp(right), right);
        }
    }

    #[test]
    fn pushing_into_a_corner_along_another_edge_never_switches() {
        let layout = Layout::single(Bounds::new(0, 0, 1512, 982).unwrap());
        for delay in [0, 250] {
            let mut logic = CaptureLogic::default();
            logic.set_arm("peer", config(FlowEdge::Left, FlowMode::Edge, delay, true));
            let t0 = Instant::now();
            for i in 0..20u64 {
                let now = t0 + Duration::from_millis(i * 40);
                for (location, delta) in [((0.0, 0.0), (0.0, -6.0)), ((0.0, 981.5), (0.0, 6.0))] {
                    let (x, y) = crate::screen::pointer_sample(&layout, location, delta);
                    let from = Some(crate::screen::pointer_location(&layout, location));
                    let out = logic.handle(RawEvent::Move { x, y, from }, false, false, &layout, now);
                    assert_eq!(out, (Verdict::Pass, None), "delay {delay} at {x},{y}");
                }
            }
            let hits = (0..10u64)
                .filter_map(|i| {
                    let now = t0 + Duration::from_secs(5) + Duration::from_millis(i * 60);
                    let (x, y) = crate::screen::pointer_sample(&layout, (0.0, 0.0), (-4.0, -4.0));
                    let from = Some((0, 0));
                    logic
                        .handle(RawEvent::Move { x, y, from }, false, false, &layout, now)
                        .1
                })
                .count();
            assert_eq!(hits, 1, "delay {delay}: pushing out past the armed edge still switches");
        }
    }

    #[test]
    fn a_neighbouring_monitor_never_lends_its_edge_to_the_current_one() {
        let b = |l, t, w, h| Bounds::new(l, t, w, h).unwrap();
        let layout = Layout::new(vec![b(0, 0, 1512, 982), b(1512, -200, 2560, 1440)]).unwrap();
        let mut logic = CaptureLogic::default();
        logic.set_arm("peer", config(FlowEdge::Left, FlowMode::Edge, 0, true));
        let now = Instant::now();
        for _ in 0..10 {
            let (x, y) = crate::screen::pointer_sample(&layout, (1511.7, 0.0), (0.0, -3.0));
            assert_eq!((x, y), (1511, -3));
            let out = logic.handle(
                RawEvent::Move {
                    x,
                    y,
                    from: Some((1511, 0)),
                },
                false,
                false,
                &layout,
                now,
            );
            assert_eq!(out, (Verdict::Pass, None));
        }

        let mut watch = ReturnWatch::new(FlowEdge::Right);
        assert_eq!(watch.edge, FlowEdge::Left);
        let mut p = (1511, 0);
        for _ in 0..10 {
            let (next, crossed) = watch.step_layout(p, 0, -3, &layout);
            assert_eq!((next, crossed), ((1511, 0), None));
            p = next;
        }
        let (next, crossed) = watch.step_layout(p, 0, 40, &layout);
        assert_eq!((next, crossed), ((1511, 40), None));
        let mut p = (0, 400);
        let mut crossed = None;
        for _ in 0..3 {
            let step = watch.step_layout(p, -4, 0, &layout);
            p = step.0;
            crossed = crossed.or(step.1);
        }
        assert!(crossed.is_some(), "the real return edge still returns");
    }
}
