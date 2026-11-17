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
}
static CTX: Mutex<Option<HookCtx>> = Mutex::new(None);
static FORWARDING: AtomicBool = AtomicBool::new(false);
static HIDDEN: AtomicBool = AtomicBool::new(false);
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
const ENIGO_MARKER: i64 = 100;
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
    let was = FORWARDING.swap(value, Ordering::SeqCst);
    if was && !value {
        release_pointer();
    }
}
fn release_pointer() {
    ffi::associate(true);
    if HIDDEN.swap(false, Ordering::SeqCst) {
        ffi::hide_cursor(false);
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
fn modifiers_down() -> impl Iterator<Item = u32> {
    keymap::mac_held_modifiers(ffi::modifier_flags()).filter_map(|kvk| {
        keymap::scan_for_mac(kvk, false).map(|(scan, extended)| super::key_id(scan, extended))
    })
}
pub fn begin_forwarding() {
    take_moves();
    let anchor = CTX.lock().ok().and_then(|mut guard| {
        guard.as_mut().map(|ctx| {
            let anchor = ctx.layout.monitor_for((0, 0)).center();
            ctx.logic.start_forwarding(anchor);
            ctx.logic.adopt_local_keys(modifiers_down());
            ctx.anchor = anchor;
            ctx.residual = (0.0, 0.0);
            anchor
        })
    });
    let Some(anchor) = anchor else {
        return;
    };
    heartbeat();
    ffi::warp(anchor, false);
    ffi::associate(false);
    if !HIDDEN.swap(true, Ordering::SeqCst) {
        ffi::hide_cursor(true);
    }
    set_forwarding(true);
}
pub fn place_cursor(edge: FlowEdge, entry: f64) {
    let point = CTX.lock().ok().and_then(|guard| {
        guard
            .as_ref()
            .map(|ctx| ctx.layout.entry_point(edge, entry, EDGE_INSET_PX))
    });
    if let Some(point) = point {
        screen::mac::set_cursor(point);
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

fn handle(
    injected: bool,
    build: impl FnOnce(&mut HookCtx, bool) -> Option<RawEvent>,
) -> bool {
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
    let Some(ctx) = guard.as_mut() else {
        return false;
    };
    let Some(event) = build(ctx, forwarding) else {
        return forwarding;
    };
    let HookCtx {
        logic, tx, layout, ..
    } = ctx;
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

fn take_whole(residual: &mut (f64, f64), (dx, dy): (f64, f64)) -> (i32, i32) {
    let limit = f64::from(super::MAX_DELTA);
    let add = |acc: &mut f64, d: f64| {
        if d.is_finite() {
            *acc = (*acc + d).clamp(-limit, limit);
        }
        let whole = acc.trunc();
        *acc -= whole;
        whole as i32
    };
    (add(&mut residual.0, dx), add(&mut residual.1, dy))
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
        || ffi::CGEventGetIntegerValueField(event, SOURCE_USER_DATA) == ENIGO_MARKER;
    match event_type {
        MOUSE_MOVED | LEFT_DRAGGED | RIGHT_DRAGGED | OTHER_DRAGGED => {
            let delta = (
                ffi::CGEventGetDoubleValueField(event, DELTA_X),
                ffi::CGEventGetDoubleValueField(event, DELTA_Y),
            );
            let location = ffi::CGEventGetLocation(event);
            handle(injected, |ctx, forwarding| {
                let (x, y) = if forwarding {
                    let (dx, dy) = take_whole(&mut ctx.residual, delta);
                    (ctx.anchor.0 + dx, ctx.anchor.1 + dy)
                } else {
                    screen::pointer_sample(&ctx.layout, (location.x, location.y), delta)
                };
                let from = screen::pointer_location(&ctx.layout, (location.x, location.y));
                Some(RawEvent::Move {
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
            handle(injected, |_, _| Some(RawEvent::Button { index, down }))
        }
        KEY_DOWN | KEY_UP => {
            let kvk = ffi::CGEventGetIntegerValueField(event, KEYCODE) as u16;
            let keyboard_type = ffi::CGEventGetIntegerValueField(event, KEYBOARD_TYPE);
            let down = event_type == KEY_DOWN;
            handle(injected, |ctx, _| {
                Some(key_event(ctx, kvk, keyboard_type, down))
            })
        }
        FLAGS_CHANGED => {
            let kvk = ffi::CGEventGetIntegerValueField(event, KEYCODE) as u16;
            let keyboard_type = ffi::CGEventGetIntegerValueField(event, KEYBOARD_TYPE);
            if kvk == keymap::MAC_CAPS_LOCK {
                let press = handle(injected, |ctx, _| {
                    Some(key_event(ctx, kvk, keyboard_type, true))
                });
                let release = handle(injected, |ctx, _| {
                    Some(key_event(ctx, kvk, keyboard_type, false))
                });
                return press || release;
            }
            let flags = ffi::CGEventGetFlags(event);
            match keymap::mac_modifier_down(kvk, flags) {
                Some(down) => handle(injected, |ctx, _| {
                    Some(match key_event(ctx, kvk, keyboard_type, down) {
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
                None => handle(injected, |_, _| None),
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
            let swallow = handle(injected, |_, _| {
                Some(RawEvent::Wheel {
                    delta: vertical,
                    horizontal: false,
                })
            });
            if horizontal != 0 {
                let swallow_h = handle(injected, |_, _| {
                    Some(RawEvent::Wheel {
                        delta: horizontal,
                        horizontal: true,
                    })
                });
                return swallow || swallow_h;
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
            ffi::CGEventTapEnable(tap, true);
            recovered(REASON_REMOVED);
        }
        return event;
    }
    if event.is_null() {
        return event;
    }
    let swallow = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        process(event_type, event)
    }))
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
    INSTALLED.store(true, Ordering::SeqCst);
    REINSTALLS.fetch_add(1, Ordering::Relaxed);
    LAST_REINSTALL_MS.store(now_ms().max(1), Ordering::Relaxed);
    REASON.store(reason, Ordering::Relaxed);
    send(Captured::HookReinstalled {
        removed: reason == REASON_REMOVED,
    });
}
fn lost() {
    INSTALLED.store(false, Ordering::SeqCst);
    set_forwarding(false);
    send(Captured::HookFailed);
}
fn refresh_layout() {
    if let Some(layout) = screen::mac::layout() {
        if let Ok(mut guard) = CTX.lock() {
            if let Some(ctx) = guard.as_mut() {
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
                        ffi::CGEventTapEnable(current.port, true);
                        if ffi::CGEventTapIsEnabled(current.port) {
                            recovered(REASON_REMOVED);
                        } else if INSTALLED.load(Ordering::SeqCst) {
                            lost();
                        }
                    }
                }
            }
            set_forwarding(false);
            if let Some(tap) = tap.take() {
                uninstall(run_loop, tap);
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
            if handle.is_finished() {
                let _ = handle.join();
            }
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
    if let Ok(mut run_loop) = RUN_LOOP.lock() {
        if *run_loop != 0 {
            unsafe { ffi::CFRelease(*run_loop as CFTypeRef) };
            *run_loop = 0;
        }
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
        });
        set_forwarding(false);
        take_moves();
        REINSTALLS.store(0, Ordering::Relaxed);
        LAST_REINSTALL_MS.store(0, Ordering::Relaxed);
        REASON.store(0, Ordering::Relaxed);
        match spawn() {
            Ok(handle) => {
                if let Ok(mut thread) = THREAD.lock() {
                    *thread = Some(handle);
                }
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

#[cfg(test)]
mod tests {
    use super::take_whole;

    #[test]
    fn fractional_deltas_accumulate() {
        let mut residual = (0.0, 0.0);
        assert_eq!(take_whole(&mut residual, (0.4, -0.6)), (0, 0));
        assert_eq!(take_whole(&mut residual, (0.4, -0.6)), (0, -1));
        assert_eq!(take_whole(&mut residual, (0.4, 2.0)), (1, 1));
        assert!((residual.0 - 0.2).abs() < 1e-9 && (residual.1 - 0.8).abs() < 1e-9);
        assert_eq!(take_whole(&mut residual, (f64::NAN, 0.0)), (0, 0));
        assert_eq!(take_whole(&mut residual, (1e9, 0.0)).0, super::super::MAX_DELTA);
    }
}
