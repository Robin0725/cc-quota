//! Focused-window attribution: the window the user just clicked names the assistant they are
//! looking at.
//!
//! The frontmost *application* usually cannot answer which assistant is in use — every CLI runs
//! in the same terminal — but the focused window's *title* usually can, because terminals title
//! their windows after the running command ("claude", "kimi", …). Reading a window title needs
//! the macOS Accessibility permission. The tray menu can request it, and each exact ad-hoc build
//! repairs its own stale row once on its first untrusted launch; without a valid grant everything here
//! reports `None` and the caller falls back to the prompt-history signal.
//!
//! Privacy rules, same spirit as [`crate::providers::activity`]:
//!
//! * A window title may contain project names or document paths. It is compared against provider
//!   hints **in memory only** — never logged, never stored, never sent anywhere, and never
//!   returned beyond this crate except as a provider id.
//! * No polling loop lives here; the caller asks when it already needed the frontmost app.

#[cfg(target_os = "macos")]
mod macos {
    use core_foundation::{
        base::{CFGetTypeID, CFRelease, CFTypeRef, TCFType},
        boolean::CFBoolean,
        dictionary::CFDictionary,
        string::{CFString, CFStringRef},
    };
    use std::{
        ffi::c_void,
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    type AXUIElementRef = *const c_void;
    type AXError = i32;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> u8;
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> u8;
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> AXError;
        static kAXTrustedCheckOptionPrompt: CFStringRef;
    }

    pub fn trusted() -> bool {
        unsafe { AXIsProcessTrusted() != 0 }
    }

    /// Shows the system's Accessibility prompt (which routes to System Settings) unless the
    /// permission is already granted. Granting may only take effect after an app restart; that is
    /// the system's behaviour, not ours.
    pub fn request_trust() {
        unsafe {
            let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
            let options = CFDictionary::from_CFType_pairs(&[(
                key.as_CFType(),
                CFBoolean::true_value().as_CFType(),
            )]);
            AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef() as *const c_void);
        }
    }

    /// Removes only this app's stale Accessibility decision. Updates distributed with an ad-hoc
    /// signature have a different designated requirement even though their bundle id is stable,
    /// so toggling the old System Settings row cannot make the new binary trusted. Resetting the
    /// bundle-scoped decision makes the immediately following `request_trust` create a fresh row.
    pub fn reset_trust() -> Result<(), String> {
        let mut child = Command::new("/usr/bin/tccutil")
            .args(["reset", "Accessibility", crate::APP_BUNDLE_ID])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "failed to start the macOS privacy reset".to_string())?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(_)) => return Err("macOS rejected the Accessibility reset".into()),
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("the macOS Accessibility reset timed out".into());
                }
            }
        }
    }

    /// One attribute read on one element, releasing the element's borrow of nothing — the caller
    /// owns both the element and, on success, the returned value.
    unsafe fn copy_attribute(element: AXUIElementRef, name: &'static str) -> Option<CFTypeRef> {
        let attribute = CFString::from_static_string(name);
        let mut value: CFTypeRef = std::ptr::null();
        let error =
            AXUIElementCopyAttributeValue(element, attribute.as_concrete_TypeRef(), &mut value);
        (error == 0 && !value.is_null()).then_some(value)
    }

    /// The title of the application's focused window, or `None` without the permission, without a
    /// focused window, or for a window that reports no string title.
    pub fn focused_window_title(pid: i32) -> Option<String> {
        if !trusted() {
            return None;
        }
        unsafe {
            let application = AXUIElementCreateApplication(pid);
            if application.is_null() {
                return None;
            }
            let window = copy_attribute(application, "AXFocusedWindow");
            CFRelease(application as CFTypeRef);
            let window = window?;
            let title = copy_attribute(window as AXUIElementRef, "AXTitle");
            CFRelease(window);
            let title = title?;
            if CFGetTypeID(title) != CFString::type_id() {
                CFRelease(title);
                return None;
            }
            Some(CFString::wrap_under_create_rule(title as CFStringRef).to_string())
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos::{focused_window_title, request_trust, reset_trust, trusted};

#[cfg(not(target_os = "macos"))]
pub fn trusted() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn request_trust() {}

#[cfg(not(target_os = "macos"))]
pub fn reset_trust() -> Result<(), String> {
    Err("Accessibility recovery is only available on macOS".into())
}

#[cfg(not(target_os = "macos"))]
pub fn focused_window_title(_pid: i32) -> Option<String> {
    None
}
