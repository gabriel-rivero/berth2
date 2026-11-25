//! The macOS system menu bar for the portable window.
//!
//! Winit installs an application menu (About, Hide, Quit); this appends the
//! window's own menus to it and routes a chosen item back through the event
//! loop. The command table is the same one the painted bar uses on other
//! platforms, so the two can never drift apart.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{ClassType, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
use objc2_foundation::{MainThreadMarker, NSString};
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;
use crate::menu::{self, Entry};

thread_local! {
    /// The proxy the menu callbacks post through. Menus live on the main
    /// thread, so a thread-local is enough and avoids asking the proxy to be
    /// shareable across threads.
    static PROXY: RefCell<Option<EventLoopProxy<UserEvent>>> = const { RefCell::new(None) };
    /// AppKit holds menu targets unretained, so the instance has to outlive the
    /// menus that point at it.
    static TARGET: RefCell<Option<Retained<MenuTarget>>> = const { RefCell::new(None) };
}

static INSTALLED: AtomicBool = AtomicBool::new(false);

define_class!(
    /// Receives every menu choice. The item's tag says which entry it is.
    #[unsafe(super(NSObject))]
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(berthMenuAction:))]
        fn menu_action(&self, sender: &NSMenuItem) {
            let tag = sender.tag();
            PROXY.with(|proxy| {
                if let Some(proxy) = &*proxy.borrow() {
                    let _ = proxy.send_event(UserEvent::Menu(tag));
                }
            });
        }
    }
);

/// Add Berth's menus to the system menu bar. Safe to call more than once: only
/// the first call builds anything.
pub fn install(proxy: &EventLoopProxy<UserEvent>) {
    if INSTALLED.load(Ordering::SeqCst) {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("berth-gui: menu: not on the main thread");
        return;
    };
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    PROXY.with(|cell| *cell.borrow_mut() = Some(proxy.clone()));
    let app = NSApplication::sharedApplication(mtm);
    let menubar = match app.mainMenu() {
        Some(menu) => menu,
        None => {
            let menu = NSMenu::new(mtm);
            app.setMainMenu(Some(&menu));
            menu
        }
    };

    let target: Retained<MenuTarget> = unsafe { msg_send![MenuTarget::class(), new] };
    // SAFETY: every `Message` type may be reinterpreted as `AnyObject`, and the
    // retained instance is kept alive in `TARGET` for the process lifetime.
    let target_ref: &AnyObject = unsafe { &*(Retained::as_ptr(&target) as *const AnyObject) };

    // The app menu holds the real Quit; our own menus skip it. Without one we
    // have to keep Quit somewhere or the app has no way out.
    let app_quit = menubar.itemAtIndex(0).and_then(|item| item.submenu());
    let include_quit = app_quit.is_none();

    for (index, title) in menu::TITLES.iter().enumerate() {
        let entries = visible_entries(menu::entries(index), include_quit);
        let top = NSMenuItem::new(mtm);
        let name = NSString::from_str(title);
        top.setTitle(&name);
        let submenu = NSMenu::new(mtm);
        submenu.setTitle(&name);
        for (row, entry) in entries {
            if matches!(entry, Entry::Separator) {
                submenu.addItem(&NSMenuItem::separatorItem(mtm));
                continue;
            }
            let label = NSString::from_str(entry.label());
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    mtm.alloc(),
                    &label,
                    Some(sel!(berthMenuAction:)),
                    &NSString::from_str(""),
                )
            };
            unsafe { item.setTarget(Some(target_ref)) };
            item.setTag(menu::tag(index, row));
            submenu.addItem(&item);
        }
        top.setSubmenu(Some(&submenu));
        menubar.addItem(&top);
    }

    if std::env::var_os("BERTH_TRACE").is_some() {
        let titles: Vec<String> = (0..menubar.numberOfItems())
            .filter_map(|index| menubar.itemAtIndex(index))
            .map(|item| item.title().to_string())
            .collect();
        eprintln!(
            "berth-gui: menu: {} items in the system menu bar: {}",
            menubar.numberOfItems(),
            titles.join(", ")
        );
    }

    // Winit's app menu Quit ends the process without saving the session. Point
    // it at our own Quit entry so it goes through the normal save-and-exit path.
    if let Some(app_menu) = app_quit
        && let Some(quit_row) = menu::entries(0)
            .iter()
            .position(|entry| matches!(entry, Entry::Quit))
    {
        for index in 0..app_menu.numberOfItems() {
            if let Some(item) = app_menu.itemAtIndex(index)
                && item.action() == Some(sel!(terminate:))
            {
                unsafe { item.setAction(Some(sel!(berthMenuAction:))) };
                unsafe { item.setTarget(Some(target_ref)) };
                item.setTag(menu::tag(0, quit_row));
            }
        }
    }

    TARGET.with(|cell| *cell.borrow_mut() = Some(target));
}

/// The entries to show, with a trailing separator trimmed and Quit optionally
/// removed. The original row index is kept so the tag still resolves.
fn visible_entries(entries: &[Entry], include_quit: bool) -> Vec<(usize, Entry)> {
    let visible: Vec<(usize, Entry)> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| include_quit || !matches!(entry, Entry::Quit))
        .map(|(row, entry)| (row, *entry))
        .collect();
    let start = visible
        .iter()
        .position(|(_, entry)| !matches!(entry, Entry::Separator))
        .unwrap_or(0);
    let end = visible
        .iter()
        .rposition(|(_, entry)| !matches!(entry, Entry::Separator))
        .map(|index| index + 1)
        .unwrap_or(0);
    visible[start..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_the_berth_menu_when_quit_is_in_the_app_menu_then_it_is_dropped_with_the_trailer() {
        let entries = visible_entries(menu::entries(0), false);
        assert!(entries.iter().all(|(_, entry)| *entry != Entry::Quit));
        assert_eq!(
            entries.last().map(|(_, entry)| *entry),
            Some(Entry::Action(crate::config::Action::QuickTerminal))
        );
        assert!(entries.iter().all(|(_, entry)| *entry != Entry::Separator));
    }

    #[test]
    fn given_no_app_menu_then_our_menus_keep_quit() {
        let entries = visible_entries(menu::entries(0), true);
        assert!(entries.iter().any(|(_, entry)| *entry == Entry::Quit));
    }

    #[test]
    fn given_a_row_then_its_tag_round_trips_to_the_same_entry() {
        for menu_index in 0..menu::TITLES.len() {
            for (row, entry) in menu::entries(menu_index).iter().enumerate() {
                assert_eq!(
                    menu::entry_for_tag(menu::tag(menu_index, row)),
                    Some(*entry)
                );
            }
        }
    }
}
