//! Geometry in screen pixels. Never fall back to screen/desktop coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
impl Rect {
    pub fn width(self) -> i32 {
        self.right - self.left
    }
    pub fn height(self) -> i32 {
        self.bottom - self.top
    }
    pub fn overlaps(self, other: Self) -> bool {
        self.left < other.right
            && self.right > other.left
            && self.top < other.bottom
            && self.bottom > other.top
    }
}
pub fn pixels(value: i32, dpi: u32) -> i32 {
    ((value as i64 * dpi.clamp(96, 768) as i64 + 48) / 96) as i32
}
/// Find an actual empty slot, preferring the gap immediately before the system tray.
///
/// On Windows 10, `notify` is `Some(Rect)` from `TrayNotifyWnd`.
/// On Windows 11, `notify` is `None` because the notification area is part of Explorer's XAML tree,
/// and its interactive buttons are reported directly in `occupied`.
pub fn modern_slot_free(
    taskbar: Rect,
    notify: Option<Rect>,
    occupied: &[Rect],
    dpi: u32,
) -> Option<Rect> {
    if taskbar.height() < pixels(24, dpi)
        || taskbar.height() > pixels(120, dpi)
        || taskbar.width() <= taskbar.height()
    {
        return None;
    }
    if let Some(n) = notify {
        if !taskbar.overlaps(n) {
            return None;
        }
    }
    let margin = pixels(6, dpi);
    let left = taskbar.left + margin;
    let right = notify
        .map(|n| (n.left - margin).min(taskbar.right - margin))
        .unwrap_or(taskbar.right - margin);
    if right <= left {
        return None;
    }
    let mut intervals = occupied
        .iter()
        .filter(|r| r.overlaps(taskbar))
        .map(|r| ((r.left - margin).max(left), (r.right + margin).min(right)))
        .filter(|(l, r)| l < r)
        .collect::<Vec<_>>();
    // Seeing only notification-area controls does not establish that the app-button area is empty.
    if intervals.is_empty() {
        return None;
    }
    intervals.sort_unstable();
    let mut merged: Vec<(i32, i32)> = vec![];
    for (l, r) in intervals {
        if let Some(last) = merged.last_mut().filter(|last| last.1 >= l) {
            last.1 = last.1.max(r);
        } else {
            merged.push((l, r));
        }
    }
    let minimum = pixels(100, dpi);
    let desired = pixels(400, dpi);
    let mut end = right;
    for (start, stop) in merged
        .into_iter()
        .rev()
        .chain(std::iter::once((left, left)))
    {
        if end - stop >= minimum {
            let width = desired.min(end - stop);
            let height = pixels(32, dpi).min(taskbar.height() - pixels(4, dpi));
            let top = taskbar.top + (taskbar.height() - height) / 2;
            return Some(Rect {
                left: end - width,
                top,
                right: end,
                bottom: top + height,
            });
        }
        end = end.min(start);
    }
    None
}
pub fn modern_slot(taskbar: Rect, notify: Rect, occupied: &[Rect], dpi: u32) -> Option<Rect> {
    modern_slot_free(taskbar, Some(notify), occupied, dpi)
}
/// Reserve space in the classic task-button band, leaving its buttons a usable width.
pub fn classic_slot(tasklist: Rect, dpi: u32) -> Option<(Rect, Rect)> {
    if tasklist.height() < pixels(24, dpi) || tasklist.height() > pixels(120, dpi) {
        return None;
    }
    let gap = pixels(6, dpi);
    let available = tasklist.width() - pixels(160, dpi) - gap;
    if available < pixels(120, dpi) {
        return None;
    }
    let width = available.min(pixels(400, dpi));
    let height = pixels(32, dpi).min(tasklist.height() - pixels(4, dpi));
    let top = tasklist.top + (tasklist.height() - height) / 2;
    let slot = Rect {
        left: tasklist.right - width,
        top,
        right: tasklist.right,
        bottom: top + height,
    };
    let shrunk = Rect {
        right: slot.left - gap,
        ..tasklist
    };
    Some((slot, shrunk))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn rect(l: i32, t: i32, r: i32, b: i32) -> Rect {
        Rect {
            left: l,
            top: t,
            right: r,
            bottom: b,
        }
    }
    #[test]
    fn places_before_tray_and_avoids_centered_buttons() {
        let bar = rect(0, 1032, 1920, 1080);
        let tray = rect(1550, 1032, 1920, 1080);
        let buttons = rect(650, 1032, 1100, 1080);
        let slot = modern_slot(bar, tray, &[buttons], 96).unwrap();
        assert_eq!(slot.right, 1544);
        assert!(!slot.overlaps(buttons));
        assert!(!slot.overlaps(tray));
    }
    #[test]
    fn never_covers_icons_or_uses_desktop_when_full_or_hidden() {
        let bar = rect(0, 1032, 1920, 1080);
        let tray = rect(1500, 1032, 1920, 1080);
        assert!(modern_slot(bar, tray, &[rect(0, 1032, 1490, 1080)], 96).is_none());
        assert!(modern_slot(bar, tray, &[tray], 96).is_none());
        assert!(modern_slot(bar, tray, &[], 96).is_none());
        assert!(modern_slot(rect(0, 1078, 1920, 1080), tray, &[], 96).is_none());
        assert!(modern_slot(rect(0, 0, 48, 1080), rect(0, 900, 48, 1080), &[], 96).is_none());
    }
    #[test]
    fn dpi_negative_monitor_origin_top_taskbar_and_multiple_obstacles() {
        let bar = rect(-2560, -48, 0, 24);
        let tray = rect(-400, -48, 0, 24);
        let occupied = [
            rect(-2500, -48, -1800, 24),
            rect(-1700, -48, -1200, 24),
            rect(-1600, -48, -1300, 24),
        ];
        let slot = modern_slot(bar, tray, &occupied, 144).unwrap();
        assert_eq!(slot.width(), 600);
        assert_eq!(slot.height(), 48);
        assert!(occupied.iter().all(|r| !slot.overlaps(*r)));
        assert!(!slot.overlaps(tray));
    }
    #[test]
    fn classic_reservation_is_bounded_and_never_consumes_all_buttons() {
        let original = rect(200, 0, 1400, 40);
        let (slot, band) = classic_slot(original, 96).unwrap();
        assert_eq!(slot.width(), 400);
        assert_eq!(band.left, original.left);
        assert_eq!(slot.right, original.right);
        assert!(!slot.overlaps(band));
        assert!(band.width() >= 160);
        assert!(classic_slot(rect(0, 0, 250, 40), 96).is_none());
    }
    #[test]
    fn windows_11_without_notify_window_places_between_app_buttons_and_tray() {
        let bar = rect(0, 1032, 1920, 1080);
        let start_widgets = rect(10, 1032, 60, 1080);
        let centered_apps = rect(700, 1032, 1200, 1080);
        let tray_cluster = rect(1650, 1032, 1910, 1080);
        let occupied = [start_widgets, centered_apps, tray_cluster];
        let slot = modern_slot_free(bar, None, &occupied, 96).unwrap();
        assert_eq!(slot.right, 1644);
        assert!(slot.width() <= 400);
        assert!(slot.width() >= 100);
        for obs in &occupied {
            assert!(!slot.overlaps(*obs));
        }
    }
    #[test]
    fn windows_11_without_notify_returns_none_when_taskbar_is_crowded() {
        let bar = rect(0, 1032, 1920, 1080);
        let apps = rect(0, 1032, 1580, 1080);
        let tray = rect(1620, 1032, 1920, 1080);
        assert!(modern_slot_free(bar, None, &[apps, tray], 96).is_none());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryKey {
    pub hwnd: isize,
    pub bounds: Rect,
    pub generation: u64,
}
pub fn snapshot_fresh(sample: GeometryKey, current: GeometryKey, age: std::time::Duration) -> bool {
    sample == current && current.hwnd != 0 && age < std::time::Duration::from_secs(2)
}
#[cfg(test)]
mod snapshot_tests {
    use super::*;
    #[test]
    fn ignores_late_geometry_after_restart_move_cancel_or_slow_query() {
        let key = GeometryKey {
            hwnd: 7,
            bounds: Rect {
                left: 0,
                top: 1000,
                right: 1920,
                bottom: 1080,
            },
            generation: 3,
        };
        assert!(snapshot_fresh(
            key,
            key,
            std::time::Duration::from_millis(500)
        ));
        for stale in [
            GeometryKey { hwnd: 8, ..key },
            GeometryKey {
                generation: 4,
                ..key
            },
            GeometryKey {
                bounds: Rect {
                    left: -1920,
                    right: 0,
                    ..key.bounds
                },
                ..key
            },
        ] {
            assert!(!snapshot_fresh(stale, key, std::time::Duration::ZERO));
        }
        assert!(!snapshot_fresh(key, key, std::time::Duration::from_secs(2)));
    }
}
