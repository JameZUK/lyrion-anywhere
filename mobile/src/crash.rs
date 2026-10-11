//! Crash reports for test builds: a panic anywhere in the Rust layer (including the audio
//! engine's own threads and callbacks, which would otherwise take the app down without a
//! trace) is written to a file the app shows on its next start.
//!
//! The report holds the panic message, where it happened, the thread and a backtrace. No
//! request contents, tokens or passwords are ever in a panic message in this crate.

use std::io::Write;
use std::sync::Once;

/// Largest report kept, in bytes (the rest is cut).
const MAX_REPORT_BYTES: usize = 32 * 1024;

static INSTALL: Once = Once::new();

/// Write panics to `path` (a file in the app's private storage), then let the previous
/// panic handling carry on as before. Only the first call takes effect.
#[uniffi::export]
pub fn install_crash_report(path: String) {
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            let message = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "(no message)".into());
            let place = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_default();
            let backtrace = std::backtrace::Backtrace::force_capture();
            let report = report_text(
                thread.name().unwrap_or("unnamed"),
                &message,
                &place,
                &backtrace.to_string(),
            );
            write_report(&path, &report);
            previous(info);
        }));
    });
}

/// The report's text, capped at [`MAX_REPORT_BYTES`] on a character boundary.
fn report_text(thread: &str, message: &str, place: &str, backtrace: &str) -> String {
    let mut s = format!(
        "Rust panic in library {}\nthread: {thread}\nat: {place}\nmessage: {message}\n\n{backtrace}",
        env!("CARGO_PKG_VERSION")
    );
    if s.len() > MAX_REPORT_BYTES {
        let cut = (0..=MAX_REPORT_BYTES)
            .rev()
            .find(|&i| s.is_char_boundary(i))
            .unwrap_or(0);
        s.truncate(cut);
        s.push_str("\n…");
    }
    s
}

/// Best effort: a failure to write is ignored (the app is going down anyway).
fn write_report(path: &str, report: &str) {
    if let Ok(mut f) = std::fs::File::create(path) {
        let _ = f.write_all(report.as_bytes());
        let _ = f.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_names_the_thread_place_and_message_and_is_capped() {
        let r = report_text("audio-out", "boom", "src/x.rs:7", "frames");
        assert!(
            r.contains("thread: audio-out")
                && r.contains("at: src/x.rs:7")
                && r.contains("message: boom")
        );
        let long = report_text("t", &"é".repeat(MAX_REPORT_BYTES), "p", "");
        assert!(long.len() <= MAX_REPORT_BYTES + 4 && long.ends_with('…'));
    }

    #[test]
    fn a_panic_on_another_thread_is_written_to_the_file() {
        let path = std::env::temp_dir().join(format!("anywhere-crash-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&path);
        install_crash_report(path.to_string_lossy().into_owned());
        let r = std::thread::Builder::new()
            .name("engine-test".into())
            .spawn(|| panic!("engine failed: {}", 42))
            .unwrap()
            .join();
        assert!(r.is_err());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("engine failed: 42"), "{text}");
        assert!(text.contains("thread: engine-test"));
        std::fs::remove_file(&path).unwrap();
    }
}
