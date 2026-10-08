//! Watches the clipboard and rewrites copied Instagram and X post links to the embed-fix host.
//!
//! Usage: `uet-clip https://fix.example.com` (or set `UET_FIX_URL`).

use std::{process::ExitCode, thread, time::Duration};

use arboard::Clipboard;
use url::Url;

const POLL: Duration = Duration::from_millis(300);

fn main() -> ExitCode {
    let Some(base) = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("UET_FIX_URL").ok())
    else {
        eprintln!("usage: uet-clip <fix-base-url>   (or set UET_FIX_URL)");
        return ExitCode::from(2);
    };
    let base = match Url::parse(&base) {
        Ok(u) if matches!(u.scheme(), "http" | "https") && u.host_str().is_some() => u,
        _ => {
            eprintln!("invalid fix base url: {base}");
            return ExitCode::from(2);
        }
    };
    let mut clipboard = match Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("clipboard unavailable: {e}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("uet-clip: rewriting Instagram and X links to {base}");

    let mut watcher = ChangeWatcher::default();
    loop {
        thread::sleep(POLL);
        if !watcher.changed(&mut clipboard) {
            continue;
        }
        let Ok(text) = clipboard.get_text() else { continue };
        // Only a clipboard that is exactly one link; never touch copied prose or code.
        let Some(fixed) = uet_core::rewrite_url(&text, &base) else { continue };
        match clipboard.set_text(&fixed) {
            Ok(()) => eprintln!("{} -> {fixed}", text.trim()),
            Err(e) => eprintln!("clipboard write failed: {e}"),
        }
    }
}

/// macOS: compare NSPasteboard's change counter, so idle polls read nothing.
#[cfg(target_os = "macos")]
#[derive(Default)]
struct ChangeWatcher {
    last: Option<isize>,
}

#[cfg(target_os = "macos")]
impl ChangeWatcher {
    fn changed(&mut self, _clipboard: &mut Clipboard) -> bool {
        let count = objc2_app_kit::NSPasteboard::generalPasteboard().changeCount();
        self.last.replace(count) != Some(count)
    }
}

/// Elsewhere: compare clipboard text.
#[cfg(not(target_os = "macos"))]
#[derive(Default)]
struct ChangeWatcher {
    last: Option<String>,
}

#[cfg(not(target_os = "macos"))]
impl ChangeWatcher {
    fn changed(&mut self, clipboard: &mut Clipboard) -> bool {
        let Ok(text) = clipboard.get_text() else { return false };
        if self.last.as_ref() == Some(&text) {
            return false;
        }
        self.last = Some(text);
        true
    }
}
