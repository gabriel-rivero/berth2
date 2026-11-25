//! The native file picker for the wallpaper path.
//!
//! `rfd` is the one dependency that talks to the platform's own dialog: an
//! `NSOpenPanel` on macOS, the XDG desktop portal on Linux, `IFileDialog` on
//! Windows. It is deliberately configured *without* the GTK backend, so the
//! window keeps its "no GTK headers" rule.
//!
//! The dialog is modal and blocks the calling thread until it closes. Callers
//! must therefore run it while the event loop is not spinning, or the window
//! freezes behind the panel — see `Gui::pick_wallpaper`.

use std::path::PathBuf;

/// The extensions the decoder can actually read. `image` is built with only its
/// `png` and `jpeg` features, so offering anything else would let the user pick
/// a file that then silently fails to draw.
const EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

/// Ask for one image. `start` is the directory to open in, if it exists.
///
/// Returns `None` when the user cancels, so cancelling never clears a working
/// wallpaper.
pub fn pick(start: Option<&std::path::Path>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Choose a wallpaper")
        .add_filter("Images", &EXTENSIONS);
    if let Some(directory) = start.filter(|path| path.is_dir()) {
        dialog = dialog.set_directory(directory);
    }
    dialog.pick_file()
}

/// The directory a picker should open in: the current wallpaper's own folder if
/// it has one, otherwise the user's pictures directory if it exists.
pub fn start_directory(current: &str) -> Option<PathBuf> {
    let configured = std::path::Path::new(current.trim());
    if let Some(parent) = configured.parent()
        && !current.trim().is_empty()
        && parent.is_dir()
    {
        return Some(parent.to_path_buf());
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let pictures = home.as_ref().map(|home| home.join("Pictures"));
    pictures
        .filter(|path| path.is_dir())
        .or_else(|| home.filter(|path| path.is_dir()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_a_configured_path_then_the_picker_opens_in_its_folder() {
        let directory = std::env::temp_dir();
        let file = directory.join("wallpaper.png");
        let start = start_directory(&file.to_string_lossy()).expect("the folder exists");
        assert_eq!(start, directory);
    }

    #[test]
    fn given_no_wallpaper_then_the_picker_falls_back_to_pictures() {
        // Either Pictures or the home directory, whichever exists; never the
        // configured empty string as a path.
        if let Some(start) = start_directory("") {
            assert!(start.is_dir(), "the fallback must be a real directory");
        }
    }

    #[test]
    fn given_a_missing_folder_then_the_picker_does_not_open_there() {
        let start = start_directory("/no/such/folder/wallpaper.png");
        assert!(
            start.is_none() || start.as_ref().is_some_and(|path| path.is_dir()),
            "a missing folder must not be offered as the start directory"
        );
    }

    #[test]
    fn given_the_filter_then_it_matches_what_the_decoder_reads() {
        // PNG and JPEG are the only formats `image` is built with, so the
        // filter must not advertise anything else.
        assert_eq!(EXTENSIONS, ["png", "jpg", "jpeg"]);
    }
}
