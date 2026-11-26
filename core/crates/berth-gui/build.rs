//! Compile the application icon into the Windows `.exe`.
//!
//! `set_window_icon` only reaches the running window. Explorer, the taskbar and
//! the file association read the icon resource inside the executable.

fn main() {
    // Paths here resolve against the crate root, which is not the process's
    // working directory. Anchor on the manifest dir so both agree.
    let root = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../../../packaging");
    let icon = root.join("berth.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-env-changed=BERTH_SKIP_ICON");
    // A build script runs on the host, so `cfg!(windows)` would be wrong when
    // cross-compiling. The resource belongs to the target OS.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // Checking a Windows target from a host without a resource compiler is the
    // one case where skipping is correct. A real build must not skip, or it
    // ships the icon-less executable this script exists to prevent.
    if std::env::var_os("BERTH_SKIP_ICON").is_some() {
        println!("cargo:warning=skipping the Windows icon (BERTH_SKIP_ICON is set)");
        return;
    }
    let icon_path = icon
        .to_str()
        .unwrap_or_else(|| panic!("the icon path is not valid UTF-8: {}", icon.display()));
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(icon_path);
    if let Err(error) = resource.compile() {
        panic!(
            "could not embed {}: {error}. Install a resource compiler (rc.exe, or \
             windres/llvm-rc), or set BERTH_SKIP_ICON=1 to build without the icon.",
            icon.display()
        );
    }
}
