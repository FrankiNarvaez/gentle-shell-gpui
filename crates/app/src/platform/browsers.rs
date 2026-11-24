//! The web browsers installed on this machine, and opening a page in one.
//! What to look for and how to launch it is `gentle_core::web_browsers`;
//! the places are probed like the editors' (`platform::ides`).

use std::path::PathBuf;
use std::process::Stdio;

use gentle_core::ides::Os;
use gentle_core::web_browsers::{self, BROWSERS, WebBrowser};

use super::ides::find;

/// A browser found on disk.
#[derive(Clone, Debug)]
pub struct Installed {
    pub browser: &'static WebBrowser,
    /// The app bundle (macOS) or the program to run.
    pub path: PathBuf,
}

/// Where `browser` is installed here, if it is. Reads the disk and may run
/// the login shell once for the PATH: call it off the main thread.
pub fn locate(browser: &'static WebBrowser) -> Option<Installed> {
    let path = browser.lookups(Os::CURRENT).iter().find_map(find)?;
    Some(Installed { browser, path })
}

/// Every catalogued browser present here, in catalog order. Off the main
/// thread, like [`locate`].
pub fn detect() -> Vec<Installed> {
    BROWSERS.iter().filter_map(locate).collect()
}

/// Open `url` (http or https only) in `browser`, which comes to the front,
/// without waiting for it. The URL is one argument of the browser's own
/// program (`open -a` on macOS); no shell runs.
pub fn open(browser: &Installed, url: &str) -> std::io::Result<()> {
    let (program, args) =
        web_browsers::launch(Os::CURRENT, &browser.path, url).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "only http(s) pages open in a browser",
            )
        })?;
    let mut child = pi_rpc::command(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Reap it whenever it exits (`open` returns at once; a browser that was
    // already running hands the page over and exits too).
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
