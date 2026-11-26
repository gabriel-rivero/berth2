//! Compile the application icon into the Windows `.exe`.
//!
//! `set_window_icon` only reaches the running window. Explorer, the taskbar and
//! the file association read the icon resource inside the executable.

fn main() {
    // Paths here resolve against the crate root, which is not the process's
    // working directory. Anchor on the manifest dir so both agree.
    let root = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../../../packaging");
    println!(
        "cargo:rerun-if-changed={}",
        root.join("berth.ico").display()
    );
    // A build script runs on the host, so `cfg!(windows)` would be wrong when
    // cross-compiling. The resource belongs to the target OS.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(root.join("berth.ico").to_str().unwrap());
    if let Err(error) = resource.compile() {
        println!("cargo:warning=could not embed the application icon: {error}");
    }
}
