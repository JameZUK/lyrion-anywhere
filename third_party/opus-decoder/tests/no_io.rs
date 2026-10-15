//! The decoder does no I/O (a Lyrion Anywhere addition, see `PATCHES.md`).
//!
//! Upstream 0.1.1 shipped debugging code that opened and appended to a log file at a
//! hard-coded path on its author's machine on every CELT frame. The decoder ships in every
//! Lyrion Anywhere binary, whose core does no I/O, so this test fails if file or stream I/O
//! (or that path, or the debug-log regions it came in) appears anywhere in the library
//! sources outside their test modules and doc comments.

use std::fs;
use std::path::{Path, PathBuf};

const FORBIDDEN: &[&str] = &[
    "OpenOptions",
    "std::fs",
    "std::io::Write",
    "/Users/",
    "#region agent log",
    "File::create",
    "File::open",
];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn library_sources_do_no_io() {
    let mut files = Vec::new();
    sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    assert!(files.len() > 20, "found only {} sources", files.len());
    let mut hits = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).unwrap();
        for (n, line) in text.lines().enumerate() {
            // Each file's test module comes last; everything after it is test-only.
            if line.trim() == "#[cfg(test)]" {
                break;
            }
            let code = line.trim_start();
            if code.starts_with("///") || code.starts_with("//!") {
                continue;
            }
            for pat in FORBIDDEN {
                if line.contains(pat) {
                    hits.push(format!("{}:{}: {pat}", file.display(), n + 1));
                }
            }
        }
    }
    assert!(hits.is_empty(), "I/O in the decoder:\n{}", hits.join("\n"));
}
