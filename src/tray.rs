#![allow(unexpected_cfgs)]
#![allow(unused_imports)]

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
        if status(paths)? {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok(())
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
    const TAG_TUI: isize = 17;
    const TAG_QUIT: isize = 99;

    extern "C" fn on_menu_action(_this: &Object, _cmd: Sel, sender: id) {
        unsafe {
            let tag: isize = msg_send![sender, tag];
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
            }
            decl.register();
        });
        class!(WocliTrayActionTarget)
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

                    let new_info = format!("{title} · {artist}");
                    if new_info != last_menu_info {
                        set_item_title(item_info, &new_info);
                        last_menu_info = new_info;
                    }

                    let new_status = format!("{phase_label}  {} / {}", clock(pos), clock(dur));
                    if new_status != last_menu_status {
                        set_item_title(item_status, &new_status);
                        last_menu_status = new_status;
                    }

                    let new_lyric = active_lyr.as_deref().unwrap_or("暂无歌词");
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

                    // 顶部菜单栏标题：实时呈现当前唱到的歌词（未唱到歌词或间奏时显示歌名·歌手）
                    let menu_bar_title = if phase == "playing" {
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
                    let bar_display = if !lyrics_bar_enabled {
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
                } else {
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
    use std::sync::atomic::{AtomicU32, Ordering};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW,
        CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, InvalidateRect,
        SelectObject, SetBkMode, SetTextColor, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX,
        DT_SINGLELINE, DT_VCENTER, FW_BOLD, PAINTSTRUCT, SRCCOPY, TRANSPARENT,
    };
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
        NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        DispatchMessageW, GetClientRect, GetCursorPos, GetMessageW, GetSystemMetrics, LoadIconW,
        PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SendMessageW, SetForegroundWindow,
        SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, SystemParametersInfoW,
        TrackPopupMenuEx, TranslateMessage, CS_HREDRAW, CS_VREDRAW, IDI_APPLICATION, LWA_COLORKEY,
        MF_DISABLED, MF_SEPARATOR, MF_STRING, MSG, SM_CXSCREEN, SM_CYSCREEN, SPI_GETWORKAREA,
        SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
        TPM_RIGHTBUTTON, WM_COMMAND, WM_DESTROY, WM_DISPLAYCHANGE, WM_ERASEBKGND, WM_LBUTTONUP,
        WM_PAINT, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER, WM_USER, WNDCLASSW, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
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
    const CMD_TUI: usize = 209;
    const CMD_QUIT: usize = 210;

    struct TrayState {
        paths: Paths,
        nid: NOTIFYICONDATAW,
        lyrics_hwnd: HWND,
        current_text: String,
        current_title: String,
        current_artist: String,
        current_mode: String,
        current_volume: u8,
        lyrics_bar_enabled: bool,
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

    unsafe fn reposition_lyrics_bar(hwnd: HWND) {
        let mut work_area: RECT = std::mem::zeroed();
        let has_work_area =
            SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work_area as *mut _ as *mut _, 0) != 0;
        let (screen_w, work_bottom) = if has_work_area {
            (
                (work_area.right - work_area.left).max(800),
                work_area.bottom,
            )
        } else {
            let sw = GetSystemMetrics(SM_CXSCREEN).max(800);
            let sh = GetSystemMetrics(SM_CYSCREEN);
            (sw, sh - 48)
        };
        let bar_w = 920.min(screen_w - 40);
        let bar_h = 46;
        let bar_x = (screen_w - bar_w) / 2;
        let bar_y = (work_bottom - bar_h - 8).max(0);

        SetWindowPos(
            hwnd,
            -1isize as HWND,
            bar_x,
            bar_y,
            bar_w,
            bar_h,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
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
                let cmd = (wparam & 0xFFFF) as usize;
                handle_command(hwnd, cmd);
                SendMessageW(hwnd, WM_TIMER, TIMER_ID, 0);
                0
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                STATE.with(|st| {
                    if let Some(state) = st.borrow().as_ref() {
                        if state.lyrics_hwnd != 0 {
                            reposition_lyrics_bar(state.lyrics_hwnd);
                        }
                    }
                });
                0
            }
            WM_DESTROY => {
                STATE.with(|st| {
                    if let Some(mut state) = st.borrow_mut().take() {
                        Shell_NotifyIconW(NIM_DELETE, &mut state.nid);
                        if state.lyrics_hwnd != 0 {
                            DestroyWindow(state.lyrics_hwnd);
                        }
                    }
                });
                PostQuitMessage(0);
                0
            }
            _ => {
                if msg == taskbar_created_msg() {
                    STATE.with(|st| {
                        if let Some(state) = st.borrow_mut().as_mut() {
                            Shell_NotifyIconW(NIM_ADD, &mut state.nid);
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

                    let font_name = to_wide("Microsoft YaHei");
                    let font = CreateFontW(
                        28,
                        0,
                        0,
                        0,
                        FW_BOLD as i32,
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

                    let (text, is_paused) = STATE.with(|st| {
                        let borrowed = st.borrow();
                        let s = borrowed.as_ref();
                        let txt = s.map(|x| x.current_text.clone()).unwrap_or_default();
                        let paused = s.map(|x| x.phase.as_str() == "paused").unwrap_or(false);
                        (txt, paused)
                    });
                    let wide_text = to_wide(&text);

                    if wide_text.len() > 1 {
                        let text_len = wide_text.len() as i32 - 1;

                        // 绘制深黑 8 邻域阴影，确保在任何桌面壁纸下清晰易读
                        SetTextColor(mem_dc, 0x00000000);
                        for (dx, dy) in [
                            (-1, -1),
                            (0, -1),
                            (1, -1),
                            (-1, 0),
                            (1, 0),
                            (-1, 1),
                            (0, 1),
                            (1, 1),
                            (0, 2),
                            (1, 2),
                            (-1, 2),
                        ] {
                            let mut shadow_rect = rect;
                            shadow_rect.left += dx;
                            shadow_rect.right += dx;
                            shadow_rect.top += dy;
                            shadow_rect.bottom += dy;
                            DrawTextW(
                                mem_dc,
                                wide_text.as_ptr(),
                                text_len,
                                &mut shadow_rect,
                                DT_CENTER
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
                            &mut rect,
                            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
                        );
                    }

                    // 双缓冲原子复制到前台屏幕，杜绝闪烁
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

            let active_lyr = active_lyric(lyrics_text, pos, offset);
            let display_text = if state.phase == "playing" {
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
                if state.lyrics_hwnd != 0 {
                    InvalidateRect(state.lyrics_hwnd, null(), 0);
                }
            }

            // 更新托盘 Tooltip（严格保证尾部 \0 结尾）
            let tip = format!(
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
            );
            let wide_tip = to_wide(&tip);
            let buf_len = state.nid.szTip.len();
            let copy_len = (buf_len - 1).min(wide_tip.len());
            state.nid.szTip.fill(0);
            state.nid.szTip[..copy_len].copy_from_slice(&wide_tip[..copy_len]);
            state.nid.szTip[buf_len - 1] = 0;
            Shell_NotifyIconW(NIM_MODIFY, &mut state.nid);

            // 控制任务栏歌词条显隐
            if state.lyrics_hwnd != 0 {
                if state.lyrics_bar_enabled && (state.phase == "playing" || state.phase == "paused")
                {
                    ShowWindow(state.lyrics_hwnd, SW_SHOWNOACTIVATE);
                } else {
                    ShowWindow(state.lyrics_hwnd, SW_HIDE);
                }
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
                let header = format!("{} · {}", state.current_title, state.current_artist);
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
                    "暂停"
                } else {
                    "播放"
                };
                AppendMenuW(hmenu, MF_STRING, CMD_TOGGLE, to_wide(toggle_label).as_ptr());
                AppendMenuW(hmenu, MF_STRING, CMD_PREV, to_wide("上一首").as_ptr());
                AppendMenuW(hmenu, MF_STRING, CMD_NEXT, to_wide("下一首").as_ptr());

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
            let hinstance = 0;
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

            // 创建系统托盘图标
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.uCallbackMessage = WM_TRAY;
            nid.hIcon = LoadIconW(0, IDI_APPLICATION);
            let tip = to_wide("wocli-music");
            nid.szTip[..tip.len().min(128)].copy_from_slice(&tip[..tip.len().min(128)]);
            Shell_NotifyIconW(NIM_ADD, &mut nid);

            // 注册任务栏歌词条窗口
            let lyrics_class_name = to_wide("wocli_lyrics_bar_class");
            let mut lwc: WNDCLASSW = std::mem::zeroed();
            lwc.style = CS_HREDRAW | CS_VREDRAW;
            lwc.lpfnWndProc = Some(lyrics_wnd_proc);
            lwc.hInstance = hinstance;
            lwc.lpszClassName = lyrics_class_name.as_ptr();
            RegisterClassW(&lwc);

            let mut work_area: RECT = std::mem::zeroed();
            let has_work_area =
                SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work_area as *mut _ as *mut _, 0)
                    != 0;
            let (screen_w, work_bottom) = if has_work_area {
                (
                    (work_area.right - work_area.left).max(800),
                    work_area.bottom,
                )
            } else {
                let sw = GetSystemMetrics(SM_CXSCREEN).max(800);
                let sh = GetSystemMetrics(SM_CYSCREEN);
                (sw, sh - 48)
            };
            let bar_w = 920.min(screen_w - 40);
            let bar_h = 46;
            let bar_x = (screen_w - bar_w) / 2;
            let bar_y = (work_bottom - bar_h - 8).max(0);

            let lyrics_hwnd = CreateWindowExW(
                WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE,
                lyrics_class_name.as_ptr(),
                lyrics_class_name.as_ptr(),
                WS_POPUP,
                bar_x,
                bar_y,
                bar_w,
                bar_h,
                0,
                0,
                hinstance,
                null(),
            );
            if lyrics_hwnd != 0 {
                SetLayeredWindowAttributes(lyrics_hwnd, 0x000A0A0A, 0, LWA_COLORKEY);
            }

            STATE.with(|st| {
                *st.borrow_mut() = Some(TrayState {
                    paths: paths.clone(),
                    nid,
                    lyrics_hwnd,
                    current_text: "wocli-music".into(),
                    current_title: "".into(),
                    current_artist: "".into(),
                    current_mode: "list".into(),
                    current_volume: 60,
                    lyrics_bar_enabled: true,
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
