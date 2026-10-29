#![allow(unexpected_cfgs)]
#![allow(unused_imports)]

#[cfg(target_os = "macos")]
mod macos_hotkey;
#[cfg(any(windows, test))]
#[path = "tray/taskbar_layout.rs"]
mod taskbar_layout;
#[cfg(any(windows, test))]
#[path = "tray/windows_backend.rs"]
mod windows_backend;
#[cfg(windows)]
#[path = "tray/windows_taskbar.rs"]
mod windows_taskbar;

#[cfg(any(target_os = "macos", windows))]
use crate::ipc;
use crate::lyrics;
#[cfg(any(target_os = "macos", windows))]
use crate::model::{clock, mode_label};
use crate::model::{Error, Result};
use crate::store::Paths;
use fs2::FileExt;
#[cfg(any(target_os = "macos", windows))]
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::process::Command;

#[cfg(any(target_os = "macos", windows, test))]
struct DailyRequest<'a>(&'a std::sync::atomic::AtomicBool);

#[cfg(any(target_os = "macos", windows, test))]
impl<'a> DailyRequest<'a> {
    fn start(busy: &'a std::sync::atomic::AtomicBool) -> Option<Self> {
        busy.compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .ok()
        .map(|_| Self(busy))
    }
}

#[cfg(any(target_os = "macos", windows, test))]
impl Drop for DailyRequest<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(any(target_os = "macos", windows))]
fn play_daily(paths: Paths) {
    static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let Some(request) = DailyRequest::start(&BUSY) else {
        return;
    };
    std::thread::spawn(move || {
        // Keep one recommendation lookup (including its error dialog) in flight.
        // Repeated menu clicks must not create unbounded workers and requests.
        let _request = request;
        let result = (|| -> Result<()> {
            // The daemon invalidates this intent when the user controls playback
            // while recommendations are loading. A late result must not replace
            // a newer song, stop, or pause request.
            let prepared = ipc::request(&paths, json!({"op":"prepare_play"}), true)?
                .ok_or_else(|| Error::new("后台状态不可用"))?;
            let token = prepared["play_token"]
                .as_str()
                .ok_or_else(|| Error::new("后台未返回推荐播放凭据"))?;
            let store = crate::store::Store::open(&paths)?;
            let provider = crate::provider::Provider::new(paths.settings()?.direct);
            let daily = crate::recommend::daily(&store, &provider, true, false)?;
            let tracks: Vec<_> = daily.items.into_iter().map(|item| item.track).collect();
            if tracks.is_empty() {
                return Err(Error::new("每日推荐为空，请先收藏歌曲或导入歌单"));
            }
            ipc::request(
                &paths,
                json!({"op":"play", "tracks":tracks, "index":0, "play_token":token}),
                false,
            )?
            .ok_or_else(|| Error::coded("cancelled", "后台已退出，已取消推荐播放"))?;
            Ok(())
        })();
        if let Err(error) = result {
            if error.code != "cancelled" {
                eprintln!("每日推荐：{error}");
                daily_error_notice(&error.message);
            }
        }
    });
}

#[cfg(target_os = "macos")]
fn daily_error_notice(message: &str) {
    // Pass the message as an argv value, not AppleScript source.
    let script =
        "on run argv\n display alert \"每日推荐\" message (item 1 of argv) as warning\nend run";
    let _ = Command::new("osascript")
        .args(["-e", script, message])
        .status();
}

#[cfg(windows)]
fn daily_error_notice(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONWARNING, MB_OK};
    let text: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    let title: Vec<u16> = "每日推荐"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        MessageBoxW(0, text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONWARNING);
    }
}

pub fn hotkeys(paths: &Paths) -> serde_json::Value {
    let pid = fs::read_to_string(pid_path(paths)).unwrap_or_default();
    let running = status(paths).unwrap_or(false);
    if running {
        if let Ok(data) = fs::read(paths.runtime.join("hotkeys.json")) {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&data) {
                if value["pid"].as_u64().map(|p| p.to_string()).as_deref() == Some(pid.trim()) {
                    return value;
                }
            }
        }
    }
    serde_json::json!({"keys":[], "note":if running { "托盘正在初始化或仍是旧版，未提供快捷键注册结果；稍后重试，或运行 wocli tray stop 后再 wocli tray start" } else { "全局快捷键需要托盘运行；macOS/Windows 使用 wocli tray start，Linux 需在桌面设置绑定 wocli boss" }})
}
#[cfg(any(target_os = "macos", windows))]
fn record_hotkeys(paths: &Paths, keys: Vec<serde_json::Value>) {
    for key in &keys {
        if key["registered"] != true {
            eprintln!(
                "全局快捷键 {} 注册失败（{}），请检查按键占用；仍可使用托盘菜单或 wocli boss",
                key["shortcut"], key["error_code"]
            );
        }
    }
    let value = serde_json::json!({"pid":std::process::id(),"keys":keys});
    let _ = fs::write(paths.runtime.join("hotkeys.json"), value.to_string());
}

pub fn lyrics_surface(paths: &Paths) -> serde_json::Value {
    let pid = fs::read_to_string(pid_path(paths)).unwrap_or_default();
    if status(paths).unwrap_or(false) {
        if let Ok(bytes) = fs::read(paths.runtime.join("taskbar.json")) {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if value["pid"].as_u64().map(|p| p.to_string()).as_deref() == Some(pid.trim()) {
                    return value;
                }
            }
        }
    }
    serde_json::json!({"mode":"taskbar-child","attached":false,"visible":false,"note":"任务栏歌词正在初始化或托盘未运行；请先运行 wocli tray start"})
}

pub fn active_lyric(lyrics_text: &str, position: f64, offset: i64) -> Option<String> {
    if lyrics_text.trim().is_empty() {
        return None;
    }
    let lines = lyrics::parse(lyrics_text);
    let index = lyrics::current(&lines, position, offset)?;
    let line = lines.get(index)?;
    let trimmed = line.text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(any(windows, test))]
fn tooltip_utf16<const N: usize>(text: &str) -> [u16; N] {
    let mut result = [0; N];
    let mut length = 0;
    for character in text.chars().take_while(|c| *c != '\0') {
        let mut encoded = [0; 2];
        let units = character.encode_utf16(&mut encoded);
        if length + units.len() >= N {
            break;
        }
        result[length..length + units.len()].copy_from_slice(units);
        length += units.len();
    }
    result
}

pub fn lock_path(paths: &Paths) -> PathBuf {
    paths.runtime.join("tray.lock")
}

pub fn pid_path(paths: &Paths) -> PathBuf {
    paths.runtime.join("tray.pid")
}

pub fn status(paths: &Paths) -> Result<bool> {
    let lock_file = lock_path(paths);
    if !lock_file.exists() {
        return Ok(false);
    }
    let Ok(file) = OpenOptions::new().write(true).open(&lock_file) else {
        return Ok(false);
    };
    if file.try_lock_exclusive().is_ok() {
        // Keep the inode stable: a concurrent starter may already have opened
        // this file. Unlinking lets two processes lock different files at once.
        let _ = fs::remove_file(pid_path(paths));
        Ok(false)
    } else {
        Ok(true)
    }
}

#[cfg(windows)]
fn tray_window_for_pid(pid: u32) -> windows_sys::Win32::Foundation::HWND {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetWindowThreadProcessId};
    let class: Vec<u16> = "wocli_tray_class".encode_utf16().chain(Some(0)).collect();
    let mut previous = 0;
    loop {
        let hwnd = unsafe { FindWindowExW(0, previous, class.as_ptr(), std::ptr::null()) };
        if hwnd == 0 {
            return 0;
        }
        let mut owner = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut owner) };
        if owner == pid {
            return hwnd;
        }
        previous = hwnd;
    }
}

pub fn stop(paths: &Paths) -> Result<()> {
    if !status(paths)? {
        return Ok(());
    }
    let pid_file = pid_path(paths);
    if pid_file.exists() {
        if let Ok(content) = fs::read_to_string(&pid_file) {
            if let Ok(pid) = content.trim().parse::<u32>() {
                #[cfg(unix)]
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
                #[cfg(windows)]
                {
                    unsafe {
                        use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
                        // Different libraries can run separate trays with the same
                        // window class. Find this PID instead of only the first tray.
                        let hwnd = tray_window_for_pid(pid);
                        if hwnd != 0 && PostMessageW(hwnd, WM_CLOSE, 0, 0) != 0 {
                            // Give WM_DESTROY time to restore the classic task-button band.
                            for _ in 0..40 {
                                if !status(paths)? {
                                    break;
                                }
                                std::thread::sleep(std::time::Duration::from_millis(50));
                            }
                        }
                    }
                    if status(paths)? {
                        let _ = Command::new("taskkill")
                            .args(["/F", "/PID", &pid.to_string()])
                            .output();
                    }
                }
            }
        }
    }
    let _ = fs::remove_file(pid_file);
    Ok(())
}

pub fn start(paths: &Paths) -> Result<()> {
    if status(paths)? {
        return Ok(());
    }
    #[cfg(unix)]
    let exe = std::env::current_exe()?;
    #[cfg(unix)]
    {
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(paths.runtime.join("tray.log"))
            .map(std::process::Stdio::from)
            .unwrap_or_else(|_| std::process::Stdio::null());
        let err_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(paths.runtime.join("tray.log"))
            .map(std::process::Stdio::from)
            .unwrap_or_else(|_| std::process::Stdio::null());
        let mut cmd = Command::new(&exe);
        cmd.arg("tray")
            .arg("run")
            .arg("--home")
            .arg(&paths.home)
            .env("WOCLI_HOME", &paths.home)
            .stdin(std::process::Stdio::null())
            .stdout(log_file)
            .stderr(err_file);
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        cmd.spawn()?;
    }
    #[cfg(windows)]
    {
        let _process = crate::ipc::spawn_detached(paths, &["tray", "run"])?;
    }
    for _ in 0..60 {
        if status(paths)?
            && (!cfg!(any(target_os = "macos", windows))
                || hotkeys(paths)["keys"]
                    .as_array()
                    .is_some_and(|keys| !keys.is_empty()))
        {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Err(Error::new(format!(
        "托盘未就绪，请检查 {}",
        paths.runtime.join("tray.log").display()
    )))
}

pub fn run(paths: Paths) -> Result<()> {
    fs::create_dir_all(&paths.runtime)?;
    let lock_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(lock_path(&paths))?;
    if lock_file.try_lock_exclusive().is_err() {
        return Err(Error::new("托盘服务已在运行"));
    }
    fs::write(pid_path(&paths), format!("{}\n", std::process::id()))?;

    #[cfg(target_os = "macos")]
    {
        macos::run_tray(paths)
    }
    #[cfg(windows)]
    {
        windows::run_tray(paths)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        println!("当前平台（Linux）暂未启用图形托盘，可使用 CLI 与 TUI 命令控制播放。");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// macOS Cocoa Status Bar & Lyrics Implementation
// ---------------------------------------------------------------------------
#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use cocoa::appkit::{
        NSApp, NSApplication, NSApplicationActivationPolicyAccessory, NSBackingStoreBuffered,
        NSStatusBar, NSWindowCollectionBehavior, NSWindowStyleMask,
    };
    use cocoa::base::{id, nil, NO, YES};
    use cocoa::foundation::{NSAutoreleasePool, NSPoint, NSRect, NSSize, NSString};
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Object, Sel};
    use objc::{class, msg_send, sel, sel_impl};
    use std::sync::atomic::{AtomicU32, Ordering};

    static PENDING_ACTION: AtomicU32 = AtomicU32::new(0);

    const TAG_INFO: isize = 0;
    const TAG_STATUS: isize = 1;
    const TAG_LYRIC: isize = 2;
    const TAG_TOGGLE: isize = 10;
    const TAG_PREV: isize = 11;
    const TAG_NEXT: isize = 12;
    const TAG_MODE: isize = 13;
    const TAG_VOL_UP: isize = 14;
    const TAG_VOL_DOWN: isize = 15;
    const TAG_LYRICS_BAR: isize = 16;
    const TAG_BOSS: isize = 18;
    const TAG_CONTROLS: isize = 19;
    const TAG_DAILY: isize = 20;
    const TAG_TUI: isize = 17;
    const TAG_QUIT: isize = 99;

    unsafe fn trigger_action(tag: isize) {
        PENDING_ACTION.store(tag as u32, Ordering::SeqCst);
        let app = NSApp();
        let empty_event: id = msg_send![
            class!(NSEvent),
            otherEventWithType: 14isize // NSEventTypeApplicationDefined
            location: cocoa::foundation::NSPoint::new(0.0, 0.0)
            modifierFlags: 0usize
            timestamp: 0.0f64
            windowNumber: 0isize
            context: nil
            subtype: 0i16
            data1: 0isize
            data2: 0isize
        ];
        if empty_event != nil {
            let _: () = msg_send![app, postEvent: empty_event atStart: YES];
        }
    }

    pub(super) fn boss_shortcut_pressed() {
        unsafe {
            trigger_action(TAG_BOSS);
        }
    }

    extern "C" fn on_menu_action(_this: &Object, _cmd: Sel, sender: id) {
        unsafe {
            let tag: isize = msg_send![sender, tag];
            trigger_action(tag);
        }
    }

    extern "C" fn on_remote_toggle(_this: &Object, _cmd: Sel, _event: id) -> isize {
        unsafe {
            trigger_action(TAG_TOGGLE);
        }
        0 // MPRemoteCommandHandlerStatusSuccess
    }

    extern "C" fn on_remote_prev(_this: &Object, _cmd: Sel, _event: id) -> isize {
        unsafe {
            trigger_action(TAG_PREV);
        }
        0
    }

    extern "C" fn on_remote_next(_this: &Object, _cmd: Sel, _event: id) -> isize {
        unsafe {
            trigger_action(TAG_NEXT);
        }
        0
    }

    fn target_class() -> &'static Class {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let superclass = class!(NSObject);
            let mut decl = ClassDecl::new("WocliTrayActionTarget", superclass).unwrap();
            unsafe {
                decl.add_method(
                    sel!(onAction:),
                    on_menu_action as extern "C" fn(&Object, Sel, id),
                );
                decl.add_method(
                    sel!(onRemoteToggle:),
                    on_remote_toggle as extern "C" fn(&Object, Sel, id) -> isize,
                );
                decl.add_method(
                    sel!(onRemotePrev:),
                    on_remote_prev as extern "C" fn(&Object, Sel, id) -> isize,
                );
                decl.add_method(
                    sel!(onRemoteNext:),
                    on_remote_next as extern "C" fn(&Object, Sel, id) -> isize,
                );
            }
            decl.register();
        });
        class!(WocliTrayActionTarget)
    }

    unsafe fn setup_remote_commands(target: id) {
        let mp_path = to_nsstring("/System/Library/Frameworks/MediaPlayer.framework");
        let bundle: id = msg_send![class!(NSBundle), bundleWithPath: mp_path];
        if bundle != nil {
            let _: () = msg_send![bundle, load];
        }
        if let Some(center_cls) = Class::get("MPRemoteCommandCenter") {
            let center: id = msg_send![center_cls, sharedCommandCenter];
            if center != nil {
                let cmds: &[(Sel, Sel)] = &[
                    (sel!(togglePlayPauseCommand), sel!(onRemoteToggle:)),
                    (sel!(playCommand), sel!(onRemoteToggle:)),
                    (sel!(pauseCommand), sel!(onRemoteToggle:)),
                    (sel!(previousTrackCommand), sel!(onRemotePrev:)),
                    (sel!(nextTrackCommand), sel!(onRemoteNext:)),
                ];
                for &(cmd_sel, action_sel) in cmds {
                    let cmd: id = msg_send![center, performSelector: cmd_sel];
                    if cmd != nil {
                        let _: () = msg_send![cmd, setEnabled: YES];
                        let _: () = msg_send![cmd, addTarget: target action: action_sel];
                    }
                }
            }
        }
    }

    unsafe fn update_now_playing(
        title: &str,
        artist: &str,
        duration: f64,
        elapsed: f64,
        playing: bool,
    ) {
        if let Some(info_cls) = Class::get("MPNowPlayingInfoCenter") {
            let center: id = msg_send![info_cls, defaultCenter];
            if center != nil {
                let dict: id = msg_send![class!(NSMutableDictionary), dictionary];
                let title_ns = to_nsstring(title);
                let artist_ns = to_nsstring(artist);
                let dur_num: id = msg_send![class!(NSNumber), numberWithDouble: duration];
                let pos_num: id = msg_send![class!(NSNumber), numberWithDouble: elapsed];
                let rate_num: id = msg_send![
                    class!(NSNumber),
                    numberWithDouble: if playing { 1.0f64 } else { 0.0f64 }
                ];

                let k_title = to_nsstring("title");
                let k_artist = to_nsstring("artist");
                let k_dur = to_nsstring("playbackDuration");
                let k_pos = to_nsstring("MPNowPlayingInfoPropertyElapsedPlaybackTime");
                let k_rate = to_nsstring("MPNowPlayingInfoPropertyPlaybackRate");

                let _: () = msg_send![dict, setObject: title_ns forKey: k_title];
                let _: () = msg_send![dict, setObject: artist_ns forKey: k_artist];
                let _: () = msg_send![dict, setObject: dur_num forKey: k_dur];
                let _: () = msg_send![dict, setObject: pos_num forKey: k_pos];
                let _: () = msg_send![dict, setObject: rate_num forKey: k_rate];

                let _: () = msg_send![center, setNowPlayingInfo: dict];
            }
        }
    }

    unsafe fn clear_now_playing() {
        if let Some(info_cls) = Class::get("MPNowPlayingInfoCenter") {
            let center: id = msg_send![info_cls, defaultCenter];
            if center != nil {
                let _: () = msg_send![center, setNowPlayingInfo: nil];
            }
        }
    }

    unsafe fn to_nsstring(s: &str) -> id {
        let str_obj = NSString::alloc(nil).init_str(s);
        let _: () = msg_send![str_obj, autorelease];
        str_obj
    }

    unsafe fn create_item(title: &str, tag: isize, target: id) -> id {
        let item: id = msg_send![class!(NSMenuItem), alloc];
        let title_ns = to_nsstring(title);
        let key_ns = to_nsstring("");
        let item: id =
            msg_send![item, initWithTitle:title_ns action:sel!(onAction:) keyEquivalent:key_ns];
        let _: () = msg_send![item, setTarget: target];
        let _: () = msg_send![item, setTag: tag];
        let _: () = msg_send![item, autorelease];
        item
    }

    unsafe fn set_item_title(item: id, title: &str) {
        let title_ns = to_nsstring(title);
        let _: () = msg_send![item, setTitle: title_ns];
    }

    pub fn run_tray(paths: Paths) -> Result<()> {
        unsafe {
            let _root_pool = NSAutoreleasePool::new(nil);
            let app = NSApp();
            app.setActivationPolicy_(NSApplicationActivationPolicyAccessory);
            let _: () = msg_send![app, finishLaunching];

            let target_obj: id = msg_send![target_class(), alloc];
            let target_obj: id = msg_send![target_obj, init];
            setup_remote_commands(target_obj);
            let boss_hotkey = super::macos_hotkey::BossHotkey::register();
            let boss_shortcut_ready = boss_hotkey.is_ok();
            super::record_hotkeys(
                &paths,
                vec![
                    json!({"shortcut":"Ctrl+Shift+B","registered":boss_shortcut_ready,"error_code":boss_hotkey.as_ref().err().copied()}),
                ],
            );
            let boss_menu_label = if boss_shortcut_ready {
                "老板模式 (Ctrl+Shift+B)"
            } else {
                "老板模式 (快捷键不可用，点击切换)"
            };

            // 1. 顶部菜单栏 StatusItem 与 控制按钮
            let status_bar = NSStatusBar::systemStatusBar(nil);

            // 快捷控制按钮：下一首、播放/暂停、上一首
            // 在 macOS NSStatusBar 中，后创建的 item 会排在前创建 item 的左侧。
            // 依次创建：下一首 -> 播放/暂停 -> 上一首 -> 歌词/状态栏
            // 最终在菜单栏呈现顺序（自左向右）：
            // [ 🎵 歌词/标题 ] [ ⏮ 上一首 ] [ ⏯ 播放/暂停 ] [ ⏭ 下一首 ]
            let item_next = status_bar.statusItemWithLength_(24.0);
            let _: () = msg_send![item_next, retain];
            let btn_next: id = msg_send![item_next, button];
            let _: () = msg_send![btn_next, retain];
            let _: () = msg_send![btn_next, setTarget: target_obj];
            let _: () = msg_send![btn_next, setAction: sel!(onAction:)];
            let _: () = msg_send![btn_next, setTag: TAG_NEXT];
            let _: () = msg_send![btn_next, setImagePosition: 1usize]; // NSImageOnly
            let _: () = msg_send![btn_next, setToolTip: to_nsstring("下一首")];

            let sym_next = to_nsstring("forward.end.fill");
            let img_next: id = msg_send![
                class!(NSImage),
                imageWithSystemSymbolName: sym_next
                accessibilityDescription: nil
            ];
            if img_next != nil {
                let _: () = msg_send![img_next, setTemplate: YES];
                let _: () = msg_send![btn_next, setImage: img_next];
            }

            let item_toggle = status_bar.statusItemWithLength_(24.0);
            let _: () = msg_send![item_toggle, retain];
            let btn_toggle: id = msg_send![item_toggle, button];
            let _: () = msg_send![btn_toggle, retain];
            let _: () = msg_send![btn_toggle, setTarget: target_obj];
            let _: () = msg_send![btn_toggle, setAction: sel!(onAction:)];
            let _: () = msg_send![btn_toggle, setTag: TAG_TOGGLE];
            let _: () = msg_send![btn_toggle, setImagePosition: 1usize]; // NSImageOnly
            let _: () = msg_send![btn_toggle, setToolTip: to_nsstring("播放 / 暂停")];

            let sym_play = to_nsstring("play.fill");
            let img_play: id = msg_send![
                class!(NSImage),
                imageWithSystemSymbolName: sym_play
                accessibilityDescription: nil
            ];
            if img_play != nil {
                let _: () = msg_send![img_play, setTemplate: YES];
                let _: () = msg_send![img_play, retain];
                let _: () = msg_send![btn_toggle, setImage: img_play];
            }

            let sym_pause = to_nsstring("pause.fill");
            let img_pause: id = msg_send![
                class!(NSImage),
                imageWithSystemSymbolName: sym_pause
                accessibilityDescription: nil
            ];
            if img_pause != nil {
                let _: () = msg_send![img_pause, setTemplate: YES];
                let _: () = msg_send![img_pause, retain];
            }

            let item_prev = status_bar.statusItemWithLength_(24.0);
            let _: () = msg_send![item_prev, retain];
            let btn_prev: id = msg_send![item_prev, button];
            let _: () = msg_send![btn_prev, retain];
            let _: () = msg_send![btn_prev, setTarget: target_obj];
            let _: () = msg_send![btn_prev, setAction: sel!(onAction:)];
            let _: () = msg_send![btn_prev, setTag: TAG_PREV];
            let _: () = msg_send![btn_prev, setImagePosition: 1usize]; // NSImageOnly
            let _: () = msg_send![btn_prev, setToolTip: to_nsstring("上一首")];

            let sym_prev = to_nsstring("backward.end.fill");
            let img_prev: id = msg_send![
                class!(NSImage),
                imageWithSystemSymbolName: sym_prev
                accessibilityDescription: nil
            ];
            if img_prev != nil {
                let _: () = msg_send![img_prev, setTemplate: YES];
                let _: () = msg_send![btn_prev, setImage: img_prev];
            }

            // 主歌词 / 状态菜单项
            let status_item = status_bar.statusItemWithLength_(-1.0); // NSVariableStatusItemLength
            let _: () = msg_send![status_item, retain];
            let button: id = msg_send![status_item, button];
            let _: () = msg_send![button, retain];

            let menu: id = msg_send![class!(NSMenu), alloc];
            let menu: id = msg_send![menu, init];
            let _: () = msg_send![menu, retain];

            let item_info = create_item("wocli-music", TAG_INFO, target_obj);
            let _: () = msg_send![item_info, setEnabled: NO];
            let _: () = msg_send![menu, addItem: item_info];

            let item_status = create_item("未连接播放器", TAG_STATUS, target_obj);
            let _: () = msg_send![item_status, setEnabled: NO];
            let _: () = msg_send![menu, addItem: item_status];

            let item_lyric = create_item("暂无歌词", TAG_LYRIC, target_obj);
            let _: () = msg_send![item_lyric, setEnabled: NO];
            let _: () = msg_send![menu, addItem: item_lyric];

            let sep: id = msg_send![class!(NSMenuItem), separatorItem];
            let _: () = msg_send![menu, addItem: sep];

            let item_toggle = create_item("播放 / 暂停", TAG_TOGGLE, target_obj);
            let _: () = msg_send![menu, addItem: item_toggle];

            let item_prev = create_item("上一首", TAG_PREV, target_obj);
            let _: () = msg_send![menu, addItem: item_prev];

            let item_next = create_item("下一首", TAG_NEXT, target_obj);
            let _: () = msg_send![menu, addItem: item_next];

            let sep: id = msg_send![class!(NSMenuItem), separatorItem];
            let _: () = msg_send![menu, addItem: sep];

            let item_mode = create_item("模式: 列表循环", TAG_MODE, target_obj);
            let _: () = msg_send![menu, addItem: item_mode];

            let item_vol_up = create_item("音量 +10%", TAG_VOL_UP, target_obj);
            let _: () = msg_send![menu, addItem: item_vol_up];

            let item_vol_down = create_item("音量 -10%", TAG_VOL_DOWN, target_obj);
            let _: () = msg_send![menu, addItem: item_vol_down];

            let item_lyrics_bar = create_item("任务栏歌词: 开启", TAG_LYRICS_BAR, target_obj);
            let _: () = msg_send![menu, addItem: item_lyrics_bar];

            let item_controls_bar = create_item("菜单栏控制按钮: 开启", TAG_CONTROLS, target_obj);
            let _: () = msg_send![menu, addItem: item_controls_bar];

            let item_boss = create_item(boss_menu_label, TAG_BOSS, target_obj);
            let _: () = msg_send![menu, addItem: item_boss];

            let sep: id = msg_send![class!(NSMenuItem), separatorItem];
            let _: () = msg_send![menu, addItem: sep];

            let item_daily = create_item("播放每日推荐", TAG_DAILY, target_obj);
            let _: () = msg_send![menu, addItem: item_daily];
            let item_tui = create_item("打开终端播放器", TAG_TUI, target_obj);
            let _: () = msg_send![menu, addItem: item_tui];

            let item_quit = create_item("退出", TAG_QUIT, target_obj);
            let _: () = msg_send![menu, addItem: item_quit];

            let _: () = msg_send![status_item, setMenu: menu];

            // 优先使用 macOS 原生 SF Symbol 矢量单色模板图标 (music.note)，自适应深浅色模式
            let symbol_name = to_nsstring("music.note");
            let symbol_img: id = msg_send![
                class!(NSImage),
                imageWithSystemSymbolName: symbol_name
                accessibilityDescription: nil
            ];
            if symbol_img != nil {
                let _: () = msg_send![symbol_img, setTemplate: YES];
                let _: () = msg_send![button, setImage: symbol_img];
                let _: () = msg_send![button, setImagePosition: 2usize]; // NSImageLeft
            }

            let init_title = to_nsstring("wocli");
            let _: () = msg_send![button, setTitle: init_title];

            // 2. 桌面 / 任务栏悬浮透明歌词窗口 (NSPanel)
            let screen: id = msg_send![class!(NSScreen), mainScreen];
            let (screen_w, _screen_h) = if screen != nil {
                let f: NSRect = msg_send![screen, frame];
                (f.size.width, f.size.height)
            } else {
                (1440.0, 900.0)
            };

            let bar_w = 800.0f64;
            let bar_h = 56.0f64;
            let bar_x = (screen_w - bar_w) / 2.0;
            let bar_y = 60.0f64; // 悬浮于 macOS Dock 之上

            let rect = NSRect::new(NSPoint::new(bar_x, bar_y), NSSize::new(bar_w, bar_h));
            let lyrics_window: id = msg_send![class!(NSPanel), alloc];
            let lyrics_window: id = msg_send![
                lyrics_window,
                initWithContentRect: rect
                styleMask: NSWindowStyleMask::NSBorderlessWindowMask
                backing: NSBackingStoreBuffered
                defer: NO
            ];
            let _: () = msg_send![lyrics_window, retain];

            let clear_color: id = msg_send![class!(NSColor), clearColor];
            let _: () = msg_send![lyrics_window, setBackgroundColor: clear_color];
            let _: () = msg_send![lyrics_window, setOpaque: NO];
            let _: () = msg_send![lyrics_window, setHasShadow: NO];
            let _: () = msg_send![lyrics_window, setLevel: 101]; // NSFloatingWindowLevel 置顶
            let _: () = msg_send![lyrics_window, setIgnoresMouseEvents: YES]; // 鼠标点击穿透
            let behavior = NSWindowCollectionBehavior::NSWindowCollectionBehaviorCanJoinAllSpaces
                | NSWindowCollectionBehavior::NSWindowCollectionBehaviorStationary;
            let _: () = msg_send![lyrics_window, setCollectionBehavior: behavior];

            let content_view: id = msg_send![lyrics_window, contentView];
            let lyrics_label: id = msg_send![class!(NSTextField), alloc];
            let label_rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(bar_w, bar_h));
            let lyrics_label: id = msg_send![lyrics_label, initWithFrame: label_rect];
            let _: () = msg_send![lyrics_label, retain];

            let _: () = msg_send![lyrics_label, setBezeled: NO];
            let _: () = msg_send![lyrics_label, setDrawsBackground: NO];
            let _: () = msg_send![lyrics_label, setEditable: NO];
            let _: () = msg_send![lyrics_label, setSelectable: NO];
            let font: id = msg_send![class!(NSFont), boldSystemFontOfSize: 22.0f64];
            let _: () = msg_send![lyrics_label, setFont: font];
            let text_color: id = msg_send![class!(NSColor), whiteColor];
            let _: () = msg_send![lyrics_label, setTextColor: text_color];
            let _: () = msg_send![lyrics_label, setAlignment: 1usize]; // 居中对齐

            let shadow: id = msg_send![class!(NSShadow), alloc];
            let shadow: id = msg_send![shadow, init];
            let shadow_color: id = msg_send![class!(NSColor), colorWithSRGBRed:0.0f64 green:0.0f64 blue:0.0f64 alpha:0.85f64];
            let _: () = msg_send![shadow, setShadowColor: shadow_color];
            let _: () = msg_send![shadow, setShadowOffset: NSSize::new(1.0, -2.0)];
            let _: () = msg_send![shadow, setShadowBlurRadius: 4.0f64];
            let _: () = msg_send![lyrics_label, setShadow: shadow];
            let _: () = msg_send![shadow, release];
            let _: () = msg_send![content_view, addSubview: lyrics_label];

            let empty_str = to_nsstring("");
            let _: () = msg_send![empty_str, retain];
            let mode_str = to_nsstring("kCFRunLoopDefaultMode");
            let _: () = msg_send![mode_str, retain];

            let mut last_menu_info = String::new();
            let mut last_menu_status = String::new();
            let mut last_menu_lyric = String::new();
            let mut last_menu_toggle = String::new();
            let mut last_menu_mode = String::new();
            let mut last_menu_vol = String::new();
            let mut last_menu_lyrics_bar = String::new();
            let mut last_menu_boss = String::new();

            let mut last_title = String::new();
            let mut last_bar_text = String::new();
            let mut current_mode = "list".to_string();
            let mut current_volume = 60u8;
            let mut lyrics_bar_enabled = true;
            let mut controls_enabled = true;
            let mut last_phase = String::new();
            let mut last_controls_visible = true;

            loop {
                let subpool = NSAutoreleasePool::new(nil);

                // 1. 处理来自菜单项的操作
                let action = PENDING_ACTION.swap(0, Ordering::Relaxed);
                if action > 0 {
                    match action as isize {
                        TAG_TOGGLE => {
                            let _ = ipc::request(&paths, json!({"op":"toggle"}), true);
                        }
                        TAG_PREV => {
                            let _ = ipc::request(&paths, json!({"op":"prev"}), true);
                        }
                        TAG_NEXT => {
                            let _ = ipc::request(&paths, json!({"op":"next"}), true);
                        }
                        TAG_MODE => {
                            let next_mode = match current_mode.as_str() {
                                "list" => "repeat",
                                "repeat" => "single",
                                "single" => "shuffle",
                                _ => "list",
                            };
                            let _ = ipc::request(
                                &paths,
                                json!({"op":"config","key":"mode","value":next_mode}),
                                true,
                            );
                        }
                        TAG_VOL_UP => {
                            let next_vol = (current_volume + 10).min(100);
                            let _ = ipc::request(
                                &paths,
                                json!({"op":"config","key":"volume","value":next_vol}),
                                true,
                            );
                        }
                        TAG_VOL_DOWN => {
                            let next_vol = current_volume.saturating_sub(10);
                            let _ = ipc::request(
                                &paths,
                                json!({"op":"config","key":"volume","value":next_vol}),
                                true,
                            );
                        }
                        TAG_LYRICS_BAR => {
                            let next_lb = !lyrics_bar_enabled;
                            let _ = ipc::request(
                                &paths,
                                json!({"op":"config","key":"lyrics_bar","value":next_lb}),
                                true,
                            );
                        }
                        TAG_BOSS => {
                            let _ = ipc::request(&paths, json!({"op":"boss"}), true);
                        }
                        TAG_CONTROLS => {
                            controls_enabled = !controls_enabled;
                            let new_cb = format!(
                                "菜单栏控制按钮: {}",
                                if controls_enabled { "开启" } else { "关闭" }
                            );
                            set_item_title(item_controls_bar, &new_cb);
                            let flag = if controls_enabled { YES } else { NO };
                            let _: () = msg_send![item_prev, setVisible: flag];
                            let _: () = msg_send![item_toggle, setVisible: flag];
                            let _: () = msg_send![item_next, setVisible: flag];
                            last_controls_visible = controls_enabled;
                        }
                        TAG_DAILY => super::play_daily(paths.clone()),
                        TAG_TUI => {
                            let exe =
                                std::env::current_exe().unwrap_or_else(|_| PathBuf::from("wocli"));
                            let script = format!(
                                "tell application \"Terminal\" to do script \"\\\"{}\\\"\"",
                                exe.to_string_lossy()
                            );
                            let _ = Command::new("osascript").args(["-e", &script]).spawn();
                        }
                        TAG_QUIT => {
                            clear_now_playing();
                            let _: () = msg_send![lyrics_window, orderOut: nil];
                            let _: () = msg_send![status_bar, removeStatusItem: item_prev];
                            let _: () = msg_send![status_bar, removeStatusItem: item_toggle];
                            let _: () = msg_send![status_bar, removeStatusItem: item_next];
                            let _: () = msg_send![status_bar, removeStatusItem: status_item];
                            let _ = super::stop(&paths);
                            let _ = ipc::request(&paths, json!({"op":"shutdown"}), false);
                            break;
                        }
                        _ => {}
                    }
                }

                // 2. 轮询后台状态并刷新菜单、状态栏与悬浮歌词
                if let Ok(Some(st)) = ipc::request(&paths, json!({"op":"status"}), false) {
                    let is_boss = st["boss"].as_bool().unwrap_or(false);
                    let phase = st["phase"].as_str().unwrap_or("stopped");
                    let track = &st["track"];
                    let title = track["title"].as_str().unwrap_or("未知歌曲");
                    let artist = track["artist"].as_str().unwrap_or("未知歌手");
                    let pos = st["position"].as_f64().unwrap_or(0.0);
                    let dur = st["duration"].as_f64().unwrap_or(0.0);
                    let lyrics_text = st["lyrics"].as_str().unwrap_or("");
                    let offset = st["lyric_offset"].as_i64().unwrap_or(0);
                    let vol = st["volume"].as_u64().unwrap_or(60) as u8;
                    current_volume = vol;
                    let mode = st["mode"].as_str().unwrap_or("list");
                    current_mode = mode.to_string();
                    lyrics_bar_enabled = st["lyrics_bar"].as_bool().unwrap_or(true);

                    let active_lyr = active_lyric(lyrics_text, pos, offset);
                    let phase_label = match phase {
                        "playing" => "播放中",
                        "paused" => "已暂停",
                        "loading" => "加载中",
                        "error" => "播放失败",
                        _ => "已停止",
                    };

                    let new_info = if is_boss {
                        "wocli".into()
                    } else {
                        format!("{title} · {artist}")
                    };
                    if new_info != last_menu_info {
                        set_item_title(item_info, &new_info);
                        last_menu_info = new_info;
                    }

                    let new_status = if is_boss {
                        "老板模式".into()
                    } else {
                        format!("{phase_label}  {} / {}", clock(pos), clock(dur))
                    };
                    if new_status != last_menu_status {
                        set_item_title(item_status, &new_status);
                        last_menu_status = new_status;
                    }

                    let new_lyric = if is_boss {
                        ""
                    } else {
                        active_lyr.as_deref().unwrap_or("暂无歌词")
                    };
                    if new_lyric != last_menu_lyric {
                        set_item_title(item_lyric, new_lyric);
                        last_menu_lyric = new_lyric.to_string();
                    }

                    let new_toggle = if phase == "playing" {
                        "暂停"
                    } else {
                        "播放"
                    };
                    if new_toggle != last_menu_toggle {
                        set_item_title(item_toggle, new_toggle);
                        last_menu_toggle = new_toggle.to_string();
                    }

                    let new_mode = format!("模式: {}", mode_label(mode));
                    if new_mode != last_menu_mode {
                        set_item_title(item_mode, &new_mode);
                        last_menu_mode = new_mode;
                    }

                    let new_vol = format!("音量 +10% (当前 {vol}%)");
                    if new_vol != last_menu_vol {
                        set_item_title(item_vol_up, &new_vol);
                        last_menu_vol = new_vol;
                    }

                    let new_lb = format!(
                        "任务栏歌词: {}",
                        if lyrics_bar_enabled {
                            "开启"
                        } else {
                            "关闭"
                        }
                    );
                    if new_lb != last_menu_lyrics_bar {
                        set_item_title(item_lyrics_bar, &new_lb);
                        last_menu_lyrics_bar = new_lb;
                    }

                    let new_boss = if is_boss {
                        "老板模式: 开启 (点击退出)"
                    } else {
                        boss_menu_label
                    };
                    if new_boss != last_menu_boss {
                        set_item_title(item_boss, new_boss);
                        last_menu_boss = new_boss.to_string();
                    }

                    // 顶部菜单栏标题：实时呈现当前唱到的歌词（未唱到歌词或间奏时显示歌名·歌手）
                    let menu_bar_title = if is_boss {
                        "wocli".to_string()
                    } else if phase == "playing" {
                        if let Some(lyr) = &active_lyr {
                            let count = lyr.chars().count();
                            if count > 25 {
                                format!("{}…", lyr.chars().take(24).collect::<String>())
                            } else {
                                lyr.clone()
                            }
                        } else {
                            let fallback = format!("{title} · {artist}");
                            if fallback.chars().count() > 25 {
                                format!("{}…", fallback.chars().take(24).collect::<String>())
                            } else {
                                fallback
                            }
                        }
                    } else if phase == "paused" {
                        if let Some(lyr) = &active_lyr {
                            let count = lyr.chars().count();
                            let truncated = if count > 20 {
                                format!("{}…", lyr.chars().take(19).collect::<String>())
                            } else {
                                lyr.clone()
                            };
                            format!("{truncated} (暂停)")
                        } else {
                            let fallback = format!("{title} (暂停)");
                            if fallback.chars().count() > 20 {
                                format!("{}…", fallback.chars().take(19).collect::<String>())
                            } else {
                                fallback
                            }
                        }
                    } else if phase == "loading" {
                        format!("{title} (加载中)")
                    } else {
                        "wocli".to_string()
                    };

                    if menu_bar_title != last_title {
                        let ns_title = to_nsstring(&menu_bar_title);
                        let _: () = msg_send![button, setTitle: ns_title];
                        last_title = menu_bar_title;
                    }

                    // 桌面/任务栏悬浮歌词条更新
                    let bar_display = if is_boss || !lyrics_bar_enabled {
                        None
                    } else if phase == "playing" || phase == "paused" {
                        if let Some(lyr) = &active_lyr {
                            Some(lyr.clone())
                        } else {
                            Some(format!("{title} · {artist}"))
                        }
                    } else {
                        None
                    };

                    if let Some(text) = bar_display {
                        if text != last_bar_text {
                            let ns_lyr = to_nsstring(&text);
                            let _: () = msg_send![lyrics_label, setStringValue: ns_lyr];
                            last_bar_text = text;
                        }
                        let _: () = msg_send![lyrics_window, orderFrontRegardless];
                    } else {
                        if !last_bar_text.is_empty() {
                            let _: () = msg_send![lyrics_label, setStringValue: empty_str];
                            last_bar_text.clear();
                        }
                        let _: () = msg_send![lyrics_window, orderOut: nil];
                    }

                    if !is_boss && (phase == "playing" || phase == "paused") {
                        update_now_playing(title, artist, dur, pos, phase == "playing");
                    } else {
                        clear_now_playing();
                    }

                    let controls_visible = !is_boss && controls_enabled;
                    if controls_visible != last_controls_visible {
                        let flag = if controls_visible { YES } else { NO };
                        let _: () = msg_send![item_prev, setVisible: flag];
                        let _: () = msg_send![item_toggle, setVisible: flag];
                        let _: () = msg_send![item_next, setVisible: flag];
                        last_controls_visible = controls_visible;
                    }

                    if phase != last_phase {
                        if phase == "playing" {
                            if img_pause != nil {
                                let _: () = msg_send![btn_toggle, setImage: img_pause];
                            }
                            let tip_ns = to_nsstring("暂停");
                            let _: () = msg_send![btn_toggle, setToolTip: tip_ns];
                        } else {
                            if img_play != nil {
                                let _: () = msg_send![btn_toggle, setImage: img_play];
                            }
                            let tip_ns = to_nsstring("播放");
                            let _: () = msg_send![btn_toggle, setToolTip: tip_ns];
                        }
                        last_phase = phase.to_string();
                    }
                } else {
                    clear_now_playing();
                    let new_status = "后台未运行";
                    if new_status != last_menu_status {
                        set_item_title(item_status, new_status);
                        last_menu_status = new_status.to_string();
                    }
                    if last_title != "wocli" {
                        let ns_title = to_nsstring("wocli");
                        let _: () = msg_send![button, setTitle: ns_title];
                        last_title = "wocli".to_string();
                    }
                    if last_phase != "stopped" {
                        if img_play != nil {
                            let _: () = msg_send![btn_toggle, setImage: img_play];
                        }
                        let tip_ns = to_nsstring("播放");
                        let _: () = msg_send![btn_toggle, setToolTip: tip_ns];
                        last_phase = "stopped".to_string();
                    }
                    let controls_visible = controls_enabled;
                    if controls_visible != last_controls_visible {
                        let flag = if controls_visible { YES } else { NO };
                        let _: () = msg_send![item_prev, setVisible: flag];
                        let _: () = msg_send![item_toggle, setVisible: flag];
                        let _: () = msg_send![item_next, setVisible: flag];
                        last_controls_visible = controls_visible;
                    }
                    if !last_bar_text.is_empty() {
                        let _: () = msg_send![lyrics_label, setStringValue: empty_str];
                        last_bar_text.clear();
                    }
                    let _: () = msg_send![lyrics_window, orderOut: nil];
                }

                // 3. 处理 Cocoa 事件（0.12 秒超时自动循环，应用自定义事件即刻唤醒）
                let date: id = msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: 0.12f64];
                let event: id = msg_send![
                    app,
                    nextEventMatchingMask: usize::MAX
                    untilDate: date
                    inMode: mode_str
                    dequeue: YES
                ];
                if event != nil {
                    let _: () = msg_send![app, sendEvent: event];
                    let _: () = msg_send![app, updateWindows];
                }

                subpool.drain();
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Windows Win32 System Tray & Taskbar Lyrics Implementation
// ---------------------------------------------------------------------------
#[cfg(windows)]
mod windows {
    use super::*;
    use std::cell::RefCell;
    use std::ptr::null;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
    use windows_sys::Win32::Foundation::{
        GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        BeginPaint, BitBlt, CreateBitmap, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW,
        CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, GetDC,
        InvalidateRect, ReleaseDC, SelectObject, SetBkMode, SetTextColor, DT_CENTER,
        DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FW_BOLD, FW_SEMIBOLD,
        PAINTSTRUCT, SRCCOPY, TRANSPARENT,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
    };
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
        NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CopyIcon, CreateIconIndirect, CreatePopupMenu, CreateWindowExW,
        DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GetClientRect,
        GetCursorPos, GetMessageW, LoadCursorW, LoadIconW, PostMessageW, PostQuitMessage,
        RegisterClassW, RegisterWindowMessageW, SetCursor, SetForegroundWindow, SetTimer,
        SetWindowTextW, TrackPopupMenuEx, TranslateMessage, WindowFromPoint, CS_DBLCLKS,
        CS_HREDRAW, CS_VREDRAW, HICON, HTCLIENT, ICONINFO, IDC_HAND, IDI_APPLICATION,
        MA_NOACTIVATE, MF_DISABLED, MF_SEPARATOR, MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
        TPM_RIGHTBUTTON, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_DISPLAYCHANGE, WM_ERASEBKGND,
        WM_HOTKEY, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE,
        WM_NCHITTEST, WM_NCRBUTTONUP, WM_NULL, WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR,
        WM_SETTINGCHANGE, WM_TIMER, WM_USER, WNDCLASSW,
    };

    const WM_TRAY: u32 = WM_USER + 101;
    const TIMER_ID: usize = 1;

    const CMD_HEADER: usize = 200;
    const CMD_LYRIC: usize = 201;
    const CMD_TOGGLE: usize = 202;
    const CMD_PREV: usize = 203;
    const CMD_NEXT: usize = 204;
    const CMD_MODE: usize = 205;
    const CMD_VOL_UP: usize = 206;
    const CMD_VOL_DOWN: usize = 207;
    const CMD_LYRICS_BAR: usize = 208;
    const CMD_BOSS: usize = 211;
    const CMD_DAILY: usize = 212;
    const CMD_TUI: usize = 209;
    const CMD_QUIT: usize = 210;

    const HOTKEY_PLAY_PAUSE: i32 = 1001;
    const HOTKEY_PREV: i32 = 1002;
    const HOTKEY_NEXT: i32 = 1003;
    const HOTKEY_STOP: i32 = 1004;
    const HOTKEY_BOSS: i32 = 1005;
    const HOTKEY_BOSS_ALT: i32 = 1006;

    const VK_MEDIA_NEXT_TRACK: u32 = 0xB0;
    const VK_MEDIA_PREV_TRACK: u32 = 0xB1;
    const VK_MEDIA_STOP: u32 = 0xB2;
    const VK_MEDIA_PLAY_PAUSE: u32 = 0xB3;
    const VK_KEY_B: u32 = 0x42;

    static HOVER_BTN: AtomicU8 = AtomicU8::new(0);
    static RESET_PENDING: AtomicBool = AtomicBool::new(false);
    static READD_PENDING: AtomicBool = AtomicBool::new(false);
    static CLOSE_PENDING: AtomicBool = AtomicBool::new(false);

    struct TrayState {
        paths: Paths,
        nid: NOTIFYICONDATAW,
        icon_present: bool,
        taskbar: super::windows_taskbar::TaskbarLyrics,
        current_text: String,
        current_title: String,
        current_artist: String,
        current_mode: String,
        current_volume: u8,
        lyrics_bar_enabled: bool,
        boss: bool,
        boss_shortcut_label: String,
        phase: String,
        backend: super::windows_backend::Backend,
        status_checked: Option<std::time::Instant>,
        quitting: bool,
    }

    thread_local! {
        static STATE: RefCell<Option<TrayState>> = const { RefCell::new(None) };
        // Painting can be reentered by native layout calls while STATE is mutably
        // borrowed. Publish a separate, small render snapshot before those calls.
        static PAINT_STATE: RefCell<(String, String)> = const { RefCell::new((String::new(), String::new())) };
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn taskbar_created_msg() -> u32 {
        static MSG_ID: AtomicU32 = AtomicU32::new(0);
        let id = MSG_ID.load(Ordering::Relaxed);
        if id != 0 {
            return id;
        }
        let registered = unsafe { RegisterWindowMessageW(to_wide("TaskbarCreated").as_ptr()) };
        MSG_ID.store(registered, Ordering::Relaxed);
        registered
    }

    unsafe fn create_music_tray_icon() -> HICON {
        let dc = GetDC(0);
        if dc == 0 {
            return fallback_tray_icon();
        }
        let color_dc = CreateCompatibleDC(dc);
        let mask_dc = CreateCompatibleDC(dc);
        let sz = 32;
        let color_bmp = CreateCompatibleBitmap(dc, sz, sz);
        let mask_bmp = CreateBitmap(sz, sz, 1, 1, null());

        let old_color = SelectObject(color_dc, color_bmp);
        let old_mask = SelectObject(mask_dc, mask_bmp);

        let mut rect = RECT {
            left: 0,
            top: 0,
            right: sz,
            bottom: sz,
        };

        // Mask bitmap: 1 (白色) 为透明，0 (黑色) 为不透明
        let white_brush = CreateSolidBrush(0x00FFFFFF);
        FillRect(mask_dc, &rect, white_brush);
        DeleteObject(white_brush);

        // Color bitmap: 底色全黑
        let black_brush = CreateSolidBrush(0x00000000);
        FillRect(color_dc, &rect, black_brush);
        DeleteObject(black_brush);

        SetBkMode(color_dc, TRANSPARENT as i32);
        SetBkMode(mask_dc, TRANSPARENT as i32);

        let font_name = to_wide("Segoe UI Symbol");
        let font = CreateFontW(
            22,
            0,
            0,
            0,
            FW_BOLD as i32,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            font_name.as_ptr(),
        );
        let old_f1 = SelectObject(color_dc, font);
        let old_f2 = SelectObject(mask_dc, font);

        let note_text = to_wide("♫");
        let note_len = 1;

        // Mask 绘制黑色（不透明）
        SetTextColor(mask_dc, 0x00000000);
        DrawTextW(
            mask_dc,
            note_text.as_ptr(),
            note_len,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );

        // Color 绘制金黄色音符
        SetTextColor(color_dc, 0x0000D7FF);
        DrawTextW(
            color_dc,
            note_text.as_ptr(),
            note_len,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );

        SelectObject(color_dc, old_f1);
        SelectObject(mask_dc, old_f2);
        DeleteObject(font);

        SelectObject(color_dc, old_color);
        SelectObject(mask_dc, old_mask);
        DeleteDC(color_dc);
        DeleteDC(mask_dc);
        ReleaseDC(0, dc);

        let icon_info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask_bmp,
            hbmColor: color_bmp,
        };
        let hicon = CreateIconIndirect(&icon_info);

        DeleteObject(color_bmp);
        DeleteObject(mask_bmp);

        if hicon != 0 {
            hicon
        } else {
            fallback_tray_icon()
        }
    }

    unsafe fn fallback_tray_icon() -> HICON {
        // LoadIcon returns a shared system icon; WM_DESTROY owns and destroys
        // nid.hIcon, so give it a private copy even on the fallback path.
        CopyIcon(LoadIconW(0, IDI_APPLICATION))
    }

    fn button_at(hwnd: HWND, x: i32, y: i32) -> u8 {
        unsafe {
            let mut area: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut area);
            let dpi = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd);
            let dp = |v| super::taskbar_layout::pixels(v, dpi);
            let h = area.bottom - area.top;
            if area.right < dp(180) || y < dp(4) || y >= h - dp(4) {
                return 0;
            }
            if (dp(6)..dp(34)).contains(&x) {
                1
            } else if (dp(36)..dp(64)).contains(&x) {
                2
            } else if (dp(66)..dp(94)).contains(&x) {
                3
            } else {
                0
            }
        }
    }
    unsafe extern "system" fn tray_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_TIMER => {
                // Cross-thread Win32 calls can dispatch incoming synchronous
                // messages before returning. Defer work until the outer update ends.
                if !STATE.with(|st| st.try_borrow_mut().is_ok()) {
                    return 0;
                }
                if CLOSE_PENDING.swap(false, Ordering::Relaxed) {
                    DestroyWindow(hwnd);
                    return 0;
                }
                let quit = STATE.with(|st| {
                    if let Some(state) = st.borrow_mut().as_mut() {
                        if READD_PENDING.swap(false, Ordering::Relaxed) {
                            state.icon_present = false;
                        }
                        sync_notification_icon(&mut state.icon_present, false, |operation| {
                            Shell_NotifyIconW(operation, &state.nid) != 0
                        });
                        if RESET_PENDING.swap(false, Ordering::Relaxed) {
                            state.taskbar.reset();
                        }
                        if update_status(state) {
                            return true;
                        }

                        // 检查鼠标是否已移出歌词窗口以重置悬停按钮状态
                        if state.taskbar.hwnd != 0 && HOVER_BTN.load(Ordering::Relaxed) != 0 {
                            let mut cur_pt: POINT = std::mem::zeroed();
                            GetCursorPos(&mut cur_pt);
                            if WindowFromPoint(cur_pt) != state.taskbar.hwnd {
                                HOVER_BTN.store(0, Ordering::Relaxed);
                                InvalidateRect(state.taskbar.hwnd, null(), 0);
                            }
                        }
                    }
                    false
                });
                if quit {
                    DestroyWindow(hwnd);
                }
                0
            }
            WM_TRAY => {
                let event = lparam as u32;
                if event == WM_LBUTTONUP {
                    queue_command(json!({"op":"toggle"}));
                } else if event == WM_RBUTTONUP {
                    show_menu(hwnd);
                }
                0
            }
            WM_COMMAND => {
                let cmd = wparam & 0xFFFF;
                handle_command(hwnd, cmd);
                0
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                RESET_PENDING.store(true, Ordering::Relaxed);
                0
            }
            WM_CLOSE => {
                CLOSE_PENDING.store(true, Ordering::Relaxed);
                PostMessageW(hwnd, WM_TIMER, TIMER_ID, 0);
                0
            }
            WM_HOTKEY => {
                let id = wparam as i32;
                let op = match id {
                    HOTKEY_PLAY_PAUSE => Some("toggle"),
                    HOTKEY_PREV => Some("prev"),
                    HOTKEY_NEXT => Some("next"),
                    HOTKEY_STOP => Some("stop"),
                    HOTKEY_BOSS | HOTKEY_BOSS_ALT => Some("boss"),
                    _ => None,
                };
                if let Some(op) = op {
                    queue_command(json!({"op":op}));
                }
                0
            }
            WM_DESTROY => {
                UnregisterHotKey(hwnd, HOTKEY_PLAY_PAUSE);
                UnregisterHotKey(hwnd, HOTKEY_PREV);
                UnregisterHotKey(hwnd, HOTKEY_NEXT);
                UnregisterHotKey(hwnd, HOTKEY_STOP);
                UnregisterHotKey(hwnd, HOTKEY_BOSS);
                UnregisterHotKey(hwnd, HOTKEY_BOSS_ALT);
                let state = STATE.with(|st| st.borrow_mut().take());
                if let Some(mut state) = state {
                    Shell_NotifyIconW(NIM_DELETE, &state.nid);
                    if state.nid.hIcon != 0 {
                        DestroyIcon(state.nid.hIcon);
                    }
                    state.taskbar.shutdown();
                }
                PostQuitMessage(0);
                0
            }
            _ => {
                if msg == taskbar_created_msg() {
                    READD_PENDING.store(true, Ordering::Relaxed);
                    RESET_PENDING.store(true, Ordering::Relaxed);
                    0
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
        }
    }

    unsafe extern "system" fn lyrics_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_ERASEBKGND => 1,
            WM_NCHITTEST => HTCLIENT as LRESULT,
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            WM_SETCURSOR => {
                let mut point: POINT = std::mem::zeroed();
                GetCursorPos(&mut point);
                windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point);
                if button_at(hwnd, point.x, point.y) != 0 {
                    SetCursor(LoadCursorW(0, IDC_HAND));
                    1
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            WM_MOUSEMOVE => {
                let x = (lparam & 0xFFFF) as i16 as i32;
                let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
                let new_hover = button_at(hwnd, x, y);
                let old_hover = HOVER_BTN.swap(new_hover, Ordering::Relaxed);
                if old_hover != new_hover {
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_LBUTTONDOWN => {
                let x = (lparam & 0xFFFF) as i16 as i32;
                let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
                let op = match button_at(hwnd, x, y) {
                    1 => Some("prev"),
                    2 => Some("toggle"),
                    3 => Some("next"),
                    _ => None,
                };
                if let Some(op) = op {
                    queue_command(json!({"op":op}));
                }
                0
            }
            WM_NCRBUTTONUP | WM_RBUTTONUP => {
                // TrackPopupMenuEx pumps timer and command messages. Do not keep
                // a RefCell borrow alive across its nested message loop.
                let owner = STATE.with(|st| st.borrow().as_ref().map(|state| state.nid.hWnd));
                if let Some(owner) = owner {
                    show_menu(owner);
                }
                0
            }
            WM_LBUTTONDBLCLK => {
                let x = (lparam & 0xFFFF) as i16 as i32;
                let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
                if button_at(hwnd, x, y) == 0 {
                    open_console();
                }
                0
            }
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);

                let mut rect: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut rect);
                let w = rect.right - rect.left;
                let h = rect.bottom - rect.top;

                if w > 0 && h > 0 {
                    let mem_dc = CreateCompatibleDC(hdc);
                    let mem_bmp = CreateCompatibleBitmap(hdc, w, h);
                    let old_bmp = SelectObject(mem_dc, mem_bmp);

                    // 填充透明基色 (0x000A0A0A)
                    let brush = CreateSolidBrush(0x000A0A0A);
                    FillRect(mem_dc, &rect, brush);
                    DeleteObject(brush);

                    SetBkMode(mem_dc, TRANSPARENT as i32);

                    let (text, phase) = PAINT_STATE.with(|snapshot| snapshot.borrow().clone());
                    let is_paused = phase == "paused";

                    let hover = HOVER_BTN.load(Ordering::Relaxed);

                    let dp = |v| {
                        super::taskbar_layout::pixels(
                            v,
                            windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd),
                        )
                    };
                    let controls = w >= dp(180);
                    if controls {
                        // Color-key pixels pass mouse input through to Explorer.
                        // Give each button a solid face matching its entire hitbox.
                        let normal_brush = CreateSolidBrush(0x00202020);
                        let hover_brush = CreateSolidBrush(0x00404040);
                        for (button, left, right) in [(1, 6, 34), (2, 36, 64), (3, 66, 94)] {
                            let btn_rc = RECT {
                                left: dp(left),
                                top: dp(4),
                                right: dp(right),
                                bottom: h - dp(4),
                            };
                            FillRect(
                                mem_dc,
                                &btn_rc,
                                if hover == button {
                                    hover_brush
                                } else {
                                    normal_brush
                                },
                            );
                        }
                        DeleteObject(normal_brush);
                        DeleteObject(hover_brush);

                        // 2. 绘制上一首、播放/暂停、下一首矢量文字图标
                        let btn_font_name = to_wide("Segoe UI Symbol");
                        let btn_font = CreateFontW(
                            dp(16),
                            0,
                            0,
                            0,
                            FW_BOLD as i32,
                            0,
                            0,
                            0,
                            1,
                            0,
                            0,
                            4, // ANTIALIASED_QUALITY
                            0,
                            btn_font_name.as_ptr(),
                        );
                        let old_btn_font = SelectObject(mem_dc, btn_font);

                        let btn_configs = [
                            (1, "⏮", 6, 34),
                            (2, if phase == "playing" { "⏸" } else { "▶" }, 36, 64),
                            (3, "⏭", 66, 94),
                        ];

                        for (btn_idx, glyph, left, right) in btn_configs {
                            let mut btn_rc = RECT {
                                left: dp(left),
                                top: 0,
                                right: dp(right),
                                bottom: h,
                            };
                            let wide_glyph = to_wide(glyph);
                            let glyph_len = wide_glyph.len() as i32 - 1;

                            // 阴影
                            let mut shadow_rc = btn_rc;
                            shadow_rc.left += 1;
                            shadow_rc.top += 1;
                            shadow_rc.right += 1;
                            shadow_rc.bottom += 1;
                            SetTextColor(mem_dc, 0x00000000);
                            DrawTextW(
                                mem_dc,
                                wide_glyph.as_ptr(),
                                glyph_len,
                                &mut shadow_rc,
                                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                            );

                            // 前景色：悬停金色，默认典雅银白
                            let color = if hover == btn_idx {
                                0x0000D7FF
                            } else {
                                0x00D0D0D0
                            };
                            SetTextColor(mem_dc, color);
                            DrawTextW(
                                mem_dc,
                                wide_glyph.as_ptr(),
                                glyph_len,
                                &mut btn_rc,
                                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                            );
                        }

                        SelectObject(mem_dc, old_btn_font);
                        DeleteObject(btn_font);

                        // 3. 绘制精致纵向分割线
                        let sep_brush = CreateSolidBrush(0x00404040);
                        let sep_rc = RECT {
                            left: dp(98),
                            top: dp(8),
                            right: dp(99),
                            bottom: h - dp(8),
                        };
                        FillRect(mem_dc, &sep_rc, sep_brush);
                        DeleteObject(sep_brush);
                    }
                    // 4. 绘制歌词文本
                    let font_name = to_wide("Microsoft YaHei");
                    let font = CreateFontW(
                        dp(16),
                        0,
                        0,
                        0,
                        FW_SEMIBOLD as i32,
                        0,
                        0,
                        0,
                        1, // DEFAULT_CHARSET
                        0,
                        0,
                        4, // ANTIALIASED_QUALITY
                        0,
                        font_name.as_ptr(),
                    );
                    let old_font = SelectObject(mem_dc, font);

                    let mut text_rect = RECT {
                        left: if controls { dp(104) } else { dp(8) },
                        top: 0,
                        right: w - dp(8),
                        bottom: h,
                    };

                    let wide_text = to_wide(&text);
                    if wide_text.len() > 1 {
                        let text_len = wide_text.len() as i32 - 1;

                        // 绘制深黑 4 邻域阴影，确保在任何壁纸与任务栏主题下清晰易读
                        SetTextColor(mem_dc, 0x00000000);
                        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (1, 1)] {
                            let mut shadow_rect = text_rect;
                            shadow_rect.left += dx;
                            shadow_rect.right += dx;
                            shadow_rect.top += dy;
                            shadow_rect.bottom += dy;
                            DrawTextW(
                                mem_dc,
                                wide_text.as_ptr(),
                                text_len,
                                &mut shadow_rect,
                                DT_LEFT
                                    | DT_VCENTER
                                    | DT_SINGLELINE
                                    | DT_NOPREFIX
                                    | DT_END_ELLIPSIS,
                            );
                        }

                        // 绘制正文字体：播放时金黄色 (0x0000D7FF)，暂停时银白色 (0x00E0E0E0)
                        let text_color = if is_paused { 0x00E0E0E0 } else { 0x0000D7FF };
                        SetTextColor(mem_dc, text_color);
                        DrawTextW(
                            mem_dc,
                            wide_text.as_ptr(),
                            text_len,
                            &mut text_rect,
                            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
                        );
                    }

                    // 双缓冲原子复制到前台屏幕，完全杜绝闪烁
                    BitBlt(hdc, 0, 0, w, h, mem_dc, 0, 0, SRCCOPY);

                    SelectObject(mem_dc, old_font);
                    DeleteObject(font);
                    SelectObject(mem_dc, old_bmp);
                    DeleteObject(mem_bmp);
                    DeleteDC(mem_dc);
                }

                EndPaint(hwnd, &ps);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe fn update_status(state: &mut TrayState) -> bool {
        let update = state.backend.take_update();
        if let Some(update) = update.as_ref().filter(|update| update.quit) {
            if update.result.is_ok() {
                return true;
            }
            // Keep the controls available if the daemon failed to shut down.
            state.quitting = false;
        }
        if let Some(update) = update {
            state.status_checked = Some(update.started);
            let status = if update.started.elapsed() < std::time::Duration::from_secs(2) {
                update.result.ok().flatten()
            } else {
                None
            };
            if let Some(st) = status {
                state.phase = st["phase"].as_str().unwrap_or("stopped").to_string();
                let track = &st["track"];
                state.current_title = track["title"].as_str().unwrap_or("未知歌曲").to_string();
                state.current_artist = track["artist"].as_str().unwrap_or("未知歌手").to_string();
                let pos = st["position"].as_f64().unwrap_or(0.0);
                let dur = st["duration"].as_f64().unwrap_or(0.0);
                let lyrics_text = st["lyrics"].as_str().unwrap_or("");
                let offset = st["lyric_offset"].as_i64().unwrap_or(0);
                state.current_volume = st["volume"].as_u64().unwrap_or(60) as u8;
                state.current_mode = st["mode"].as_str().unwrap_or("list").to_string();
                state.lyrics_bar_enabled = st["lyrics_bar"].as_bool().unwrap_or(true);
                state.boss = st["boss"].as_bool().unwrap_or(false);

                let active_lyr = active_lyric(lyrics_text, pos, offset);
                let display_text = if state.boss {
                    "wocli-music".to_string()
                } else if state.phase == "playing" {
                    active_lyr.unwrap_or_else(|| {
                        format!("{} · {}", state.current_title, state.current_artist)
                    })
                } else if state.phase == "paused" {
                    if let Some(lyr) = active_lyr {
                        format!("已暂停: {}", lyr)
                    } else {
                        format!("已暂停: {} · {}", state.current_title, state.current_artist)
                    }
                } else {
                    "wocli-music".to_string()
                };

                if display_text != state.current_text {
                    state.current_text = display_text;
                    if state.taskbar.hwnd != 0 {
                        InvalidateRect(state.taskbar.hwnd, null(), 0);
                    }
                }

                // 更新托盘 Tooltip（严格保证尾部 \0 结尾）
                let tip = if state.boss {
                    "wocli".to_string()
                } else {
                    format!(
                        "wocli: {} · {}\n{}  {} / {}\n{}",
                        state.current_title,
                        state.current_artist,
                        match state.phase.as_str() {
                            "playing" => "播放中",
                            "paused" => "已暂停",
                            "loading" => "加载中",
                            _ => "已停止",
                        },
                        clock(pos),
                        clock(dur),
                        state.current_text
                    )
                };
                set_tooltip(state, &tip);
            } else {
                clear_playback(state);
            }
        }
        if state
            .status_checked
            .is_some_and(|time| time.elapsed() >= std::time::Duration::from_secs(2))
        {
            clear_playback(state);
        }

        let visible = !state.boss
            && state.lyrics_bar_enabled
            && matches!(state.phase.as_str(), "playing" | "paused");
        PAINT_STATE.with(|snapshot| {
            *snapshot.borrow_mut() = (state.current_text.clone(), state.phase.clone());
        });
        state.taskbar.refresh(visible);
        if state.taskbar.hwnd != 0 {
            SetWindowTextW(
                state.taskbar.hwnd,
                to_wide(if visible {
                    &state.current_text
                } else {
                    "wocli"
                })
                .as_ptr(),
            );
        }
        false
    }

    unsafe fn clear_playback(state: &mut TrayState) {
        state.phase = "stopped".into();
        state.current_title.clear();
        state.current_artist.clear();
        state.current_text = "wocli-music".into();
        state.boss = false;
        state.status_checked = None;
        set_tooltip(state, "wocli-music · 后台未连接");
    }

    unsafe fn set_tooltip(state: &mut TrayState, text: &str) {
        let tip = super::tooltip_utf16::<128>(text);
        let changed = state.nid.szTip != tip;
        state.nid.szTip = tip;
        sync_notification_icon(&mut state.icon_present, changed, |operation| {
            Shell_NotifyIconW(operation, &state.nid) != 0
        });
    }

    fn sync_notification_icon(
        present: &mut bool,
        changed: bool,
        mut notify: impl FnMut(u32) -> bool,
    ) {
        if !*present {
            // Explorer can broadcast TaskbarCreated before accepting icons. Retry
            // on later timer ticks; MODIFY also handles an icon that survived a
            // transient failure of the previous request.
            *present = notify(NIM_ADD) || notify(NIM_MODIFY);
        } else if changed {
            *present = notify(NIM_MODIFY);
        }
    }

    unsafe fn show_menu(hwnd: HWND) {
        let hmenu = CreatePopupMenu();
        if hmenu == 0 {
            return;
        }

        STATE.with(|st| {
            if let Some(state) = st.borrow().as_ref() {
                let header = if state.boss {
                    "wocli".into()
                } else {
                    format!("{} · {}", state.current_title, state.current_artist)
                };
                AppendMenuW(
                    hmenu,
                    MF_STRING | MF_DISABLED,
                    CMD_HEADER,
                    to_wide(&header).as_ptr(),
                );

                let lyric_item = state.current_text.clone();
                AppendMenuW(
                    hmenu,
                    MF_STRING | MF_DISABLED,
                    CMD_LYRIC,
                    to_wide(&lyric_item).as_ptr(),
                );

                AppendMenuW(hmenu, MF_SEPARATOR, 0, null());

                let toggle_label = if state.phase == "playing" {
                    "⏸ 暂停"
                } else {
                    "▶ 播放"
                };
                AppendMenuW(hmenu, MF_STRING, CMD_TOGGLE, to_wide(toggle_label).as_ptr());
                AppendMenuW(hmenu, MF_STRING, CMD_PREV, to_wide("⏮ 上一首").as_ptr());
                AppendMenuW(hmenu, MF_STRING, CMD_NEXT, to_wide("⏭ 下一首").as_ptr());

                AppendMenuW(hmenu, MF_SEPARATOR, 0, null());

                let mode_str = format!("模式: {}", mode_label(&state.current_mode));
                AppendMenuW(hmenu, MF_STRING, CMD_MODE, to_wide(&mode_str).as_ptr());

                let vol_str = format!("音量: {}%（增加 10%）", state.current_volume);
                AppendMenuW(hmenu, MF_STRING, CMD_VOL_UP, to_wide(&vol_str).as_ptr());
                AppendMenuW(
                    hmenu,
                    MF_STRING,
                    CMD_VOL_DOWN,
                    to_wide("降低音量 10%").as_ptr(),
                );

                let lb_str = format!(
                    "任务栏歌词: {}",
                    if state.lyrics_bar_enabled {
                        "开启 (点击关闭)"
                    } else {
                        "关闭 (点击开启)"
                    }
                );
                AppendMenuW(hmenu, MF_STRING, CMD_LYRICS_BAR, to_wide(&lb_str).as_ptr());

                let boss_str = if state.boss {
                    "老板模式: 开启 (点击退出)"
                } else {
                    &state.boss_shortcut_label
                };
                AppendMenuW(hmenu, MF_STRING, CMD_BOSS, to_wide(boss_str).as_ptr());

                AppendMenuW(hmenu, MF_SEPARATOR, 0, null());

                AppendMenuW(
                    hmenu,
                    MF_STRING,
                    CMD_DAILY,
                    to_wide("播放每日推荐").as_ptr(),
                );
                AppendMenuW(hmenu, MF_STRING, CMD_TUI, to_wide("打开控制终端").as_ptr());
                AppendMenuW(hmenu, MF_STRING, CMD_QUIT, to_wide("退出播放器").as_ptr());
            }
        });

        let mut pt: POINT = std::mem::zeroed();
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        TrackPopupMenuEx(
            hmenu,
            TPM_RIGHTBUTTON | TPM_LEFTALIGN | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            hwnd,
            null(),
        );
        DestroyMenu(hmenu);
        PostMessageW(hwnd, WM_NULL, 0, 0);
    }

    fn queue_command(command: serde_json::Value) {
        STATE.with(|st| {
            if let Some(state) = st.borrow().as_ref() {
                if !state.quitting && !state.backend.send(command) {
                    eprintln!("托盘请求队列已满或后台工作线程已停止，请稍后重试");
                }
            }
        });
    }

    fn open_console() {
        let paths = STATE.with(|st| st.borrow().as_ref().map(|state| state.paths.clone()));
        if let Some(paths) = paths {
            if let Err(error) = ipc::spawn_console(&paths) {
                eprintln!("打开控制终端失败：{error}");
            }
        }
    }

    unsafe fn handle_command(_hwnd: HWND, cmd: usize) {
        if cmd == CMD_TUI {
            open_console();
            return;
        }
        STATE.with(|st| {
            if let Some(state) = st.borrow_mut().as_mut() {
                if state.quitting {
                    return;
                }
                let request = match cmd {
                    CMD_TOGGLE => json!({"op":"toggle"}),
                    CMD_PREV => json!({"op":"prev"}),
                    CMD_NEXT => json!({"op":"next"}),
                    CMD_MODE => json!({"op":"tray_mode"}),
                    CMD_VOL_UP => json!({"op":"tray_volume","delta":10}),
                    CMD_VOL_DOWN => json!({"op":"tray_volume","delta":-10}),
                    CMD_LYRICS_BAR => json!({"op":"tray_lyrics_bar"}),
                    CMD_BOSS => json!({"op":"boss"}),
                    CMD_DAILY => {
                        super::play_daily(state.paths.clone());
                        return;
                    }
                    CMD_QUIT => json!({"op":"shutdown"}),
                    _ => return,
                };
                if state.backend.send(request) {
                    // The worker asks the UI to close once shutdown completes.
                    // Never call tray::stop here: it would wait for / kill itself.
                    state.quitting = cmd == CMD_QUIT;
                } else {
                    eprintln!("托盘请求队列已满或后台工作线程已停止，请稍后重试");
                }
            }
        });
    }

    pub fn run_tray(paths: Paths) -> Result<()> {
        let settings = paths.settings()?;
        unsafe {
            let hinstance = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(null());
            let tray_class_name = to_wide("wocli_tray_class");
            let mut wc: WNDCLASSW = std::mem::zeroed();
            wc.lpfnWndProc = Some(tray_wnd_proc);
            wc.hInstance = hinstance;
            wc.lpszClassName = tray_class_name.as_ptr();
            RegisterClassW(&wc);

            let hwnd = CreateWindowExW(
                0,
                tray_class_name.as_ptr(),
                tray_class_name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                hinstance,
                null(),
            );
            if hwnd == 0 {
                return Err(Error::new("无法创建托盘宿主窗口"));
            }

            // 注册全局多媒体系统快捷键（播放/暂停、上一首、下一首、停止）及老板键（Ctrl+Shift+B / Ctrl+Alt+B）
            RegisterHotKey(hwnd, HOTKEY_PLAY_PAUSE, 0, VK_MEDIA_PLAY_PAUSE);
            RegisterHotKey(hwnd, HOTKEY_PREV, 0, VK_MEDIA_PREV_TRACK);
            RegisterHotKey(hwnd, HOTKEY_NEXT, 0, VK_MEDIA_NEXT_TRACK);
            RegisterHotKey(hwnd, HOTKEY_STOP, 0, VK_MEDIA_STOP);
            let mut shortcuts = vec![];
            for (id, modifiers, shortcut) in [
                (
                    HOTKEY_BOSS,
                    MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT,
                    "Ctrl+Shift+B",
                ),
                (
                    HOTKEY_BOSS_ALT,
                    MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                    "Ctrl+Alt+B",
                ),
            ] {
                let registered = RegisterHotKey(hwnd, id, modifiers, VK_KEY_B) != 0;
                let error_code = if registered { 0 } else { GetLastError() };
                shortcuts.push(
                    json!({"shortcut":shortcut,"registered":registered,"error_code":error_code}),
                );
            }
            let boss_shortcut_label = shortcuts
                .iter()
                .find(|v| v["registered"] == true)
                .and_then(|v| v["shortcut"].as_str())
                .map(|s| format!("老板模式 ({s})"))
                .unwrap_or_else(|| "老板模式 (快捷键不可用，点击切换)".into());
            super::record_hotkeys(&paths, shortcuts);

            // 创建系统托盘图标（动态生成金色音符图标）
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.uCallbackMessage = WM_TRAY;
            nid.hIcon = create_music_tray_icon();
            let tip = to_wide("wocli-music");
            nid.szTip[..tip.len().min(128)].copy_from_slice(&tip[..tip.len().min(128)]);
            let icon_present = Shell_NotifyIconW(NIM_ADD, &nid) != 0;

            // 注册任务栏歌词条窗口（支持双击）
            let lyrics_class_name = to_wide("wocli_lyrics_bar_class");
            let mut lwc: WNDCLASSW = std::mem::zeroed();
            lwc.style = CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS;
            lwc.lpfnWndProc = Some(lyrics_wnd_proc);
            lwc.hInstance = hinstance;
            lwc.lpszClassName = lyrics_class_name.as_ptr();
            RegisterClassW(&lwc);

            let taskbar = super::windows_taskbar::TaskbarLyrics::new(&paths, lyrics_class_name);
            let backend = super::windows_backend::Backend::start(paths.clone());
            STATE.with(|st| {
                *st.borrow_mut() = Some(TrayState {
                    paths: paths.clone(),
                    nid,
                    icon_present,
                    taskbar,
                    current_text: "wocli-music".into(),
                    current_title: "".into(),
                    current_artist: "".into(),
                    current_mode: settings.mode,
                    current_volume: settings.volume,
                    lyrics_bar_enabled: settings.lyrics_bar,
                    boss: false,
                    boss_shortcut_label,
                    phase: "stopped".into(),
                    backend,
                    status_checked: None,
                    quitting: false,
                });
            });

            // 200ms 刷新周期，平滑更新歌词并大幅降低 IPC 轮询负载
            SetTimer(hwnd, TIMER_ID, 200, None);

            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, 0, 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn fallback_icon_is_owned_and_can_be_destroyed_independently() {
            unsafe {
                let shared = LoadIconW(0, IDI_APPLICATION);
                let owned = fallback_tray_icon();
                assert_ne!(shared, 0);
                assert_ne!(owned, 0);
                assert_ne!(owned, shared);
                assert_ne!(DestroyIcon(owned), 0);
                assert_eq!(LoadIconW(0, IDI_APPLICATION), shared);
            }
        }

        #[test]
        fn notification_icon_retries_after_explorer_is_temporarily_unavailable() {
            let mut present = false;
            let mut operations = vec![];
            sync_notification_icon(&mut present, false, |operation| {
                operations.push(operation);
                false
            });
            assert!(!present);
            assert_eq!(operations, [NIM_ADD, NIM_MODIFY]);
            operations.clear();
            sync_notification_icon(&mut present, false, |operation| {
                operations.push(operation);
                true
            });
            assert!(present);
            assert_eq!(operations, [NIM_ADD]);
        }

        #[test]
        fn failed_tooltip_update_retries_even_when_the_text_has_not_changed() {
            let mut present = true;
            sync_notification_icon(&mut present, true, |_| false);
            assert!(!present);
            let mut operations = vec![];
            sync_notification_icon(&mut present, false, |operation| {
                operations.push(operation);
                operation == NIM_MODIFY
            });
            assert!(present);
            assert_eq!(operations, [NIM_ADD, NIM_MODIFY]);
            sync_notification_icon(&mut present, false, |_| panic!("already synchronized"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_daily_request_rejects_other_threads_and_allows_a_retry_after_completion() {
        let busy = std::sync::atomic::AtomicBool::new(false);
        let pending = DailyRequest::start(&busy).unwrap();
        std::thread::scope(|scope| {
            let attempts = (0..16)
                .map(|_| scope.spawn(|| DailyRequest::start(&busy).is_none()))
                .collect::<Vec<_>>();
            for attempt in attempts {
                assert!(attempt.join().unwrap());
            }
        });
        drop(pending);
        assert!(DailyRequest::start(&busy).is_some());
    }

    #[test]
    fn tray_tooltip_preserves_unicode_and_null_termination() {
        assert_eq!(
            String::from_utf16(&tooltip_utf16::<5>("歌词😀")[..4]).unwrap(),
            "歌词😀"
        );
        assert_eq!(
            tooltip_utf16::<4>("歌词😀"),
            ['歌' as u16, '词' as u16, 0, 0]
        );
        assert_eq!(tooltip_utf16::<3>("a\0b"), [97, 0, 0]);
        assert_eq!(tooltip_utf16::<1>("歌"), [0]);
        assert_eq!(tooltip_utf16::<0>("歌"), [0u16; 0]);
    }

    #[test]
    fn status_preserves_lock_shared_with_a_concurrent_starter() {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: home.path().into(),
            runtime: home.path().into(),
        };
        let starter = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(lock_path(&paths))
            .unwrap();
        assert!(!status(&paths).unwrap());
        starter.try_lock_exclusive().unwrap();
        assert!(status(&paths).unwrap());
        drop(starter);
        assert!(!status(&paths).unwrap());
    }

    #[test]
    fn test_active_lyric_parser() {
        let lrc = "[00:00.00]前奏\n[00:05.50]第一行歌词\n[00:10.00]第二行歌词";
        assert_eq!(active_lyric(lrc, 2.0, 0), Some("前奏".into()));
        assert_eq!(active_lyric(lrc, 7.0, 0), Some("第一行歌词".into()));
        assert_eq!(active_lyric(lrc, 12.0, 0), Some("第二行歌词".into()));
        assert_eq!(active_lyric("", 5.0, 0), None);
    }
}
