//! The menu-bar item as AppKit's `NSStatusItem`, made and changed on the main thread. Its two
//! commands go to a small Objective-C object of the app's, whose methods call back into Rust.

use std::cell::RefCell;
use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{ClassType, DeclaredClass, declare_class, msg_send_id, mutability, sel};
use objc2_app_kit::{NSApplication, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem};
use objc2_foundation::{MainThreadMarker, NSString};

use super::State;

/// `NSVariableStatusItemLength`: the item is as wide as its image and title.
const VARIABLE_LENGTH: f64 = -1.0;

static OPEN: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

pub(super) fn on_open(open: impl Fn() + Send + Sync + 'static) {
    let _ = OPEN.set(Box::new(open));
}

declare_class!(
    struct Target;

    unsafe impl ClassType for Target {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "VelvetHammerMenuBarTarget";
    }

    impl DeclaredClass for Target {}

    unsafe impl Target {
        #[method(openWindow:)]
        fn open_window(&self, _sender: Option<&AnyObject>) {
            if let Some(open) = OPEN.get() {
                open();
            }
        }

        #[method(quitApp:)]
        fn quit_app(&self, _sender: Option<&AnyObject>) {
            let mtm = MainThreadMarker::from(self);
            // SAFETY: AppKit on the main thread. The quit goes through the app delegate, so it
            // asks first while work is in flight (see crate::lifecycle).
            unsafe { NSApplication::sharedApplication(mtm).terminate(None) };
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: NSObject's plain initializer.
        unsafe { msg_send_id![mtm.alloc::<Self>(), init] }
    }
}

/// The item while the app runs, and what it shows.
struct Item {
    item: Retained<NSStatusItem>,
    target: Retained<Target>,
    shown: State,
}

thread_local! {
    static ITEM: RefCell<Option<Item>> = const { RefCell::new(None) };
}

pub(super) fn show(state: &State) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    ITEM.with_borrow_mut(|item| {
        if item.as_ref().is_some_and(|i| &i.shown == state) {
            return;
        }
        let item = item.get_or_insert_with(|| make(mtm));
        // SAFETY: AppKit calls on the main thread, on objects this thread made.
        unsafe { fill(item, state, mtm) };
        item.shown = state.clone();
    });
}

fn make(mtm: MainThreadMarker) -> Item {
    // SAFETY: AppKit calls on the main thread. The hammer is a template image, so the menu bar
    // tints it for light and dark.
    let item = unsafe {
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(VARIABLE_LENGTH);
        if let Some(button) = item.button(mtm) {
            let name = NSString::from_str("hammer");
            let label = NSString::from_str("Velvet Hammer");
            match NSImage::imageWithSystemSymbolName_accessibilityDescription(&name, Some(&label)) {
                Some(image) => {
                    image.setTemplate(true);
                    button.setImage(Some(&image));
                }
                None => button.setTitle(&NSString::from_str("VH")),
            }
        }
        item
    };
    Item {
        item,
        target: Target::new(mtm),
        shown: State::default(),
    }
}

/// Sets the count beside the hammer and rebuilds the menu.
unsafe fn fill(item: &Item, state: &State, mtm: MainThreadMarker) {
    unsafe {
        if let Some(button) = item.item.button(mtm) {
            let title = if state.title.is_empty() {
                String::new()
            } else {
                format!(" {}", state.title)
            };
            if button.image().is_some() {
                button.setTitle(&NSString::from_str(&title));
            }
        }
        let menu = NSMenu::new(mtm);
        menu.setAutoenablesItems(false);
        for line in &state.lines {
            let entry = NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                &NSString::from_str(line),
                None,
                &NSString::from_str(""),
            );
            entry.setEnabled(false);
            menu.addItem(&entry);
        }
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        for (title, action) in [
            ("Open Velvet Hammer", sel!(openWindow:)),
            ("Quit Velvet Hammer", sel!(quitApp:)),
        ] {
            let entry = NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(""),
            );
            entry.setTarget(Some(&item.target));
            entry.setEnabled(true);
            menu.addItem(&entry);
        }
        item.item.setMenu(Some(&menu));
    }
}
