#![allow(non_upper_case_globals, non_snake_case, dead_code)]
use std::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

pub type CFTypeRef = *const c_void;
pub type CFStringRef = *const c_void;
pub type CGEventRef = *mut c_void;
pub type CFMachPortRef = *mut c_void;
pub type CFRunLoopRef = *mut c_void;
pub type CFRunLoopSourceRef = *mut c_void;
pub type TapCallback = unsafe extern "C" fn(
    proxy: *mut c_void,
    event_type: u32,
    event: CGEventRef,
    info: *mut c_void,
) -> CGEventRef;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    pub fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    pub fn CGDisplayBounds(display: u32) -> CGRect;
    pub fn CGWarpMouseCursorPosition(point: CGPoint) -> i32;
    pub fn CGAssociateMouseAndMouseCursorPosition(connected: i32) -> i32;
    pub fn CGDisplayHideCursor(display: u32) -> i32;
    pub fn CGDisplayShowCursor(display: u32) -> i32;
    pub fn CGEventCreate(source: *const c_void) -> CGEventRef;
    pub fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    pub fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    pub fn CGEventGetDoubleValueField(event: CGEventRef, field: u32) -> f64;
    pub fn CGEventGetFlags(event: CGEventRef) -> u64;
    pub fn CGEventSourceFlagsState(state: i32) -> u64;
    pub fn CGEventCreateMouseEvent(
        source: *const c_void,
        mouse_type: u32,
        location: CGPoint,
        button: u32,
    ) -> CGEventRef;
    pub fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);
    pub fn CGEventSetFlags(event: CGEventRef, flags: u64);
    pub fn CGEventPost(tap: u32, event: CGEventRef);
    pub fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: TapCallback,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    pub fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    pub fn CGEventTapIsEnabled(tap: CFMachPortRef) -> bool;
    pub fn CGPreflightListenEventAccess() -> bool;
    pub fn CGRequestListenEventAccess() -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub static kCFRunLoopCommonModes: CFStringRef;
    pub static kCFRunLoopDefaultMode: CFStringRef;
    pub static kCFBooleanTrue: CFTypeRef;
    pub static kCFTypeDictionaryKeyCallBacks: [usize; 6];
    pub static kCFTypeDictionaryValueCallBacks: [usize; 5];
    pub fn CFMachPortCreateRunLoopSource(
        allocator: CFTypeRef,
        port: CFMachPortRef,
        order: isize,
    ) -> CFRunLoopSourceRef;
    pub fn CFMachPortInvalidate(port: CFMachPortRef);
    pub fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    pub fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    pub fn CFRunLoopRemoveSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    pub fn CFRunLoopRunInMode(mode: CFStringRef, seconds: f64, return_after_source: u8) -> i32;
    pub fn CFRunLoopStop(rl: CFRunLoopRef);
    pub fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    pub fn CFRelease(cf: CFTypeRef);
    pub fn CFDictionaryCreate(
        allocator: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFTypeRef;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    pub static kAXTrustedCheckOptionPrompt: CFStringRef;
    pub fn AXIsProcessTrusted() -> u8;
    pub fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> u8;
}

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn KBGetLayoutType(keyboard_type: i16) -> u32;
    fn LMGetKbdType() -> u8;
}

pub fn displays() -> Vec<(f64, f64, f64, f64)> {
    let mut ids = [0u32; 32];
    let mut count = 0u32;
    let error = unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
    if error != 0 {
        return Vec::new();
    }
    ids[..(count as usize).min(ids.len())]
        .iter()
        .map(|id| {
            let r = unsafe { CGDisplayBounds(*id) };
            (r.origin.x, r.origin.y, r.size.width, r.size.height)
        })
        .collect()
}
pub fn modifier_flags() -> u64 {
    unsafe { CGEventSourceFlagsState(1) }
}
pub fn cursor() -> Option<CGPoint> {
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        if event.is_null() {
            return None;
        }
        let point = CGEventGetLocation(event);
        CFRelease(event as CFTypeRef);
        (point.x.is_finite() && point.y.is_finite()).then_some(point)
    }
}
pub fn warp((x, y): (i32, i32), reconnect: bool) {
    unsafe {
        CGWarpMouseCursorPosition(CGPoint {
            x: f64::from(x),
            y: f64::from(y),
        });
        if reconnect {
            CGAssociateMouseAndMouseCursorPosition(1);
        }
    }
}
pub const INJECTED_MARKER: i64 = 100;
pub fn post_mouse_move((x, y): (i32, i32), (dx, dy): (i32, i32), drag: Option<u32>) -> bool {
    const MOUSE_MOVED: u32 = 5;
    const LEFT_DRAGGED: u32 = 6;
    const RIGHT_DRAGGED: u32 = 7;
    const OTHER_DRAGGED: u32 = 27;
    const BUTTON_NUMBER: u32 = 3;
    const DELTA_X: u32 = 4;
    const DELTA_Y: u32 = 5;
    const SOURCE_USER_DATA: u32 = 42;
    const HID_TAP: u32 = 0;
    const COMBINED_SESSION_STATE: i32 = 0;
    let (event_type, button) = match drag {
        Some(0) => (LEFT_DRAGGED, 0),
        Some(1) => (RIGHT_DRAGGED, 1),
        Some(other) => (OTHER_DRAGGED, other),
        None => (MOUSE_MOVED, 0),
    };
    unsafe {
        let event = CGEventCreateMouseEvent(
            std::ptr::null(),
            event_type,
            CGPoint {
                x: f64::from(x),
                y: f64::from(y),
            },
            button,
        );
        if event.is_null() {
            return false;
        }
        if event_type == OTHER_DRAGGED {
            CGEventSetIntegerValueField(event, BUTTON_NUMBER, i64::from(button));
        }
        CGEventSetIntegerValueField(event, DELTA_X, i64::from(dx));
        CGEventSetIntegerValueField(event, DELTA_Y, i64::from(dy));
        CGEventSetIntegerValueField(event, SOURCE_USER_DATA, INJECTED_MARKER);
        CGEventSetFlags(event, CGEventSourceFlagsState(COMBINED_SESSION_STATE));
        CGEventPost(HID_TAP, event);
        CFRelease(event as CFTypeRef);
    }
    true
}
pub fn associate(connected: bool) {
    unsafe { CGAssociateMouseAndMouseCursorPosition(i32::from(connected)) };
}
pub fn hide_cursor(hidden: bool) {
    unsafe {
        if hidden {
            CGDisplayHideCursor(0);
        } else {
            CGDisplayShowCursor(0);
        }
    }
}
pub fn accessibility_trusted(prompt: bool) -> bool {
    unsafe {
        if !prompt {
            return AXIsProcessTrusted() != 0;
        }
        let keys = [kAXTrustedCheckOptionPrompt as CFTypeRef];
        let values = [kCFBooleanTrue];
        let options = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            std::ptr::addr_of!(kCFTypeDictionaryKeyCallBacks) as *const c_void,
            std::ptr::addr_of!(kCFTypeDictionaryValueCallBacks) as *const c_void,
        );
        if options.is_null() {
            return AXIsProcessTrusted() != 0;
        }
        let trusted = AXIsProcessTrustedWithOptions(options) != 0;
        CFRelease(options);
        trusted
    }
}
pub fn input_monitoring(request: bool) -> bool {
    unsafe {
        if CGPreflightListenEventAccess() {
            return true;
        }
        request && CGRequestListenEventAccess()
    }
}
const LAYOUT_ISO: u32 = u32::from_be_bytes(*b"ISO ");
pub fn iso_keyboard(keyboard_type: Option<i64>) -> bool {
    let keyboard_type = keyboard_type
        .and_then(|t| i16::try_from(t).ok())
        .unwrap_or_else(|| i16::from(unsafe { LMGetKbdType() }));
    unsafe { KBGetLayoutType(keyboard_type) == LAYOUT_ISO }
}
