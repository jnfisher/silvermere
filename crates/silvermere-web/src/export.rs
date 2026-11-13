//! The open pages as static files: the public copy of the documentation, which has no
//! server behind it (the settings page is the reference, and nobody signs in).

use std::path::Path;

use crate::page::{self, Chrome};
use crate::site::{self, Site};

/// Writes the site into `dir` for serving under the path `base` (`""` for the root of a
/// host, `"/silvermere"` for a project site). Returns the number of files written.
pub fn export(dir: &Path, base: &str, version: &str) -> std::io::Result<usize> {
    let site = Site::build(base);
    let c = Chrome {
        site: &site,
        version,
        user: None,
        exported: true,
    };
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("index.html".into(), page::home(&c).into_bytes()),
        ("docs/index.html".into(), page::docs_index(&c).into_bytes()),
        (
            "settings.html".into(),
            page::settings_reference(&c).into_bytes(),
        ),
        (
            "404.html".into(),
            page::notice(&c, "Not found", "There is no such page.").into_bytes(),
        ),
        ("assets/style.css".into(), page::STYLE.as_bytes().to_vec()),
        ("assets/app.js".into(), page::SCRIPT.as_bytes().to_vec()),
    ];
    for p in &site.pages {
        let Some(html) = page::doc(&c, &p.slug) else {
            continue;
        };
        let path = if p.slug == site::FAQ {
            "faq.html".to_owned()
        } else {
            format!("docs/{}.html", p.slug)
        };
        files.push((path, html.into_bytes()));
    }
    for (path, bytes) in site::images() {
        files.push((format!("assets/{path}"), bytes.to_vec()));
    }
    for (path, bytes) in &files {
        let to = dir.join(path);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(to, bytes)?;
    }
    Ok(files.len())
}
