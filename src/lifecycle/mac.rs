//! The app's question in front of AppKit's `applicationShouldTerminate:`. Rune adds that method to
//! winit's app delegate when its event loop starts, and it starts Rune's quit. This swaps in a
//! method that holds the quit (`NSTerminateCancel`) and asks the captain while work is in flight,
//! and otherwise calls Rune's, so every quit Rune would see still reaches it.

use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

use objc2::runtime::{AnyClass, AnyObject, Bool, Sel};
use objc2::{msg_send, sel};
use objc2_app_kit::NSApplication;
use objc2_foundation::MainThreadMarker;

/// `NSTerminateCancel` and `NSTerminateNow`.
const CANCEL: usize = 0;
const NOW: usize = 1;

type Terminate = extern "C" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize;

/// Rune's method, which a quit that goes ahead calls; None when the delegate had none.
static RUNE: OnceLock<Option<Terminate>> = OnceLock::new();

unsafe extern "C" {
    fn class_getInstanceMethod(class: *const AnyClass, selector: Sel) -> *mut c_void;
    fn method_setImplementation(
        method: *mut c_void,
        implementation: *const c_void,
    ) -> *const c_void;
    fn class_addMethod(
        class: *const AnyClass,
        selector: Sel,
        implementation: *const c_void,
        types: *const c_char,
    ) -> Bool;
}

extern "C" fn should_terminate(
    this: *mut AnyObject,
    selector: Sel,
    sender: *mut AnyObject,
) -> usize {
    if !super::may_quit() {
        return CANCEL;
    }
    match RUNE.get().copied().flatten() {
        Some(rune) => rune(this, selector, sender),
        None => NOW,
    }
}

pub(super) fn install() {
    if RUNE.get().is_some() {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let selector = sel!(applicationShouldTerminate:);
    let ours = should_terminate as Terminate as *const c_void;
    // SAFETY: AppKit and runtime calls on the main thread. The swapped-in function has the
    // selector's ABI (`NSApplicationTerminateReply` is an `NSUInteger`, `Q` on 64-bit), and the
    // one it replaces is only ever called with that ABI.
    unsafe {
        let delegate: *mut AnyObject = msg_send![&app, delegate];
        let Some(delegate) = delegate.as_ref() else {
            return;
        };
        let class = delegate.class();
        let method = class_getInstanceMethod(class, selector);
        let rune = if method.is_null() {
            if !class_addMethod(class, selector, ours, c"Q@:@".as_ptr()).as_bool() {
                return;
            }
            None
        } else {
            let old = method_setImplementation(method, ours);
            (!old.is_null()).then(|| std::mem::transmute::<*const c_void, Terminate>(old))
        };
        let _ = RUNE.set(rune);
        // AppKit caches which optional methods its delegate answers when it is set.
        let _: () = msg_send![&app, setDelegate: delegate];
    }
}

pub(super) fn come_forward() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: AppKit on the main thread.
    unsafe { app.unhide(None) };
    // `activate` replaces it from macOS 14 on; the app still runs on 13.
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
}
