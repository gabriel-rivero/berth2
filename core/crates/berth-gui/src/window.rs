//! Window-level appearance the portable shell applies after creation.
//!
//! Opacity is the only setting here: macOS exposes it as `NSWindow.alphaValue`,
//! and the other platforms keep the window opaque because winit has no port of
//! it.

use winit::window::Window;

/// Set the window opacity, clamped to the configured range.
pub fn set_opacity(window: &Window, opacity: f32) {
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return;
        };
        let value = f64::from(opacity.clamp(crate::config::MIN_OPACITY, 1.0));
        unsafe {
            let view = appkit.ns_view.as_ptr().cast::<AnyObject>();
            let ns_window: *mut AnyObject = msg_send![view, window];
            if !ns_window.is_null() {
                let _: () = msg_send![ns_window, setAlphaValue: value];
                if std::env::var_os("BERTH_TRACE").is_some() {
                    let applied: f64 = msg_send![ns_window, alphaValue];
                    eprintln!("berth-gui: window opacity {applied:.2}");
                }
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, opacity);
    }
}
