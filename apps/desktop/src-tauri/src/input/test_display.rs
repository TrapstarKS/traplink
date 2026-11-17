//! Cursor sink for receiver tests. An encrypted handoff exercises the same
//! receiver and return-edge logic without moving the developer's OS cursor.
use crate::{flow::Bounds, screen::Layout};
use std::cell::Cell;

thread_local! {
    static CURSOR: Cell<(i32, i32)> = const { Cell::new((960, 540)) };
}

pub(super) fn layout() -> Option<Layout> {
    Layout::new(vec![
        Bounds::new(0, 0, 1920, 1080)?,
        Bounds::new(-1440, 180, 1440, 900)?,
    ])
}

pub(super) fn cursor() -> Option<(i32, i32)> {
    Some(CURSOR.get())
}

pub(super) fn move_absolute(point: (i32, i32), layout: &Layout) -> Result<(), String> {
    if point != layout.clamp(point) {
        return Err("Test cursor left the display.".into());
    }
    CURSOR.set(point);
    Ok(())
}
