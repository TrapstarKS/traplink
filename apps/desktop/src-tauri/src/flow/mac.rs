use super::{CaptureLogic, CaptureStatus, Captured, FlowEdge, RawEvent, Verdict, EDGE_INSET_PX};
use crate::{
    keymap,
    macos::{self as ffi, CFMachPortRef, CFRunLoopSourceRef, CFTypeRef, CGEventRef},
    screen::{self, Layout},
};
use std::{
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, AtomicU64, AtomicU8, Ordering},
        Mutex, OnceLock,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedSender;

pub const REMOVED_DETAIL: &str = "macOS disabled the input event tap.";
const PERMISSION_HINT: &str = "System Settings → Privacy & Security";

struct HookCtx {
    logic: CaptureLogic,
    tx: UnboundedSender<Captured>,
    layout: Layout,
    anchor: (i32, i32),
    residual: (f64, f64),
    keyboard: Option<(i64, bool)>,
    cursor_hidden: bool,
    cursor_detached: bool,
    motion_since: u64,
}
static CTX: Mutex<Option<HookCtx>> = Mutex::new(None);
static FORWARDING: AtomicBool = AtomicBool::new(false);
static HEARTBEAT: AtomicU64 = AtomicU64::new(0);
static EPOCH: OnceLock<Instant> = OnceLock::new();
static PENDING_DX: AtomicI32 = AtomicI32::new(0);
static PENDING_DY: AtomicI32 = AtomicI32::new(0);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);
static REINSTALLS: AtomicU32 = AtomicU32::new(0);
static LAST_REINSTALL_MS: AtomicU64 = AtomicU64::new(0);
static REASON: AtomicU8 = AtomicU8::new(0);
static STOP: AtomicBool = AtomicBool::new(false);
static REPAIR: AtomicBool = AtomicBool::new(false);
static TAP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static RUN_LOOP: Mutex<usize> = Mutex::new(0);
static THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
const REASON_REMOVED: u8 = 1;
const REASON_REPAIR: u8 = 2;
const STALE: Duration = Duration::from_secs(2);
const TICK_SECONDS: f64 = 0.5;

const LEFT_DOWN: u32 = 1;
const LEFT_UP: u32 = 2;
const RIGHT_DOWN: u32 = 3;
const RIGHT_UP: u32 = 4;
const MOUSE_MOVED: u32 = 5;
const LEFT_DRAGGED: u32 = 6;
const RIGHT_DRAGGED: u32 = 7;
const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const FLAGS_CHANGED: u32 = 12;
const SCROLL_WHEEL: u32 = 22;
const OTHER_DOWN: u32 = 25;
const OTHER_UP: u32 = 26;
const OTHER_DRAGGED: u32 = 27;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;
const BUTTON_NUMBER: u32 = 3;
const DELTA_X: u32 = 4;
const DELTA_Y: u32 = 5;
const KEYCODE: u32 = 9;
const KEYBOARD_TYPE: u32 = 10;
const SCROLL_LINES_1: u32 = 11;
const SCROLL_LINES_2: u32 = 12;
const SOURCE_PID: u32 = 41;
const SOURCE_USER_DATA: u32 = 42;
const SCROLL_CONTINUOUS: u32 = 88;
const SCROLL_POINTS_1: u32 = 96;
const SCROLL_POINTS_2: u32 = 97;
const HID_TAP: u32 = 0;
const SESSION_TAP: u32 = 1;
const HEAD_INSERT: u32 = 0;
const TAP_DEFAULT: u32 = 0;
const EVENT_MASK: u64 = (1 << LEFT_DOWN)
    | (1 << LEFT_UP)
    | (1 << RIGHT_DOWN)
    | (1 << RIGHT_UP)
    | (1 << MOUSE_MOVED)
    | (1 << LEFT_DRAGGED)
    | (1 << RIGHT_DRAGGED)
    | (1 << KEY_DOWN)
    | (1 << KEY_UP)
    | (1 << FLAGS_CHANGED)
    | (1 << SCROLL_WHEEL)
    | (1 << OTHER_DOWN)
    | (1 << OTHER_UP)
    | (1 << OTHER_DRAGGED);
const UNMAPPED_KEY: u32 = 0x1000;

fn now_ms() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64
}
pub fn heartbeat() {
    HEARTBEAT.store(now_ms(), Ordering::Relaxed);
}
pub fn set_forwarding(value: bool) {
    let mut guard = CTX.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(ctx) = guard.as_mut() {
        if value {
            FORWARDING.store(true, Ordering::SeqCst);
        } else {
            release_pointer(ctx);
        }
    }
}
fn release_pointer(ctx: &mut HookCtx) {
    FORWARDING.store(false, Ordering::SeqCst);
    take_moves();
    ctx.residual = (0.0, 0.0);
    if ctx.cursor_detached && ffi::associate(true) {
        ctx.cursor_detached = false;
    }
    if ctx.cursor_hidden && ffi::hide_cursor(false) {
        ctx.cursor_hidden = false;
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
fn modifiers_down() -> impl Iterator<Item = u32> {
    keymap::mac_held_modifiers(ffi::modifier_flags()).filter_map(|kvk| {
        keymap::scan_for_mac(kvk, false).map(|(scan, extended)| super::key_id(scan, extended))
    })
}
pub fn begin_forwarding(session: uuid::Uuid, peer: &str, at: Instant) -> Result<bool, String> {
    let mut guard = CTX.lock().map_err(|_| TAP_FAILED)?;
    let ctx = guard.as_mut().ok_or(TAP_FAILED)?;
    if !INSTALLED.load(Ordering::SeqCst) || STOP.load(Ordering::SeqCst) {
        return Err(TAP_FAILED.into());
    }
    let cursor = screen::mac::cursor().map(|point| screen::pointer_location(&ctx.layout, point));
    if !ctx.logic.still_at_crossing(peer, at, &ctx.layout, cursor) {
        return Ok(false);
    }
    take_moves();
    let anchor = ctx.layout.monitor_for((0, 0)).center();
    if !ffi::warp(anchor) {
        return Err("macOS could not reposition the local pointer for Flow.".into());
    }
    ctx.logic.start_forwarding(anchor);
    ctx.logic.session = Some(session);
    ctx.logic.adopt_local_keys(modifiers_down());
    ctx.anchor = anchor;
    ctx.residual = (0.0, 0.0);
    heartbeat();
    if !ctx.cursor_detached {
        ctx.cursor_detached = ffi::associate(false);
    }
    if !ctx.cursor_hidden {
        ctx.cursor_hidden = ffi::hide_cursor(true);
    }
    ctx.motion_since = ffi::current_event_timestamp();
    FORWARDING.store(true, Ordering::SeqCst);
    Ok(true)
}
pub fn place_cursor(peer: &str, edge: FlowEdge, entry: f64) {
    if let Ok(mut guard) = CTX.lock() {
        if let Some(ctx) = guard.as_mut() {
            let point = ctx.layout.entry_point(edge, entry, EDGE_INSET_PX);
            if screen::mac::set_cursor(point) {
                ctx.motion_since = ffi::current_event_timestamp();
                ctx.logic.rearm_after_return(peer, point);
            }
        }
    }
}
pub fn reinstall() {
    REPAIR.store(true, Ordering::SeqCst);
    wake();
}
fn wake() {
    if let Ok(run_loop) = RUN_LOOP.lock() {
        if *run_loop != 0 {
            unsafe { ffi::CFRunLoopStop(*run_loop as ffi::CFRunLoopRef) };
        }
    }
}
pub fn capture_status() -> Option<CaptureStatus> {
    if !RUNNING.load(Ordering::SeqCst) {
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

fn release_stale(ctx: &mut HookCtx) -> bool {
    let forwarding = FORWARDING.load(Ordering::SeqCst);
    if forwarding
        && now_ms().saturating_sub(HEARTBEAT.load(Ordering::Relaxed)) > STALE.as_millis() as u64
    {
        release_pointer(ctx);
        let _ = ctx.tx.send(Captured::CaptureStale {
            session: ctx.logic.session,
        });
        return false;
    }
    forwarding
}

fn handle(build: impl FnOnce(&mut HookCtx, bool) -> Result<RawEvent, bool>) -> bool {
    let Ok(mut guard) = CTX.lock() else {
        return false;
    };
    let Some(ctx) = guard.as_mut() else {
        return false;
    };
    // Read the forwarding state only after the capture state lock. A return
    // may have completed while this callback waited for the lock.
    let forwarding = release_stale(ctx);
    let event = match build(ctx, forwarding) {
        Ok(event) => event,
        Err(swallow) => return swallow,
    };
    let HookCtx {
        logic, tx, layout, ..
    } = ctx;
    let (verdict, output) = logic.handle(event, false, forwarding, layout, Instant::now());
    match output {
        Some(Captured::Move { dx, dy }) => {
            PENDING_DX.fetch_add(dx, Ordering::Relaxed);
            PENDING_DY.fetch_add(dy, Ordering::Relaxed);
        }
        Some(Captured::Emergency) => {
            let _ = tx.send(Captured::Emergency);
            release_pointer(ctx);
        }
        Some(Captured::Event(event)) => {
            if let Some(session) = logic.session {
                let _ = tx.send(Captured::Forwarded { session, event });
            }
        }
        Some(other) => {
            let _ = tx.send(other);
        }
        None => {}
    }
    verdict == Verdict::Swallow
}

fn key_event(ctx: &mut HookCtx, kvk: u16, keyboard_type: i64, down: bool) -> RawEvent {
    let iso = match ctx.keyboard {
        Some((t, iso)) if t == keyboard_type => iso,
        _ => {
            let iso = ffi::iso_keyboard(Some(keyboard_type));
            ctx.keyboard = Some((keyboard_type, iso));
            iso
        }
    };
    let (scan, extended) =
        keymap::scan_for_mac(kvk, iso).unwrap_or((UNMAPPED_KEY + u32::from(kvk), false));
    RawEvent::Key {
        scan,
        extended,
        down,
    }
}

unsafe fn process(event_type: u32, event: CGEventRef) -> bool {
    let pid = ffi::CGEventGetIntegerValueField(event, SOURCE_PID);
    let injected = pid == i64::from(std::process::id())
        || ffi::CGEventGetIntegerValueField(event, SOURCE_USER_DATA) == ffi::INJECTED_MARKER;
    if injected {
        // Do not let our posted mouse/key events alter capture residuals,
        // keyboard state or edge pressure, including late events at handoff.
        if !matches!(
            event_type,
            MOUSE_MOVED | LEFT_DRAGGED | RIGHT_DRAGGED | OTHER_DRAGGED
        ) {
            return false;
        }
        let timestamp = ffi::CGEventGetTimestamp(event);
        return CTX.lock().is_ok_and(|guard| {
            guard.as_ref().is_some_and(|ctx| {
                FORWARDING.load(Ordering::SeqCst)
                    || (timestamp != 0 && timestamp < ctx.motion_since)
            })
        });
    }
    match event_type {
        MOUSE_MOVED | LEFT_DRAGGED | RIGHT_DRAGGED | OTHER_DRAGGED => {
            let delta = (
                ffi::CGEventGetDoubleValueField(event, DELTA_X),
                ffi::CGEventGetDoubleValueField(event, DELTA_Y),
            );
            let location = ffi::CGEventGetLocation(event);
            let timestamp = ffi::CGEventGetTimestamp(event);
            handle(|ctx, forwarding| {
                // A queued pre-warp motion must not move the cursor back to
                // its previous corner or trigger the newly rearmed edge.
                if timestamp != 0 && timestamp < ctx.motion_since {
                    return Err(true);
                }
                let (x, y) = if forwarding {
                    let (dx, dy) = screen::accumulate_delta(&mut ctx.residual, delta);
                    // Disassociation only applies in the foreground. If the
                    // cursor drifts, park it away from display edges. Avoid a
                    // WindowServer warp for every physical mouse report.
                    if (location.x - f64::from(ctx.anchor.0)).abs() >= 32.0
                        || (location.y - f64::from(ctx.anchor.1)).abs() >= 32.0
                    {
                        ffi::warp(ctx.anchor);
                    }
                    (ctx.anchor.0 + dx, ctx.anchor.1 + dy)
                } else {
                    screen::pointer_sample(&ctx.layout, (location.x, location.y), delta)
                };
                let from = screen::pointer_location(&ctx.layout, (location.x, location.y));
                Ok(RawEvent::Move {
                    x,
                    y,
                    from: Some(from),
                })
            })
        }
        LEFT_DOWN | LEFT_UP | RIGHT_DOWN | RIGHT_UP | OTHER_DOWN | OTHER_UP => {
            let index = match event_type {
                LEFT_DOWN | LEFT_UP => 0,
                RIGHT_DOWN | RIGHT_UP => 2,
                _ => match ffi::CGEventGetIntegerValueField(event, BUTTON_NUMBER) {
                    2 => 1,
                    _ => 3,
                },
            };
            let down = matches!(event_type, LEFT_DOWN | RIGHT_DOWN | OTHER_DOWN);
            handle(|_, _| Ok(RawEvent::Button { index, down }))
        }
        KEY_DOWN | KEY_UP => {
            let kvk = ffi::CGEventGetIntegerValueField(event, KEYCODE) as u16;
            let keyboard_type = ffi::CGEventGetIntegerValueField(event, KEYBOARD_TYPE);
            let down = event_type == KEY_DOWN;
            handle(|ctx, _| Ok(key_event(ctx, kvk, keyboard_type, down)))
        }
        FLAGS_CHANGED => {
            let kvk = ffi::CGEventGetIntegerValueField(event, KEYCODE) as u16;
            let keyboard_type = ffi::CGEventGetIntegerValueField(event, KEYBOARD_TYPE);
            if kvk == keymap::MAC_CAPS_LOCK {
                let press = handle(|ctx, _| Ok(key_event(ctx, kvk, keyboard_type, true)));
                let release = handle(|ctx, _| Ok(key_event(ctx, kvk, keyboard_type, false)));
                return press || release;
            }
            let flags = ffi::CGEventGetFlags(event);
            match keymap::mac_modifier_down(kvk, flags) {
                Some(down) => handle(|ctx, _| {
                    Ok(match key_event(ctx, kvk, keyboard_type, down) {
                        RawEvent::Key {
                            scan,
                            extended,
                            down: true,
                        } if ctx.logic.key_held(scan, extended) => RawEvent::Key {
                            scan,
                            extended,
                            down: false,
                        },
                        other => other,
                    })
                }),
                None => FORWARDING.load(Ordering::SeqCst),
            }
        }
        SCROLL_WHEEL => {
            let lines = (
                ffi::CGEventGetIntegerValueField(event, SCROLL_LINES_1),
                ffi::CGEventGetIntegerValueField(event, SCROLL_LINES_2),
            );
            let points = (
                ffi::CGEventGetDoubleValueField(event, SCROLL_POINTS_1),
                ffi::CGEventGetDoubleValueField(event, SCROLL_POINTS_2),
            );
            let continuous = ffi::CGEventGetIntegerValueField(event, SCROLL_CONTINUOUS) != 0;
            let (vertical, horizontal) = keymap::mac_scroll(lines, points, continuous);
            let mut swallow = FORWARDING.load(Ordering::SeqCst);
            for (delta, horizontal) in [(vertical, false), (horizontal, true)] {
                if delta != 0 {
                    swallow |= handle(|_, _| Ok(RawEvent::Wheel { delta, horizontal }));
                }
            }
            swallow
        }
        _ => false,
    }
}

unsafe extern "C" fn tap_callback(
    _proxy: *mut c_void,
    event_type: u32,
    event: CGEventRef,
    _info: *mut c_void,
) -> CGEventRef {
    if event_type == TAP_DISABLED_BY_TIMEOUT || event_type == TAP_DISABLED_BY_USER_INPUT {
        let tap = TAP.load(Ordering::SeqCst);
        if !tap.is_null() {
            INSTALLED.store(false, Ordering::SeqCst);
            ffi::CGEventTapEnable(tap, true);
            if ffi::CGEventTapIsEnabled(tap) {
                recovered(REASON_REMOVED);
            } else {
                lost();
            }
        }
        return event;
    }
    if event.is_null() {
        return event;
    }
    let swallow =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| process(event_type, event)))
            .unwrap_or(false);
    if swallow {
        std::ptr::null_mut()
    } else {
        event
    }
}

struct Tap {
    port: CFMachPortRef,
    source: CFRunLoopSourceRef,
}
unsafe fn install(run_loop: ffi::CFRunLoopRef) -> Option<Tap> {
    for location in [HID_TAP, SESSION_TAP] {
        let port = ffi::CGEventTapCreate(
            location,
            HEAD_INSERT,
            TAP_DEFAULT,
            EVENT_MASK,
            tap_callback,
            std::ptr::null_mut(),
        );
        if port.is_null() {
            continue;
        }
        let source = ffi::CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0);
        if source.is_null() {
            ffi::CFMachPortInvalidate(port);
            ffi::CFRelease(port as CFTypeRef);
            continue;
        }
        ffi::CFRunLoopAddSource(run_loop, source, ffi::kCFRunLoopCommonModes);
        ffi::CGEventTapEnable(port, true);
        if !ffi::CGEventTapIsEnabled(port) {
            uninstall(run_loop, Tap { port, source });
            continue;
        }
        TAP.store(port, Ordering::SeqCst);
        return Some(Tap { port, source });
    }
    None
}
unsafe fn uninstall(run_loop: ffi::CFRunLoopRef, tap: Tap) {
    TAP.store(std::ptr::null_mut(), Ordering::SeqCst);
    ffi::CGEventTapEnable(tap.port, false);
    ffi::CFRunLoopRemoveSource(run_loop, tap.source, ffi::kCFRunLoopCommonModes);
    ffi::CFMachPortInvalidate(tap.port);
    ffi::CFRelease(tap.source as CFTypeRef);
    ffi::CFRelease(tap.port as CFTypeRef);
}
fn recovered(reason: u8) {
    REINSTALLS.fetch_add(1, Ordering::Relaxed);
    LAST_REINSTALL_MS.store(now_ms().max(1), Ordering::Relaxed);
    REASON.store(reason, Ordering::Relaxed);
    if let Ok(mut guard) = CTX.lock() {
        if let Some(ctx) = guard.as_mut() {
            let removed = reason == REASON_REMOVED;
            if removed {
                release_pointer(ctx);
            }
            INSTALLED.store(true, Ordering::SeqCst);
            let _ = ctx.tx.send(Captured::HookReinstalled {
                removed,
                session: ctx.logic.session,
            });
        }
    }
}
fn lost() {
    INSTALLED.store(false, Ordering::SeqCst);
    if let Ok(mut guard) = CTX.lock() {
        if let Some(ctx) = guard.as_mut() {
            release_pointer(ctx);
            let _ = ctx.tx.send(Captured::HookFailed {
                session: ctx.logic.session,
            });
        }
    }
}
fn refresh_layout() {
    let layout = screen::mac::layout();
    if let Ok(mut guard) = CTX.lock() {
        if let Some(ctx) = guard.as_mut() {
            if !release_stale(ctx) && (ctx.cursor_detached || ctx.cursor_hidden) {
                release_pointer(ctx);
            }
            if let Some(layout) = layout {
                ctx.layout = layout;
            }
        }
    }
}

const TAP_FAILED: &str = "Could not start macOS mouse and keyboard capture.";

fn permissions() -> Result<(), String> {
    let accessibility = ffi::accessibility_trusted(false) || ffi::accessibility_trusted(true);
    let monitoring = ffi::input_monitoring(true);
    let missing = match (accessibility, monitoring) {
        (true, true) => return Ok(()),
        (false, true) => "Accessibility",
        (true, false) => "Input Monitoring",
        (false, false) => "Accessibility and Input Monitoring",
    };
    Err(format!(
        "Flow needs permission to control the mouse and keyboard. Turn on TrapLink in \
         {PERMISSION_HINT} → {missing}, then quit and reopen TrapLink."
    ))
}

fn spawn() -> Result<JoinHandle<()>, String> {
    let (ready, wait) = std::sync::mpsc::channel::<Result<(), String>>();
    STOP.store(false, Ordering::SeqCst);
    REPAIR.store(false, Ordering::SeqCst);
    let handle = std::thread::Builder::new()
        .name("traplink-flow-tap".into())
        .spawn(move || unsafe {
            let run_loop = ffi::CFRunLoopGetCurrent();
            let Some(tap) = install(run_loop) else {
                let _ = ready.send(Err(format!(
                    "{TAP_FAILED} Make sure TrapLink is turned on in {PERMISSION_HINT} → \
                     Accessibility and Input Monitoring, then quit and reopen the app."
                )));
                return;
            };
            let mut tap = Some(tap);
            if let Ok(mut stored) = RUN_LOOP.lock() {
                ffi::CFRetain(run_loop as CFTypeRef);
                *stored = run_loop as usize;
            }
            INSTALLED.store(true, Ordering::SeqCst);
            RUNNING.store(true, Ordering::SeqCst);
            let _ = ready.send(Ok(()));
            while !STOP.load(Ordering::SeqCst) {
                if tap.is_some() {
                    ffi::CFRunLoopRunInMode(ffi::kCFRunLoopDefaultMode, TICK_SECONDS, 0);
                } else {
                    std::thread::sleep(Duration::from_secs_f64(TICK_SECONDS));
                }
                if STOP.load(Ordering::SeqCst) {
                    break;
                }
                refresh_layout();
                if REPAIR.swap(false, Ordering::SeqCst) {
                    INSTALLED.store(false, Ordering::SeqCst);
                    if let Some(old) = tap.take() {
                        uninstall(run_loop, old);
                    }
                    tap = install(run_loop);
                    if tap.is_some() {
                        recovered(REASON_REPAIR);
                    } else {
                        lost();
                    }
                } else if let Some(current) = tap.as_ref() {
                    if !ffi::CGEventTapIsEnabled(current.port) {
                        let was_installed = INSTALLED.swap(false, Ordering::SeqCst);
                        ffi::CGEventTapEnable(current.port, true);
                        if ffi::CGEventTapIsEnabled(current.port) {
                            recovered(REASON_REMOVED);
                        } else if was_installed {
                            lost();
                        }
                    }
                }
            }
            set_forwarding(false);
            if let Some(tap) = tap.take() {
                uninstall(run_loop, tap);
            }
            // The worker owns the retained run loop, including a setup that
            // finishes after its caller has already timed out.
            let mut stored = RUN_LOOP.lock().unwrap_or_else(|p| p.into_inner());
            if *stored != 0 {
                ffi::CFRelease(*stored as CFTypeRef);
                *stored = 0;
            }
            INSTALLED.store(false, Ordering::SeqCst);
            RUNNING.store(false, Ordering::SeqCst);
        })
        .map_err(|_| "Could not start mouse and keyboard capture.".to_string())?;
    match wait
        .recv_timeout(Duration::from_secs(3))
        .map_err(|_| "Mouse and keyboard capture did not respond.".to_string())
        .and_then(|r| r)
    {
        Ok(()) => Ok(handle),
        Err(error) => {
            STOP.store(true, Ordering::SeqCst);
            wake();
            // Retain ownership even if installation is delayed in
            // WindowServer. A retry must join this tap before reusing CTX.
            *THREAD.lock().unwrap_or_else(|p| p.into_inner()) = Some(handle);
            Err(error)
        }
    }
}
fn stop_thread() {
    let handle = THREAD.lock().ok().and_then(|mut thread| thread.take());
    if let Some(handle) = handle {
        STOP.store(true, Ordering::SeqCst);
        wake();
        let _ = handle.join();
    }
}

pub struct HookThread;
impl HookThread {
    pub fn start(tx: UnboundedSender<Captured>) -> Result<Self, String> {
        permissions()?;
        let layout = screen::mac::layout()
            .ok_or_else(|| "Could not read the display layout.".to_string())?;
        stop_thread();
        *CTX.lock().map_err(|_| "Flow lock unavailable.")? = Some(HookCtx {
            logic: CaptureLogic {
                swap_command_control: true,
                ..CaptureLogic::default()
            },
            tx,
            layout,
            anchor: (0, 0),
            residual: (0.0, 0.0),
            keyboard: None,
            cursor_hidden: false,
            cursor_detached: false,
            motion_since: 0,
        });
        set_forwarding(false);
        take_moves();
        REINSTALLS.store(0, Ordering::Relaxed);
        LAST_REINSTALL_MS.store(0, Ordering::Relaxed);
        REASON.store(0, Ordering::Relaxed);
        match spawn() {
            Ok(handle) => {
                *THREAD.lock().unwrap_or_else(|p| p.into_inner()) = Some(handle);
                Ok(Self)
            }
            Err(error) => {
                if let Ok(mut ctx) = CTX.lock() {
                    *ctx = None;
                }
                Err(error)
            }
        }
    }
    pub fn stop(&self) {
        set_forwarding(false);
        stop_thread();
        if let Ok(mut ctx) = CTX.lock() {
            *ctx = None;
        }
    }
}
