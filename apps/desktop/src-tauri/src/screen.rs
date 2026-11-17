use crate::{flow::Bounds, model::FlowEdge};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    monitors: Vec<Bounds>,
}

fn along(edge: FlowEdge, (x, y): (i32, i32)) -> i32 {
    match edge {
        FlowEdge::Left | FlowEdge::Right => y,
        FlowEdge::Top | FlowEdge::Bottom => x,
    }
}
fn span(edge: FlowEdge, b: &Bounds) -> (i32, i32) {
    match edge {
        FlowEdge::Left | FlowEdge::Right => (b.top, b.bottom),
        FlowEdge::Top | FlowEdge::Bottom => (b.left, b.right),
    }
}
fn extremeness(edge: FlowEdge, b: &Bounds) -> i32 {
    match edge {
        FlowEdge::Left => -b.left,
        FlowEdge::Right => b.right,
        FlowEdge::Top => -b.top,
        FlowEdge::Bottom => b.bottom,
    }
}
fn contains(b: &Bounds, (x, y): (i32, i32)) -> bool {
    (b.left..=b.right).contains(&x) && (b.top..=b.bottom).contains(&y)
}
fn distance_sq(b: &Bounds, p: (i32, i32)) -> i64 {
    let (cx, cy) = b.clamp(p);
    let (dx, dy) = (i64::from(p.0 - cx), i64::from(p.1 - cy));
    dx * dx + dy * dy
}

impl Layout {
    pub fn new(monitors: Vec<Bounds>) -> Option<Self> {
        (!monitors.is_empty()).then_some(Self { monitors })
    }
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn single(bounds: Bounds) -> Self {
        Self {
            monitors: vec![bounds],
        }
    }
    pub fn virtual_bounds(&self) -> Bounds {
        let first = self.monitors[0];
        self.monitors.iter().fold(first, |a, b| Bounds {
            left: a.left.min(b.left),
            top: a.top.min(b.top),
            right: a.right.max(b.right),
            bottom: a.bottom.max(b.bottom),
        })
    }
    #[cfg(any(target_os = "macos", target_os = "windows", test))]
    pub fn primary_point(&self, x: f64, y: f64) -> (i32, i32) {
        let bounds = self.monitor_for((0, 0));
        (
            bounds.left + (x * f64::from(bounds.right - bounds.left)).round() as i32,
            bounds.top + (y * f64::from(bounds.bottom - bounds.top)).round() as i32,
        )
    }
    pub fn monitor_for(&self, p: (i32, i32)) -> &Bounds {
        self.monitors
            .iter()
            .find(|m| contains(m, p))
            .unwrap_or_else(|| {
                self.monitors
                    .iter()
                    .min_by_key(|m| distance_sq(m, p))
                    .expect("layout is never empty")
            })
    }
    pub fn clamp(&self, p: (i32, i32)) -> (i32, i32) {
        if self.monitors.iter().any(|m| contains(m, p)) {
            p
        } else {
            self.monitor_for(p).clamp(p)
        }
    }
    fn continues(&self, m: &Bounds, edge: FlowEdge, at: i32) -> bool {
        self.monitors.iter().any(|n| {
            n != m && {
                let (start, end) = span(edge, n);
                let across = match edge {
                    FlowEdge::Left => n.right + 1 >= m.left && n.left < m.left,
                    FlowEdge::Right => n.left <= m.right + 1 && n.right > m.right,
                    FlowEdge::Top => n.bottom + 1 >= m.top && n.top < m.top,
                    FlowEdge::Bottom => n.top <= m.bottom + 1 && n.bottom > m.bottom,
                };
                across && (start..=end).contains(&at)
            }
        })
    }
    fn monitor_from(&self, p: (i32, i32), from: (i32, i32)) -> &Bounds {
        self.monitors
            .iter()
            .find(|m| contains(m, p))
            .unwrap_or_else(|| self.monitor_for(from))
    }
    pub fn clamp_from(&self, p: (i32, i32), from: (i32, i32)) -> (i32, i32) {
        self.monitor_from(p, from).clamp(p)
    }
    #[cfg(test)]
    pub fn outer_edge_overshoot(&self, edge: FlowEdge, p: (i32, i32)) -> Option<i32> {
        self.outer_edge_overshoot_from(edge, p, p)
    }
    pub fn outer_edge_overshoot_from(
        &self,
        edge: FlowEdge,
        p: (i32, i32),
        from: (i32, i32),
    ) -> Option<i32> {
        let m = self.monitor_from(p, from);
        let at = along(edge, m.clamp(p));
        if self.continues(m, edge, at) {
            return None;
        }
        let beyond = match edge {
            FlowEdge::Left => m.left - p.0,
            FlowEdge::Right => p.0 - m.right,
            FlowEdge::Top => m.top - p.1,
            FlowEdge::Bottom => p.1 - m.bottom,
        };
        (beyond >= 0).then_some(beyond)
    }
    pub fn distance_from_edge(&self, edge: FlowEdge, p: (i32, i32)) -> i32 {
        let m = self.monitor_for(p);
        m.distance(edge, m.clamp(p))
    }
    pub fn entry(&self, edge: FlowEdge, p: (i32, i32)) -> f64 {
        self.virtual_bounds().entry(edge, p)
    }
    pub fn entry_point(&self, edge: FlowEdge, entry: f64, inset: i32) -> (i32, i32) {
        let v = self.virtual_bounds();
        let probe = v.point_on(edge, entry, 0);
        let at = along(edge, probe);
        let covering = self
            .monitors
            .iter()
            .filter(|m| {
                let (start, end) = span(edge, m);
                (start..=end).contains(&at)
            })
            .max_by_key(|m| extremeness(edge, m));
        let m = covering.unwrap_or_else(|| {
            self.monitors
                .iter()
                .min_by_key(|m| {
                    let (start, end) = span(edge, m);
                    (at.clamp(start, end) - at).abs()
                })
                .expect("layout is never empty")
        });
        let (start, end) = span(edge, m);
        let at = at.clamp(start, end);
        let point = match edge {
            FlowEdge::Left => (m.left + inset, at),
            FlowEdge::Right => (m.right - inset, at),
            FlowEdge::Top => (at, m.top + inset),
            FlowEdge::Bottom => (at, m.bottom - inset),
        };
        m.clamp(point)
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn layout_from_rects(rects: &[(f64, f64, f64, f64)]) -> Option<Layout> {
    let monitors = rects
        .iter()
        .filter(|(x, y, w, h)| [x, y, w, h].iter().all(|v| v.is_finite() && v.abs() < 1e7))
        .filter_map(|(x, y, w, h)| {
            Bounds::new(
                x.round() as i32,
                y.round() as i32,
                w.round() as i32,
                h.round() as i32,
            )
        })
        .collect();
    Layout::new(monitors)
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn pointer_location(layout: &Layout, location: (f64, f64)) -> (i32, i32) {
    let floor = |v: f64| {
        if v.is_finite() {
            v.floor().clamp(-1e7, 1e7) as i32
        } else {
            0
        }
    };
    layout.clamp((floor(location.0), floor(location.1)))
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn pointer_sample(layout: &Layout, location: (f64, f64), delta: (f64, f64)) -> (i32, i32) {
    let p = pointer_location(layout, location);
    let step = |d: f64| {
        if !d.is_finite() || d == 0.0 {
            0
        } else {
            (d.abs()
                .round()
                .clamp(1.0, f64::from(crate::flow::MAX_DELTA)) as i32)
                * d.signum() as i32
        }
    };
    let (sx, sy) = (step(delta.0), step(delta.1));
    let mut out = p;
    if sx != 0 {
        let q = (p.0 + sx, p.1);
        if layout.clamp(q) != q {
            out.0 = q.0;
        }
    }
    if sy != 0 {
        let q = (p.0, p.1 + sy);
        if layout.clamp(q) != q {
            out.1 = q.1;
        }
    }
    out
}

#[cfg(any(target_os = "macos", test))]
pub fn accumulate_delta(residual: &mut (f64, f64), (dx, dy): (f64, f64)) -> (i32, i32) {
    let limit = f64::from(crate::flow::MAX_DELTA);
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

#[cfg_attr(not(windows), allow(dead_code))]
pub fn normalize_absolute(x: i32, origin: i32, size: i32) -> i32 {
    let size = i64::from(size.max(1));
    let offset = i64::from(x - origin).clamp(0, size - 1);
    ((offset * 65536 + 32768) / size).clamp(0, 65535) as i32
}

#[cfg(target_os = "windows")]
pub mod win {
    use super::Layout;
    use crate::flow::Bounds;
    use windows_sys::Win32::{
        Foundation::{LPARAM, POINT, RECT},
        Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO},
        UI::{
            HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
            WindowsAndMessaging::{
                GetCursorPos, GetSystemMetrics, SetCursorPos, SM_CMONITORS, SM_CXVIRTUALSCREEN,
                SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
            },
        },
    };

    pub fn dpi_aware() {
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    }
    pub fn signature() -> [i32; 5] {
        dpi_aware();
        unsafe {
            [
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
                GetSystemMetrics(SM_CMONITORS),
            ]
        }
    }
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> windows_sys::core::BOOL {
        let monitors = &mut *(data as *mut Vec<Bounds>);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) != 0 {
            let r = info.rcMonitor;
            if let Some(b) = Bounds::new(r.left, r.top, r.right - r.left, r.bottom - r.top) {
                monitors.push(b);
            }
        }
        1
    }
    pub fn layout() -> Option<Layout> {
        dpi_aware();
        let mut monitors: Vec<Bounds> = Vec::new();
        unsafe {
            EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                Some(collect),
                &mut monitors as *mut Vec<Bounds> as LPARAM,
            )
        };
        Layout::new(monitors).or_else(|| {
            let [x, y, w, h, _] = signature();
            Bounds::new(x, y, w, h).map(Layout::single)
        })
    }
    pub fn cursor() -> Option<(i32, i32)> {
        dpi_aware();
        let mut point = POINT { x: 0, y: 0 };
        (unsafe { GetCursorPos(&mut point) } != 0).then_some((point.x, point.y))
    }
    pub fn set_cursor((x, y): (i32, i32)) -> bool {
        dpi_aware();
        unsafe { SetCursorPos(x, y) != 0 }
    }
    pub fn vk_for_scan(scan: u16, extended: bool) -> Option<u16> {
        use windows_sys::Win32::UI::{
            Input::KeyboardAndMouse::{GetKeyboardLayout, MapVirtualKeyExW, MAPVK_VSC_TO_VK_EX},
            WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
        };
        let code = u32::from(scan & 0xFF) | if extended { 0xE000 } else { 0 };
        unsafe {
            let window = GetForegroundWindow();
            let thread = if window.is_null() {
                0
            } else {
                GetWindowThreadProcessId(window, std::ptr::null_mut())
            };
            let vk = MapVirtualKeyExW(code, MAPVK_VSC_TO_VK_EX, GetKeyboardLayout(thread));
            u16::try_from(vk).ok().filter(|vk| *vk != 0)
        }
    }
    #[cfg(not(test))]
    pub fn move_absolute((x, y): (i32, i32), layout: &Layout) -> Result<(), String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE,
            MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
        };
        dpi_aware();
        let v = layout.virtual_bounds();
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: super::normalize_absolute(x, v.left, v.right - v.left + 1),
                    dy: super::normalize_absolute(y, v.top, v.bottom - v.top + 1),
                    mouseData: 0,
                    dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let sent = unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
        if sent == 1 {
            Ok(())
        } else {
            Err("Native input injection failed: the pointer move was blocked.".into())
        }
    }
}

#[cfg(target_os = "macos")]
pub mod mac {
    use super::Layout;

    pub fn layout() -> Option<Layout> {
        super::layout_from_rects(&crate::macos::displays())
    }
    pub fn cursor() -> Option<(f64, f64)> {
        crate::macos::cursor().map(|p| (p.x, p.y))
    }
    pub fn set_cursor(point: (i32, i32)) -> bool {
        let placed = crate::macos::warp(point);
        // End Quartz's post-warp delay as soon as local control resumes.
        crate::macos::associate(true);
        placed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(left: i32, top: i32, w: i32, h: i32) -> Bounds {
        Bounds::new(left, top, w, h).unwrap()
    }
    fn mixed() -> Layout {
        Layout::new(vec![b(0, 0, 1920, 1080), b(-2560, -360, 2560, 1440)]).unwrap()
    }

    #[test]
    fn virtual_bounds_cover_negative_monitors() {
        assert_eq!(mixed().virtual_bounds(), b(-2560, -360, 4480, 1440));
        assert!(Layout::new(Vec::new()).is_none());
    }

    #[test]
    fn only_outer_edges_count() {
        let l = mixed();
        assert_eq!(l.outer_edge_overshoot(FlowEdge::Left, (0, 500)), None);
        assert_eq!(l.outer_edge_overshoot(FlowEdge::Right, (-1, 500)), None);
        assert_eq!(l.outer_edge_overshoot(FlowEdge::Left, (-2560, 0)), Some(0));
        assert_eq!(l.outer_edge_overshoot(FlowEdge::Left, (-2565, 0)), Some(5));
        assert_eq!(l.outer_edge_overshoot(FlowEdge::Left, (-2500, 0)), None);
        assert_eq!(l.outer_edge_overshoot(FlowEdge::Top, (100, 0)), Some(0));
        assert_eq!(l.outer_edge_overshoot(FlowEdge::Top, (-100, -360)), Some(0));
        assert_eq!(l.outer_edge_overshoot(FlowEdge::Top, (-100, 0)), None);
        assert_eq!(
            l.outer_edge_overshoot(FlowEdge::Right, (1925, 540)),
            Some(6)
        );
        assert_eq!(l.distance_from_edge(FlowEdge::Right, (1900, 540)), 19);
    }

    #[test]
    fn non_rectangular_seam_with_partial_overlap() {
        let l = Layout::new(vec![b(0, 0, 1920, 1080), b(1920, -500, 1920, 1080)]).unwrap();
        assert_eq!(l.outer_edge_overshoot(FlowEdge::Right, (1919, 100)), None);
        assert_eq!(
            l.outer_edge_overshoot(FlowEdge::Right, (1919, 800)),
            Some(0)
        );
        assert_eq!(l.outer_edge_overshoot(FlowEdge::Right, (3839, 0)), Some(0));
    }

    #[test]
    fn entry_points_land_on_the_outer_monitor() {
        let l = mixed();
        let v = l.virtual_bounds();
        assert_eq!(l.entry_point(FlowEdge::Left, 0.0, 2), (-2558, -360));
        assert_eq!(l.entry_point(FlowEdge::Left, 1.0, 2), (-2558, 1079));
        assert_eq!(l.entry_point(FlowEdge::Right, 0.0, 2), (-3, -360));
        let p = l.entry_point(FlowEdge::Right, 0.75, 2);
        assert_eq!(p.0, 1917);
        assert!((l.entry(FlowEdge::Right, p) - 0.75).abs() < 0.002);
        assert_eq!(l.entry_point(FlowEdge::Top, 1.0, 2), (1919, 2));
        assert_eq!(v.left, -2560);
        for edge in [
            FlowEdge::Left,
            FlowEdge::Right,
            FlowEdge::Top,
            FlowEdge::Bottom,
        ] {
            for i in 0..=20 {
                let p = l.entry_point(edge, f64::from(i) / 20.0, 2);
                assert_eq!(l.clamp(p), p, "{edge:?} {i}");
            }
        }
    }

    #[test]
    fn clamp_keeps_points_on_a_monitor() {
        let l = mixed();
        assert_eq!(l.clamp((1000, -200)), (1000, 0));
        assert_eq!(l.clamp((-100, -200)), (-100, -200));
        assert_eq!(l.clamp((5000, 5000)), (1919, 1079));
    }

    #[test]
    fn layout_from_point_rects() {
        let l = layout_from_rects(&[
            (0.0, 0.0, 1512.0, 982.0),
            (-400.0, -1080.0, 1920.0, 1080.0),
            (0.0, 0.0, 0.0, 0.0),
            (f64::NAN, 0.0, 10.0, 10.0),
        ])
        .unwrap();
        assert_eq!(
            l,
            Layout::new(vec![b(0, 0, 1512, 982), b(-400, -1080, 1920, 1080)]).unwrap()
        );
        assert!(layout_from_rects(&[]).is_none());
        assert!(layout_from_rects(&[(0.0, 0.0, -5.0, 10.0)]).is_none());
    }

    #[test]
    fn normalized_mac_input_uses_primary_display_points() {
        let l = layout_from_rects(&[(0.0, 0.0, 1512.0, 982.0), (-1920.0, -100.0, 1920.0, 1080.0)])
            .unwrap();
        assert_eq!(l.primary_point(0.0, 0.0), (0, 0));
        assert_eq!(l.primary_point(1.0, 1.0), (1511, 981));
        assert_eq!(l.primary_point(0.5, 0.5), (756, 491));
    }

    #[test]
    fn fractional_relative_motion_accumulates_and_reverses() {
        let mut residual = (0.0, 0.0);
        assert_eq!(accumulate_delta(&mut residual, (0.4, -0.6)), (0, 0));
        assert_eq!(accumulate_delta(&mut residual, (0.4, -0.6)), (0, -1));
        assert_eq!(accumulate_delta(&mut residual, (0.4, 2.0)), (1, 1));
        assert!((residual.0 - 0.2).abs() < 1e-9 && (residual.1 - 0.8).abs() < 1e-9);
        assert_eq!(accumulate_delta(&mut residual, (-0.2, -0.8)), (0, 0));
        assert!(residual.0.abs() < 1e-9 && residual.1.abs() < 1e-9);
        assert_eq!(accumulate_delta(&mut residual, (f64::NAN, 0.0)), (0, 0));
        assert_eq!(
            accumulate_delta(&mut residual, (1e9, 0.0)).0,
            crate::flow::MAX_DELTA
        );
    }

    #[test]
    fn clamped_pointer_reports_push_past_outer_edges() {
        let l = mixed();
        assert_eq!(pointer_sample(&l, (500.4, 300.9), (3.0, -2.0)), (500, 300));
        assert_eq!(pointer_sample(&l, (1919.6, 540.0), (4.2, 0.0)), (1923, 540));
        assert_eq!(pointer_sample(&l, (1919.0, 540.0), (0.3, 0.0)), (1920, 540));
        assert_eq!(
            pointer_sample(&l, (1919.0, 540.0), (-2.0, 5.0)),
            (1919, 540)
        );
        assert_eq!(pointer_sample(&l, (0.0, 500.0), (-6.0, 0.0)), (0, 500));
        assert_eq!(pointer_sample(&l, (100.0, 0.0), (0.0, -3.0)), (100, -3));
        assert_eq!(
            pointer_sample(&l, (-2560.0, -360.0), (-2.0, -2.0)),
            (-2562, -362)
        );
        assert_eq!(
            pointer_sample(&l, (f64::NAN, 5.0), (f64::INFINITY, 0.0)),
            (0, 5)
        );
    }

    #[test]
    fn absolute_normalization_round_trips() {
        for (origin, size) in [(0, 1920), (-2560, 4480), (0, 1), (-1280, 3200), (0, 7680)] {
            let pixels = [origin, origin + 1, origin + size / 2, origin + size - 1];
            for x in pixels.into_iter().filter(|x| *x < origin + size) {
                let n = normalize_absolute(x, origin, size);
                assert!((0..=65535).contains(&n));
                let back = origin + ((i64::from(n) * i64::from(size)) / 65536) as i32;
                assert_eq!(back, x, "origin {origin} size {size}");
            }
        }
        assert_eq!(
            normalize_absolute(-99999, 0, 1920),
            normalize_absolute(0, 0, 1920)
        );
    }
}
