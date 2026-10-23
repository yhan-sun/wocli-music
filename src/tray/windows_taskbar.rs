//! Explorer child-window hosting. No desktop/topmost fallback and no Explorer injection.
use super::taskbar_layout::{
    classic_slot, modern_slot_free, pixels, snapshot_fresh, GeometryKey, Rect,
};
use crate::store::Paths;
use std::{
    ptr::null,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver},
        Arc,
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{GetLastError, SetLastError, HWND, POINT, RECT},
    Graphics::Gdi::ScreenToClient,
    UI::{
        HiDpi::{GetDpiForWindow, GetWindowDpiAwarenessContext, SetThreadDpiAwarenessContext},
        WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, FindWindowExW, FindWindowW, GetParent,
            GetWindowLongPtrW, GetWindowRect, IsWindow, IsWindowVisible,
            SetLayeredWindowAttributes, SetParent, SetWindowLongPtrW, SetWindowPos, ShowWindow,
            GWL_EXSTYLE, GWL_STYLE, LWA_COLORKEY, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE,
            SW_SHOWNOACTIVATE, WS_CHILD, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
            WS_POPUP,
        },
    },
};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn rect(raw: RECT) -> Rect {
    Rect {
        left: raw.left,
        top: raw.top,
        right: raw.right,
        bottom: raw.bottom,
    }
}
unsafe fn window_rect(hwnd: HWND) -> Option<Rect> {
    let mut raw = std::mem::zeroed();
    (hwnd != 0 && GetWindowRect(hwnd, &mut raw) != 0).then(|| rect(raw))
}
unsafe fn child(parent: HWND, class: &str) -> HWND {
    FindWindowExW(parent, 0, wide(class).as_ptr(), null())
}
unsafe fn client_rect(parent: HWND, screen: Rect) -> Option<Rect> {
    let mut point = POINT {
        x: screen.left,
        y: screen.top,
    };
    if ScreenToClient(parent, &mut point) == 0 {
        return None;
    }
    Some(Rect {
        left: point.x,
        top: point.y,
        right: point.x + screen.width(),
        bottom: point.y + screen.height(),
    })
}
unsafe fn move_window(hwnd: HWND, bounds: Rect) -> bool {
    if window_rect(hwnd).and_then(|r| client_rect(GetParent(hwnd), r)) == Some(bounds) {
        return true;
    }
    SetWindowPos(
        hwnd,
        0,
        bounds.left,
        bounds.top,
        bounds.width(),
        bounds.height(),
        SWP_NOACTIVATE | SWP_NOZORDER,
    ) != 0
}
struct DpiGuard(isize);
impl DpiGuard {
    unsafe fn for_window(hwnd: HWND) -> Self {
        Self(SetThreadDpiAwarenessContext(GetWindowDpiAwarenessContext(
            hwnd,
        )))
    }
}
impl Drop for DpiGuard {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe {
                SetThreadDpiAwarenessContext(self.0);
            }
        }
    }
}

struct Snapshot {
    taskbar: HWND,
    at: Instant,
    bounds: Rect,
    generation: u64,
    occupied: std::result::Result<Vec<Rect>, String>,
}
struct Reservation {
    hwnd: HWND,
    parent: HWND,
    original: Rect,
    applied: Rect,
}
impl Reservation {
    unsafe fn restore(&self) {
        let _dpi = DpiGuard::for_window(self.parent);
        if IsWindow(self.hwnd) != 0 && GetParent(self.hwnd) == self.parent {
            if let Some(current) = window_rect(self.hwnd).and_then(|r| client_rect(self.parent, r))
            {
                // Do not overwrite a newer Explorer or another application's layout.
                if current == self.applied {
                    move_window(self.hwnd, self.original);
                }
            }
        }
    }
}

// Inspired by ANYNC/TaskbarLyrics (MIT), commit 71262b4: query only Explorer-owned
// hosts, use one background read, and expire geometry from query start rather than completion.
fn spawn_probe(
    cancel: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
) -> Receiver<Snapshot> {
    let (tx, updates) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        use ::windows::{
            core::Interface,
            Win32::{
                Foundation::HWND as ComHwnd,
                System::Com::{
                    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                    COINIT_MULTITHREADED,
                },
                UI::Accessibility::*,
            },
        };
        unsafe {
            if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
                return;
            }
            let automation: ::windows::core::Result<IUIAutomation> =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER);
            if let Ok(automation) = &automation {
                if let Ok(config) = automation.cast::<IUIAutomation2>() {
                    let _ = config.SetConnectionTimeout(800);
                    let _ = config.SetTransactionTimeout(1000);
                }
            }
            while !cancel.load(Ordering::Relaxed) {
                if active.load(Ordering::Relaxed) {
                    let at = Instant::now();
                    let sample_generation = generation.load(Ordering::Relaxed);
                    let shell = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
                    let _dpi = DpiGuard::for_window(shell);
                    let bounds = window_rect(shell).unwrap_or_default();
                    let result = (|| -> std::result::Result<Vec<Rect>, String> {
                        if shell == 0 {
                            return Err("未找到 Explorer 任务栏".into());
                        }
                        let rebar = classic_container(shell);
                        if rebar.1 != 0 {
                            return Ok(vec![]);
                        }
                        let auto = automation
                            .as_ref()
                            .map_err(|e| format!("UI Automation 初始化失败：{e}"))?;
                        let read = (|| -> ::windows::core::Result<Vec<Rect>> {
                            let root = auto.ElementFromHandle(ComHwnd(shell as *mut _))?;
                            let owner = root.CurrentProcessId()?;
                            let explorer = auto
                                .CreatePropertyCondition(UIA_ProcessIdPropertyId, &owner.into())?;
                            let visible = auto.CreatePropertyCondition(
                                UIA_IsOffscreenPropertyId,
                                &false.into(),
                            )?;
                            let types = [
                                UIA_ButtonControlTypeId,
                                UIA_SplitButtonControlTypeId,
                                UIA_MenuItemControlTypeId,
                                UIA_TabItemControlTypeId,
                                UIA_ListItemControlTypeId,
                                UIA_EditControlTypeId,
                                UIA_ComboBoxControlTypeId,
                                UIA_CheckBoxControlTypeId,
                                UIA_RadioButtonControlTypeId,
                                UIA_HyperlinkControlTypeId,
                            ];
                            let mut types_cond: Option<IUIAutomationCondition> = None;
                            for ct in types {
                                let prop = auto.CreatePropertyCondition(
                                    UIA_ControlTypePropertyId,
                                    &(ct.0).into(),
                                )?;
                                types_cond = match types_cond {
                                    Some(prev) => Some(auto.CreateOrCondition(&prev, &prop)?),
                                    None => Some(prop),
                                };
                            }
                            let types_cond = types_cond.unwrap();
                            let exp_vis = auto.CreateAndCondition(&explorer, &visible)?;
                            let condition = auto.CreateAndCondition(&exp_vis, &types_cond)?;
                            let cache = auto.CreateCacheRequest()?;
                            cache.SetTreeScope(TreeScope_Element)?;
                            for property in [
                                UIA_BoundingRectanglePropertyId,
                                UIA_ControlTypePropertyId,
                                UIA_IsOffscreenPropertyId,
                            ] {
                                cache.AddProperty(property)?;
                            }
                            let hosts =
                                root.FindAllBuildCache(TreeScope_Children, &explorer, &cache)?;
                            let host_count = hosts.Length()?;
                            let mut occupied = vec![];
                            let mut total = 0;
                            if host_count > 64 {
                                return Ok(vec![]);
                            }
                            for h in 0..host_count {
                                if cancel.load(Ordering::Relaxed) {
                                    break;
                                }
                                let host = hosts.GetElement(h)?;
                                if !host.CachedIsOffscreen()?.as_bool()
                                    && types.contains(&host.CachedControlType()?)
                                {
                                    let raw = host.CachedBoundingRectangle()?;
                                    let r = Rect {
                                        left: raw.left,
                                        top: raw.top,
                                        right: raw.right,
                                        bottom: raw.bottom,
                                    };
                                    if r.width() > 0 && r.height() > 0 && r.overlaps(bounds) {
                                        occupied.push(r);
                                    }
                                }
                                let controls = host.FindAllBuildCache(
                                    TreeScope_Descendants,
                                    &condition,
                                    &cache,
                                )?;
                                let count = controls.Length()?;
                                total += count;
                                if total > 512 {
                                    return Ok(vec![]);
                                }
                                for i in 0..count {
                                    let item = controls.GetElement(i)?;
                                    let raw = item.CachedBoundingRectangle()?;
                                    let r = Rect {
                                        left: raw.left,
                                        top: raw.top,
                                        right: raw.right,
                                        bottom: raw.bottom,
                                    };
                                    if r.width() > 0 && r.height() > 0 && r.overlaps(bounds) {
                                        occupied.push(r);
                                    }
                                }
                            }
                            Ok(occupied)
                        })();
                        let occupied = read.map_err(|e| format!("读取任务栏按钮失败：{e}"))?;
                        if occupied.is_empty() {
                            return Err("未取得任务栏交互控件边界，歌词暂时隐藏".into());
                        }
                        Ok(occupied)
                    })();
                    let _ = tx.try_send(Snapshot {
                        taskbar: shell,
                        at,
                        bounds,
                        generation: sample_generation,
                        occupied: result,
                    });
                }
                for _ in 0..5 {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            drop(automation);
            CoUninitialize();
        }
    });
    updates
}
unsafe fn classic_container(shell: HWND) -> (HWND, HWND) {
    // Feature-detect the modern XAML taskbar, including Windows 11 classic-shell replacements.
    if child(shell, "Windows.UI.Composition.DesktopWindowContentBridge") != 0 {
        return (0, 0);
    }
    for name in ["ReBarWindow32", "WorkerW"] {
        let parent = child(shell, name);
        if parent != 0 {
            for class in ["MSTaskSwWClass", "MSTaskListWClass"] {
                let list = child(parent, class);
                if list != 0 {
                    return (parent, list);
                }
            }
        }
    }
    (0, 0)
}

pub struct TaskbarLyrics {
    pub hwnd: HWND,
    paths: Paths,
    class: Vec<u16>,
    parent: HWND,
    shell: HWND,
    reservation: Option<Reservation>,
    snapshot: Option<Snapshot>,
    updates: Receiver<Snapshot>,
    cancel: Arc<AtomicBool>,
    last_report: String,
    active: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    last_enabled: bool,
}
impl TaskbarLyrics {
    pub fn new(paths: &Paths, class: Vec<u16>) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicU64::new(0));
        let updates = spawn_probe(cancel.clone(), active.clone(), generation.clone());
        Self {
            hwnd: 0,
            paths: paths.clone(),
            class,
            parent: 0,
            shell: 0,
            reservation: None,
            snapshot: None,
            updates,
            cancel,
            last_report: String::new(),
            active,
            generation,
            last_enabled: false,
        }
    }
    pub unsafe fn reset(&mut self) {
        self.hide();
        if self.hwnd != 0 && IsWindow(self.hwnd) != 0 {
            DestroyWindow(self.hwnd);
        }
        self.hwnd = 0;
        self.parent = 0;
        self.shell = 0;
        self.snapshot = None;
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
    unsafe fn hide(&mut self) {
        if self.hwnd != 0 && IsWindow(self.hwnd) != 0 {
            ShowWindow(self.hwnd, SW_HIDE);
        }
        if let Some(reservation) = self.reservation.take() {
            reservation.restore();
        }
    }
    fn report(&mut self, visible: bool, note: &str) {
        let report=serde_json::json!({"pid":std::process::id(),"mode":"taskbar-child","attached":unsafe { self.hwnd!=0 && IsWindow(self.hwnd)!=0 && GetParent(self.hwnd)==self.parent },"visible":visible,"note":note}).to_string();
        if report != self.last_report {
            let _ = std::fs::write(self.paths.runtime.join("taskbar.json"), &report);
            self.last_report = report;
        }
    }
    unsafe fn attach(&mut self, shell: HWND, parent: HWND) -> bool {
        if shell == 0 || parent == 0 || IsWindow(shell) == 0 || IsWindow(parent) == 0 {
            self.reset();
            self.report(false, "任务栏父窗口不可用，等待 Explorer 恢复");
            return false;
        }
        if self.hwnd != 0
            && IsWindow(self.hwnd) != 0
            && self.shell == shell
            && self.parent == parent
            && GetParent(self.hwnd) == parent
        {
            return true;
        }
        self.reset();
        // Create hidden; WS_VISIBLE and WS_EX_TOPMOST are intentionally never used.
        // WS_EX_TOOLWINDOW is omitted because Explorer child windows should not be tool windows.
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_NOACTIVATE,
            self.class.as_ptr(),
            wide("wocli taskbar lyrics").as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            0,
            0,
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(null()),
            null(),
        );
        if hwnd == 0 {
            self.report(false, &format!("创建歌词窗口失败：{}", GetLastError()));
            return false;
        }
        if SetLayeredWindowAttributes(hwnd, 0x000A0A0A, 0, LWA_COLORKEY) == 0 {
            let error = GetLastError();
            DestroyWindow(hwnd);
            self.report(false, &format!("任务栏透明表面初始化失败：{error}"));
            return false;
        }
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetWindowLongPtrW(
            hwnd,
            GWL_STYLE,
            (style & !(WS_POPUP as isize)) | WS_CHILD as isize,
        );
        let child_style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        if child_style & WS_CHILD as isize == 0 || child_style & WS_POPUP as isize != 0 {
            DestroyWindow(hwnd);
            self.report(false, "无法切换为任务栏子窗口样式");
            return false;
        }
        let exstyle = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            (exstyle & !(WS_EX_TOOLWINDOW as isize)) | WS_EX_NOACTIVATE as isize,
        );
        SetLastError(0);
        SetParent(hwnd, parent);
        let error = GetLastError();
        if GetParent(hwnd) != parent {
            DestroyWindow(hwnd);
            self.report(
                false,
                &format!("无法嵌入任务栏（错误 {error}），未启用桌面悬浮回退"),
            );
            return false;
        }
        self.hwnd = hwnd;
        self.shell = shell;
        self.parent = parent;
        true
    }
    pub unsafe fn refresh(&mut self, enabled: bool) {
        if enabled != self.last_enabled {
            self.generation.fetch_add(1, Ordering::Relaxed);
            self.snapshot = None;
            self.active.store(enabled, Ordering::Relaxed);
            self.last_enabled = enabled;
        }
        while let Ok(snapshot) = self.updates.try_recv() {
            self.snapshot = Some(snapshot);
        }
        if !enabled {
            self.hide();
            self.report(false, "歌词已关闭、停止播放或处于老板模式");
            return;
        }
        let shell = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
        if shell == 0 || IsWindowVisible(shell) == 0 {
            self.hide();
            self.report(false, "任务栏不可见，等待 Explorer 恢复");
            return;
        }
        let _dpi = DpiGuard::for_window(shell);
        let dpi = GetDpiForWindow(shell).max(96);
        let Some(bar) = window_rect(shell) else {
            self.hide();
            return;
        };
        if bar.height() < pixels(24, dpi) || bar.width() <= bar.height() {
            self.hide();
            self.report(false, "任务栏已收起或为竖向布局，歌词暂时隐藏");
            return;
        }
        let (rebar, tasklist) = classic_container(shell);
        let parent = if tasklist != 0 { rebar } else { shell };
        if !self.attach(shell, parent) {
            return;
        }
        let slot = if tasklist != 0 {
            let current = window_rect(tasklist).and_then(|r| client_rect(parent, r));
            let original = match (&self.reservation, current) {
                (Some(old), Some(current)) if old.hwnd == tasklist && old.applied == current => {
                    Some(old.original)
                }
                (_, current) => {
                    self.reservation = None;
                    current
                }
            };
            if let Some((slot, band)) = original.and_then(|r| classic_slot(r, dpi)) {
                if move_window(tasklist, band) {
                    self.reservation = Some(Reservation {
                        hwnd: tasklist,
                        parent,
                        original: original.unwrap(),
                        applied: band,
                    });
                    Some(slot)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            let notify = window_rect(child(shell, "TrayNotifyWnd"));
            match &self.snapshot {
                Some(snapshot)
                    if snapshot_fresh(
                        GeometryKey {
                            hwnd: snapshot.taskbar,
                            bounds: snapshot.bounds,
                            generation: snapshot.generation,
                        },
                        GeometryKey {
                            hwnd: shell,
                            bounds: bar,
                            generation: self.generation.load(Ordering::Relaxed),
                        },
                        snapshot.at.elapsed(),
                    ) =>
                {
                    match &snapshot.occupied {
                        Ok(occupied) => modern_slot_free(bar, notify, occupied, dpi)
                            .and_then(|r| client_rect(parent, r)),
                        Err(note) => {
                            let note = note.clone();
                            self.hide();
                            self.report(false, &note);
                            return;
                        }
                    }
                }
                _ => {
                    self.hide();
                    self.report(false, "等待任务栏按钮位置，超时或信息不全时保持隐藏");
                    return;
                }
            }
        };
        if let Some(slot) = slot {
            if move_window(self.hwnd, slot) {
                ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                self.report(
                    true,
                    if tasklist != 0 {
                        "已嵌入经典任务栏，已预留歌词空间"
                    } else {
                        "已嵌入任务栏，按交互控件边界避让"
                    },
                );
                return;
            }
        }
        self.hide();
        self.report(false, "任务栏空间不足或布局更新失败，未覆盖任务栏按钮");
    }
    pub unsafe fn shutdown(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.reset();
    }
}
impl Drop for TaskbarLyrics {
    fn drop(&mut self) {
        unsafe {
            self.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::{
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            DefWindowProcW, RegisterClassW, UnregisterClassW, GWL_EXSTYLE, WNDCLASSW,
            WS_EX_TOPMOST, WS_VISIBLE,
        },
    };
    struct Host {
        hwnd: HWND,
        class: Vec<u16>,
    }
    impl Host {
        unsafe fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let class = wide(&format!(
                "wocli-test-host-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut wc: WNDCLASSW = std::mem::zeroed();
            wc.hInstance = GetModuleHandleW(null());
            wc.lpszClassName = class.as_ptr();
            wc.lpfnWndProc = Some(DefWindowProcW);
            assert_ne!(
                RegisterClassW(&wc),
                0,
                "RegisterClass error {}",
                GetLastError()
            );
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP,
                0,
                0,
                1400,
                48,
                0,
                0,
                wc.hInstance,
                null(),
            );
            assert_ne!(hwnd, 0, "CreateWindow error {}", GetLastError());
            Self { hwnd, class }
        }
    }
    impl Drop for Host {
        fn drop(&mut self) {
            unsafe {
                if IsWindow(self.hwnd) != 0 {
                    DestroyWindow(self.hwnd);
                }
                UnregisterClassW(self.class.as_ptr(), GetModuleHandleW(null()));
            }
        }
    }
    fn paths(root: &tempfile::TempDir) -> Paths {
        Paths {
            home: root.path().into(),
            runtime: root.path().into(),
        }
    }
    #[test]
    fn native_surface_is_a_hidden_child_and_recreated_when_parent_changes() {
        unsafe {
            let root = tempfile::tempdir().unwrap();
            let first = Host::new();
            let second = Host::new();
            let mut surface = TaskbarLyrics::new(&paths(&root), first.class.clone());
            assert!(surface.attach(first.hwnd, first.hwnd));
            let old = surface.hwnd;
            assert_eq!(GetParent(old), first.hwnd);
            assert_ne!(GetWindowLongPtrW(old, GWL_STYLE) & WS_CHILD as isize, 0);
            assert_eq!(GetWindowLongPtrW(old, GWL_STYLE) & WS_POPUP as isize, 0);
            assert_eq!(
                GetWindowLongPtrW(old, GWL_EXSTYLE) & WS_EX_TOPMOST as isize,
                0
            );
            assert_eq!(GetWindowLongPtrW(old, GWL_STYLE) & WS_VISIBLE as isize, 0);
            assert!(surface.attach(second.hwnd, second.hwnd));
            assert_eq!(GetParent(surface.hwnd), second.hwnd);
            assert_eq!(IsWindow(old), 0);
            assert!(!surface.attach(0, 0));
            assert_eq!(surface.hwnd, 0);
        }
    }
    #[test]
    fn native_classic_band_restores_only_the_layout_we_applied() {
        unsafe {
            let host = Host::new();
            let instance = GetModuleHandleW(null());
            let band = CreateWindowExW(
                0,
                host.class.as_ptr(),
                host.class.as_ptr(),
                WS_CHILD,
                100,
                0,
                1100,
                40,
                host.hwnd,
                0,
                instance,
                null(),
            );
            assert_ne!(band, 0);
            let _dpi = DpiGuard::for_window(host.hwnd);
            let original = window_rect(band)
                .and_then(|r| client_rect(host.hwnd, r))
                .unwrap();
            let (_, applied) = classic_slot(original, 96).unwrap();
            assert!(move_window(band, applied));
            let reservation = Reservation {
                hwnd: band,
                parent: host.hwnd,
                original,
                applied,
            };
            reservation.restore();
            assert_eq!(
                window_rect(band).and_then(|r| client_rect(host.hwnd, r)),
                Some(original)
            );
            assert!(move_window(band, applied));
            let changed = Rect {
                right: applied.right - 20,
                ..applied
            };
            assert!(move_window(band, changed));
            reservation.restore();
            assert_eq!(
                window_rect(band).and_then(|r| client_rect(host.hwnd, r)),
                Some(changed)
            );
            DestroyWindow(band);
        }
    }
}
