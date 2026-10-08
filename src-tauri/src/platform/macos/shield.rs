//! Keeps the dropdown out of screen captures while it shows concealed
//! holdings (`bar::View::conceal`). AppKit draws a menu, and each submenu, in
//! a window it creates every time one opens, so there is nothing to mark
//! ahead of time: while the dropdown is being tracked, each popup menu
//! window that comes on screen is marked not to be shared.
//!
//! The status item can't be marked: its window belongs to the menu bar, not
//! to the app, so the bar masks the total instead (`bar::MASK`).

use std::{cell::Cell, ptr::NonNull};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSMenu, NSMenuDidBeginTrackingNotification, NSMenuDidEndTrackingNotification,
    NSPopUpMenuWindowLevel, NSWindow, NSWindowDidChangeOcclusionStateNotification,
    NSWindowSharingType,
};
use objc2_foundation::{NSNotification, NSNotificationCenter};

thread_local! {
    /// The dropdown, while it shows concealed holdings.
    static GUARDED: Cell<*const NSMenu> = const { Cell::new(std::ptr::null()) };
    /// It is open (tracked), submenus included.
    static TRACKING: Cell<bool> = const { Cell::new(false) };
}

/// Starts watching menus open and windows come on screen. Call once, on the
/// main thread; the observers live for the rest of the process.
pub fn install() {
    let center = NSNotificationCenter::defaultCenter();
    let began = RcBlock::new(|note: NonNull<NSNotification>| {
        // SAFETY: AppKit hands the observer a live notification.
        let menu = unsafe { note.as_ref() }.object();
        let ours = menu.is_some_and(|menu| Retained::as_ptr(&menu).cast() == GUARDED.get());
        TRACKING.set(ours);
    });
    // One menu is tracked at a time, so any ending ends ours.
    let ended = RcBlock::new(|_: NonNull<NSNotification>| TRACKING.set(false));
    let shown = RcBlock::new(|note: NonNull<NSNotification>| {
        if !TRACKING.get() {
            return;
        }
        // SAFETY: AppKit hands the observer a live notification.
        let Some(Ok(window)) = unsafe { note.as_ref() }.object().map(|o| o.downcast::<NSWindow>())
        else {
            return;
        };
        if window.level() == NSPopUpMenuWindowLevel {
            window.setSharingType(NSWindowSharingType::None);
        }
    });
    // SAFETY: reading immutable AppKit notification-name constants. A nil
    // queue runs a block on the posting thread, which for menu and window
    // notifications is the main one, where the state above lives.
    unsafe {
        for (name, block) in [
            (NSMenuDidBeginTrackingNotification, &began),
            (NSMenuDidEndTrackingNotification, &ended),
            (NSWindowDidChangeOcclusionStateNotification, &shown),
        ] {
            let token =
                center.addObserverForName_object_queue_usingBlock(Some(name), None, None, block);
            std::mem::forget(token);
        }
    }
}

/// The menu to keep out of captures from now on: the dropdown while it
/// shows concealed holdings, else none.
pub fn guard(menu: Option<&NSMenu>) {
    GUARDED.set(menu.map_or(std::ptr::null(), std::ptr::from_ref));
}
