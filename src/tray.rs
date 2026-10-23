#![allow(unexpected_cfgs)]
#![allow(unused_imports)]

#[cfg(target_os = "macos")]
mod macos_hotkey;
#[cfg(any(windows, test))]
#[path = "tray/taskbar_layout.rs"]
mod taskbar_layout;
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
        let _ = fs::remove_file(&lock_file);
        let _ = fs::remove_file(pid_path(paths));
        Ok(false)
    } else {
        Ok(true)
    }
}

pub fn stop(paths: &Paths) -> Result<()> {
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
                        use windows_sys::Win32::UI::WindowsAndMessaging::{
                            FindWindowW, PostMessageW, WM_CLOSE,
                        };
                        let class_name: Vec<u16> = "wocli_tray_class"
                            .encode_utf16()
                            .chain(std::iter::once(0))
                            .collect();
                        let hwnd = FindWindowW(class_name.as_ptr(), std::ptr::null());
                        if hwnd != 0 {
                            PostMessageW(hwnd, WM_CLOSE, 0, 0);
                            std::thread::sleep(std::time::Duration::from_millis(150));
                        }
                    }
                    let _ = Command::new("taskkill")
                        .args(["/F", "/PID", &pid.to_string()])
                        .output();
                }
            }
        }
    }
    let lock_file = lock_path(paths);
    let _ = fs::remove_file(pid_file);
    let _ = fs::remove_file(lock_file);
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

            // 1. 顶部菜单栏 StatusItem
            let status_bar = NSStatusBar::systemStatusBar(nil);
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

            let item_boss = create_item(boss_menu_label, TAG_BOSS, target_obj);
            let _: () = msg_send![menu, addItem: item_boss];

            let sep: id = msg_send![class!(NSMenuItem), separatorItem];
            let _: () = msg_send![menu, addItem: sep];

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
    use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
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
        AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
        DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GetClientRect, GetCursorPos,
        GetMessageW, LoadCursorW, LoadIconW, PostQuitMessage, RegisterClassW,
        RegisterWindowMessageW, SendMessageW, SetCursor, SetForegroundWindow, SetTimer,
        SetWindowTextW, TrackPopupMenuEx, TranslateMessage, WindowFromPoint, CS_DBLCLKS,
        CS_HREDRAW, CS_VREDRAW, HICON, HTCLIENT, ICONINFO, IDC_HAND, IDI_APPLICATION,
        MA_NOACTIVATE, MF_DISABLED, MF_SEPARATOR, MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
        TPM_RIGHTBUTTON, WM_COMMAND, WM_DESTROY, WM_DISPLAYCHANGE, WM_ERASEBKGND, WM_HOTKEY,
        WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE,
        WM_NCHITTEST, WM_NCRBUTTONUP, WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR, WM_SETTINGCHANGE,
        WM_TIMER, WM_USER, WNDCLASSW,
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

    struct TrayState {
        paths: Paths,
        nid: NOTIFYICONDATAW,
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
    }

    thread_local! {
        static STATE: RefCell<Option<TrayState>> = const { RefCell::new(None) };
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
            return LoadIconW(0, IDI_APPLICATION);
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
            LoadIconW(0, IDI_APPLICATION)
        }
    }

    fn button_at(hwnd: HWND, x: i32) -> u8 {
        unsafe {
            let mut area: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut area);
            let dpi = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd);
            let dp = |v| super::taskbar_layout::pixels(v, dpi);
            if area.right < dp(260) {
                return 0;
            }
            if (dp(6)..=dp(34)).contains(&x) {
                1
            } else if (dp(36)..=dp(64)).contains(&x) {
                2
            } else if (dp(66)..=dp(94)).contains(&x) {
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
                STATE.with(|st| {
                    if let Some(state) = st.borrow_mut().as_mut() {
                        update_status(state);

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
                });
                0
            }
            WM_TRAY => {
                let event = lparam as u32;
                if event == WM_LBUTTONUP {
                    STATE.with(|st| {
                        if let Some(state) = st.borrow().as_ref() {
                            let _ = ipc::request(&state.paths, json!({"op":"toggle"}), true);
                        }
                    });
                    SendMessageW(hwnd, WM_TIMER, TIMER_ID, 0);
                } else if event == WM_RBUTTONUP {
                    show_menu(hwnd);
                }
                0
            }
            WM_COMMAND => {
                let cmd = wparam & 0xFFFF;
                handle_command(hwnd, cmd);
                SendMessageW(hwnd, WM_TIMER, TIMER_ID, 0);
                0
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                STATE.with(|st| {
                    if let Some(state) = st.borrow_mut().as_mut() {
                        state.taskbar.reset();
                    }
                });
                0
            }
            WM_HOTKEY => {
                let id = wparam as i32;
                STATE.with(|st| {
                    if let Some(state) = st.borrow().as_ref() {
                        match id {
                            HOTKEY_PLAY_PAUSE => {
                                let _ = ipc::request(&state.paths, json!({"op":"toggle"}), true);
                            }
                            HOTKEY_PREV => {
                                let _ = ipc::request(&state.paths, json!({"op":"prev"}), true);
                            }
                            HOTKEY_NEXT => {
                                let _ = ipc::request(&state.paths, json!({"op":"next"}), true);
                            }
                            HOTKEY_STOP => {
                                let _ = ipc::request(&state.paths, json!({"op":"stop"}), true);
                            }
                            HOTKEY_BOSS | HOTKEY_BOSS_ALT => {
                                let _ = ipc::request(&state.paths, json!({"op":"boss"}), true);
                            }
                            _ => {}
                        }
                    }
                });
                SendMessageW(hwnd, WM_TIMER, TIMER_ID, 0);
                0
            }
            WM_DESTROY => {
                UnregisterHotKey(hwnd, HOTKEY_PLAY_PAUSE);
                UnregisterHotKey(hwnd, HOTKEY_PREV);
                UnregisterHotKey(hwnd, HOTKEY_NEXT);
                UnregisterHotKey(hwnd, HOTKEY_STOP);
                UnregisterHotKey(hwnd, HOTKEY_BOSS);
                UnregisterHotKey(hwnd, HOTKEY_BOSS_ALT);
                STATE.with(|st| {
                    if let Some(mut state) = st.borrow_mut().take() {
                        Shell_NotifyIconW(NIM_DELETE, &state.nid);
                        if state.nid.hIcon != 0 {
                            DestroyIcon(state.nid.hIcon);
                        }
                        state.taskbar.shutdown();
                    }
                });
                PostQuitMessage(0);
                0
            }
            _ => {
                if msg == taskbar_created_msg() {
                    STATE.with(|st| {
                        if let Some(state) = st.borrow_mut().as_mut() {
                            Shell_NotifyIconW(NIM_ADD, &state.nid);
                            state.taskbar.reset();
                        }
                    });
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
                if button_at(hwnd, point.x) != 0 {
                    SetCursor(LoadCursorW(0, IDC_HAND));
                    1
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            WM_MOUSEMOVE => {
                let x = (lparam & 0xFFFF) as i16 as i32;
                let new_hover = button_at(hwnd, x);
                let old_hover = HOVER_BTN.swap(new_hover, Ordering::Relaxed);
                if old_hover != new_hover {
                    InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_LBUTTONDOWN => {
                let x = (lparam & 0xFFFF) as i16 as i32;
                STATE.with(|st| {
                    if let Some(state) = st.borrow().as_ref() {
                        if button_at(hwnd, x) == 1 {
                            let _ = ipc::request(&state.paths, json!({"op":"prev"}), true);
                        } else if button_at(hwnd, x) == 2 {
                            let _ = ipc::request(&state.paths, json!({"op":"toggle"}), true);
                        } else if button_at(hwnd, x) == 3 {
                            let _ = ipc::request(&state.paths, json!({"op":"next"}), true);
                        }
                    }
                });
                0
            }
            WM_NCRBUTTONUP | WM_RBUTTONUP => {
                STATE.with(|st| {
                    if let Some(state) = st.borrow().as_ref() {
                        show_menu(state.nid.hWnd);
                    }
                });
                0
            }
            WM_LBUTTONDBLCLK => {
                STATE.with(|st| {
                    if st.borrow().is_some() {
                        let exe =
                            std::env::current_exe().unwrap_or_else(|_| PathBuf::from("wocli.exe"));
                        let exe_str = exe.to_string_lossy();
                        let _ = Command::new("cmd")
                            .args(["/c", "start", "wocli-music", &*exe_str])
                            .spawn();
                    }
                });
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

                    let (text, is_paused, phase) = STATE.with(|st| {
                        let borrowed = st.borrow();
                        let s = borrowed.as_ref();
                        let txt = s.map(|x| x.current_text.clone()).unwrap_or_default();
                        let paused = s.map(|x| x.phase.as_str() == "paused").unwrap_or(false);
                        let ph = s.map(|x| x.phase.clone()).unwrap_or_default();
                        (txt, paused, ph)
                    });

                    let hover = HOVER_BTN.load(Ordering::Relaxed);

                    let dp = |v| {
                        super::taskbar_layout::pixels(
                            v,
                            windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd),
                        )
                    };
                    let controls = w >= dp(260);
                    if controls {
                        // 1. 绘制控制按钮背景（悬停浅底）
                        let hover_brush = CreateSolidBrush(0x00282828);
                        if hover == 1 {
                            let btn_rc = RECT {
                                left: dp(6),
                                top: dp(4),
                                right: dp(34),
                                bottom: h - dp(4),
                            };
                            FillRect(mem_dc, &btn_rc, hover_brush);
                        } else if hover == 2 {
                            let btn_rc = RECT {
                                left: dp(36),
                                top: dp(4),
                                right: dp(64),
                                bottom: h - dp(4),
                            };
                            FillRect(mem_dc, &btn_rc, hover_brush);
                        } else if hover == 3 {
                            let btn_rc = RECT {
                                left: dp(66),
                                top: dp(4),
                                right: dp(94),
                                bottom: h - dp(4),
                            };
                            FillRect(mem_dc, &btn_rc, hover_brush);
                        }
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
                            5,
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
                        5, // CLEARTYPE_QUALITY
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

    unsafe fn update_status(state: &mut TrayState) {
        if let Ok(Some(st)) = ipc::request(&state.paths, json!({"op":"status"}), false) {
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
            let wide_tip = to_wide(&tip);
            let buf_len = state.nid.szTip.len();
            let copy_len = (buf_len - 1).min(wide_tip.len());
            state.nid.szTip.fill(0);
            state.nid.szTip[..copy_len].copy_from_slice(&wide_tip[..copy_len]);
            state.nid.szTip[buf_len - 1] = 0;
            Shell_NotifyIconW(NIM_MODIFY, &state.nid);

            let visible = !state.boss
                && state.lyrics_bar_enabled
                && matches!(state.phase.as_str(), "playing" | "paused");
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
        } else {
            state.taskbar.refresh(false);
            if state.taskbar.hwnd != 0 {
                SetWindowTextW(state.taskbar.hwnd, to_wide("wocli").as_ptr());
            }
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

                let vol_str = format!("音量: {}% (点击 +10%)", state.current_volume);
                AppendMenuW(hmenu, MF_STRING, CMD_VOL_UP, to_wide(&vol_str).as_ptr());

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
    }

    unsafe fn handle_command(hwnd: HWND, cmd: usize) {
        STATE.with(|st| {
            if let Some(state) = st.borrow_mut().as_mut() {
                match cmd {
                    CMD_TOGGLE => {
                        let _ = ipc::request(&state.paths, json!({"op":"toggle"}), true);
                    }
                    CMD_PREV => {
                        let _ = ipc::request(&state.paths, json!({"op":"prev"}), true);
                    }
                    CMD_NEXT => {
                        let _ = ipc::request(&state.paths, json!({"op":"next"}), true);
                    }
                    CMD_MODE => {
                        let next_mode = match state.current_mode.as_str() {
                            "list" => "repeat",
                            "repeat" => "single",
                            "single" => "shuffle",
                            _ => "list",
                        };
                        let _ = ipc::request(
                            &state.paths,
                            json!({"op":"config","key":"mode","value":next_mode}),
                            true,
                        );
                    }
                    CMD_VOL_UP => {
                        let next_vol = if state.current_volume >= 100 {
                            20
                        } else {
                            (state.current_volume + 10).min(100)
                        };
                        let _ = ipc::request(
                            &state.paths,
                            json!({"op":"config","key":"volume","value":next_vol}),
                            true,
                        );
                    }
                    CMD_LYRICS_BAR => {
                        let next_lb = !state.lyrics_bar_enabled;
                        let _ = ipc::request(
                            &state.paths,
                            json!({"op":"config","key":"lyrics_bar","value":next_lb}),
                            true,
                        );
                    }
                    CMD_BOSS => {
                        let _ = ipc::request(&state.paths, json!({"op":"boss"}), true);
                    }
                    CMD_TUI => {
                        let exe =
                            std::env::current_exe().unwrap_or_else(|_| PathBuf::from("wocli.exe"));
                        let exe_str = exe.to_string_lossy();
                        let _ = Command::new("cmd")
                            .args(["/c", "start", "wocli-music", &*exe_str])
                            .spawn();
                    }
                    CMD_QUIT => {
                        let _ = super::stop(&state.paths);
                        let _ = ipc::request(&state.paths, json!({"op":"shutdown"}), false);
                        DestroyWindow(hwnd);
                    }
                    _ => {}
                }
            }
        });
    }

    pub fn run_tray(paths: Paths) -> Result<()> {
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
            Shell_NotifyIconW(NIM_ADD, &nid);

            // 注册任务栏歌词条窗口（支持双击）
            let lyrics_class_name = to_wide("wocli_lyrics_bar_class");
            let mut lwc: WNDCLASSW = std::mem::zeroed();
            lwc.style = CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS;
            lwc.lpfnWndProc = Some(lyrics_wnd_proc);
            lwc.hInstance = hinstance;
            lwc.lpszClassName = lyrics_class_name.as_ptr();
            RegisterClassW(&lwc);

            let taskbar = super::windows_taskbar::TaskbarLyrics::new(&paths, lyrics_class_name);
            STATE.with(|st| {
                *st.borrow_mut() = Some(TrayState {
                    paths: paths.clone(),
                    nid,
                    taskbar,
                    current_text: "wocli-music".into(),
                    current_title: "".into(),
                    current_artist: "".into(),
                    current_mode: "list".into(),
                    current_volume: 60,
                    lyrics_bar_enabled: true,
                    boss: false,
                    boss_shortcut_label,
                    phase: "stopped".into(),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_active_lyric_parser() {
        let lrc = "[00:00.00]前奏\n[00:05.50]第一行歌词\n[00:10.00]第二行歌词";
        assert_eq!(active_lyric(lrc, 2.0, 0), Some("前奏".into()));
        assert_eq!(active_lyric(lrc, 7.0, 0), Some("第一行歌词".into()));
        assert_eq!(active_lyric(lrc, 12.0, 0), Some("第二行歌词".into()));
        assert_eq!(active_lyric("", 5.0, 0), None);
    }
}
