//! Embeds the repository's `docs/` in the crate: every Markdown file and every image, so
//! the pages a binary serves are the documentation of the commit it was built from.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let docs = manifest.join("../../docs");
    let docs = docs.canonicalize().expect("the repository's docs/ folder");
    // a file added to or removed from docs/ rebuilds the list
    println!("cargo:rerun-if-changed={}", docs.display());
    let mut files = Vec::new();
    walk(&docs, &mut files);
    files.sort();
    let mut pages = String::from("pub static PAGES: &[(&str, &str)] = &[\n");
    let mut images = String::from("pub static IMAGES: &[(&str, &[u8])] = &[\n");
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        let rel = f
            .strip_prefix(&docs)
            .expect("under docs/")
            .to_string_lossy()
            .replace('\\', "/");
        let abs = f.display().to_string();
        match f.extension().and_then(|e| e.to_str()) {
            Some("md") => writeln!(pages, "    ({rel:?}, include_str!({abs:?})),").unwrap(),
            Some("png" | "webp" | "jpg" | "svg" | "gif") => {
                writeln!(images, "    ({rel:?}, include_bytes!({abs:?})),").unwrap();
            }
            _ => {}
        }
    }
    pages.push_str("];\n");
    images.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir")).join("embedded.rs");
    std::fs::write(out, pages + &images).expect("writing embedded.rs");
}
