//! Carbon global shortcut registration, owned by the tray's main Cocoa thread.
use std::{
    ffi::c_void,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};

type Ref = *mut c_void;
#[repr(C)]
struct EventType {
    class: u32,
    kind: u32,
}
#[repr(C)]
#[derive(Default)]
struct HotkeyId {
    signature: u32,
    id: u32,
}
const SIGNATURE: u32 = u32::from_be_bytes(*b"wocl");
static KEY_DOWN: AtomicBool = AtomicBool::new(false);

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn GetApplicationEventTarget() -> Ref;
    fn InstallEventHandler(
        target: Ref,
        handler: extern "C" fn(Ref, Ref, Ref) -> i32,
        count: usize,
        types: *const EventType,
        data: Ref,
        out: *mut Ref,
    ) -> i32;
    fn RemoveEventHandler(handler: Ref) -> i32;
    fn RegisterEventHotKey(
        code: u32,
        modifiers: u32,
        id: HotkeyId,
        target: Ref,
        options: u32,
        out: *mut Ref,
    ) -> i32;
    fn UnregisterEventHotKey(key: Ref) -> i32;
    fn GetEventParameter(
        event: Ref,
        name: u32,
        kind: u32,
        actual_type: *mut u32,
        size: usize,
        actual_size: *mut usize,
        data: Ref,
    ) -> i32;
    fn GetEventKind(event: Ref) -> u32;
}

extern "C" fn pressed(_: Ref, event: Ref, _: Ref) -> i32 {
    let mut id = HotkeyId::default();
    unsafe {
        let status = GetEventParameter(
            event,
            u32::from_be_bytes(*b"----"),
            u32::from_be_bytes(*b"hkid"),
            ptr::null_mut(),
            size_of::<HotkeyId>(),
            ptr::null_mut(),
            (&mut id as *mut HotkeyId).cast(),
        );
        if status == 0 && id.signature == SIGNATURE && id.id == 1 {
            if GetEventKind(event) == 6 {
                KEY_DOWN.store(false, Ordering::Relaxed);
            } else if !KEY_DOWN.swap(true, Ordering::Relaxed) {
                super::macos::boss_shortcut_pressed();
            }
            0
        } else {
            -9874
        } // eventNotHandledErr: preserve unrelated application events.
    }
}

pub(super) struct BossHotkey {
    key: Ref,
    handler: Ref,
}
impl BossHotkey {
    /// Call only from the main thread after NSApplication has launched.
    pub(super) fn register() -> std::result::Result<Self, i32> {
        let mut result = Self {
            key: ptr::null_mut(),
            handler: ptr::null_mut(),
        };
        unsafe {
            let target = GetApplicationEventTarget();
            let events = [
                EventType {
                    class: u32::from_be_bytes(*b"keyb"),
                    kind: 5,
                },
                EventType {
                    class: u32::from_be_bytes(*b"keyb"),
                    kind: 6,
                },
            ];
            let status = InstallEventHandler(
                target,
                pressed,
                events.len(),
                events.as_ptr(),
                ptr::null_mut(),
                &mut result.handler,
            );
            if status != 0 {
                return Err(status);
            }
            // kVK_ANSI_B, controlKey | shiftKey. A plain Shift+B remains ordinary typing.
            let status = RegisterEventHotKey(
                0x0b,
                (1 << 12) | (1 << 9),
                HotkeyId {
                    signature: SIGNATURE,
                    id: 1,
                },
                target,
                0,
                &mut result.key,
            );
            if status != 0 {
                return Err(status);
            }
        }
        Ok(result)
    }
}
impl Drop for BossHotkey {
    fn drop(&mut self) {
        unsafe {
            if !self.key.is_null() {
                UnregisterEventHotKey(self.key);
            }
            if !self.handler.is_null() {
                RemoveEventHandler(self.handler);
            }
        }
    }
}
