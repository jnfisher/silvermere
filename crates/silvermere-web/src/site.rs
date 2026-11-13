//! The documentation as pages: the repository's `docs/` (embedded at build time) rendered
//! from Markdown, with the links between files turned into links between pages, and the
//! settings' descriptions read from the key tables of `docs/config.md`.

use std::collections::BTreeMap;

use pulldown_cmark::{CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded.rs"));
}

/// Where a link out of `docs/` goes: the file on GitHub.
const REPO_FILES: &str = "https://github.com/jnfisher/silvermere/blob/main/";

/// The index of `docs/` on GitHub; here the documentation's index page stands in for it.
const INDEX: &str = "README.md";

/// The page the FAQ is written in; it has a place of its own in the navigation.
pub const FAQ: &str = "faq";

/// The sidebar's groups and the pages each starts with, in order. A page in none of them
/// (a new file in `docs/`) is listed under the last.
const GROUPS: &[(&str, &[&str])] = &[
    (
        "Set up",
        &[
            "install",
            "windows",
            "deploy-checklist",
            "import",
            "auth",
            "legacy-clients",
        ],
    ),
    (
        "Run",
        &["config", "web", "cli", "sysop", "admin-api", "metrics"],
    ),
    ("Reference", &[]),
];

/// One documentation page.
#[derive(Debug, Clone)]
pub struct Page {
    /// The file's path under `docs/` without `.md`, e.g. `install` or `engine/shops`.
    pub slug: String,
    /// The first heading.
    pub title: String,
    /// The rendered body.
    pub html: String,
    /// The sidebar group, an index into [`Site::groups`].
    pub group: usize,
}

/// What `docs/config.md` says of one setting.
#[derive(Debug, Clone, Default)]
pub struct KeyDoc {
    /// The `SILVERMERE_*` variable.
    pub env: String,
    /// The default, as TOML.
    pub default: String,
    /// The meaning, rendered.
    pub meaning: String,
    /// The original game's option, if the key replaces one.
    pub original: String,
}

/// Every page, rendered for a site served under `base`.
#[derive(Debug)]
pub struct Site {
    /// The path prefix of every link (`""` on the server, `"/silvermere"` on a project
    /// site).
    pub base: String,
    /// The pages, in sidebar order.
    pub pages: Vec<Page>,
    /// The settings in the order `docs/config.md` lists them.
    pub keys: Vec<(String, KeyDoc)>,
}

impl Site {
    /// Renders the embedded documentation.
    pub fn build(base: &str) -> Site {
        let base = base.trim_end_matches('/').to_owned();
        let mut pages: Vec<Page> = embedded::PAGES
            .iter()
            // the folder's own index: the documentation's index page stands in for it
            .filter(|(path, _)| *path != INDEX)
            .map(|(path, md)| {
                let slug = path.trim_end_matches(".md").to_owned();
                let (title, html) = render(md, path, &base);
                let group = GROUPS
                    .iter()
                    .position(|(_, slugs)| slugs.contains(&slug.as_str()))
                    .unwrap_or(GROUPS.len() - 1);
                Page {
                    title: title.unwrap_or_else(|| slug.clone()),
                    slug,
                    html,
                    group,
                }
            })
            .collect();
        let place = |p: &Page| {
            GROUPS[p.group]
                .1
                .iter()
                .position(|s| *s == p.slug)
                .unwrap_or(usize::MAX)
        };
        pages.sort_by(|a, b| (a.group, place(a), &a.title).cmp(&(b.group, place(b), &b.title)));
        let keys = embedded::PAGES
            .iter()
            .find(|(path, _)| *path == "config.md")
            .map(|(_, md)| key_docs(md, &base))
            .unwrap_or_default();
        Site { base, pages, keys }
    }

    /// The names of the sidebar groups.
    pub fn groups(&self) -> impl Iterator<Item = &'static str> {
        GROUPS.iter().map(|(name, _)| *name)
    }

    /// The page with `slug`.
    pub fn page(&self, slug: &str) -> Option<&Page> {
        self.pages.iter().find(|p| p.slug == slug)
    }

    /// What the documentation says of the setting `key`.
    pub fn key(&self, key: &str) -> Option<&KeyDoc> {
        self.keys.iter().find(|(k, _)| k == key).map(|(_, d)| d)
    }
}

/// An embedded image by its path under `docs/`, with its media type.
pub fn image(path: &str) -> Option<(&'static [u8], &'static str)> {
    let (_, bytes) = embedded::IMAGES.iter().find(|(p, _)| *p == path)?;
    let mime = match path.rsplit('.').next() {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("jpg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    };
    Some((bytes, mime))
}

/// Every embedded image's path under `docs/`.
pub fn images() -> impl Iterator<Item = (&'static str, &'static [u8])> {
    embedded::IMAGES.iter().copied()
}

/// `&`, `<`, `>` and the quotes escaped for HTML text and attribute values.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A heading's anchor as GitHub makes it: lower case, letters, digits, `-` and `_` kept,
/// spaces to `-`, everything else dropped.
fn anchor(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() => Some(c.to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

/// Where the link `dest`, written in the file `from` (a path under `docs/`), leads on the
/// site: another page, an embedded image, or the file on GitHub.
fn rewrite(dest: &str, from: &str, base: &str) -> String {
    if dest.contains("://") || dest.starts_with("mailto:") || dest.starts_with('#') {
        return dest.to_owned();
    }
    let (path, frag) = match dest.split_once('#') {
        Some((p, f)) => (p, format!("#{f}")),
        None => (dest, String::new()),
    };
    // the path from the repository's root: `docs/` and the file's own folders, then `path`
    let mut parts: Vec<&str> = vec!["docs"];
    parts.extend(
        from.split('/')
            .rev()
            .skip(1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev(),
    );
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let in_repo = parts.join("/");
    let Some(in_docs) = in_repo.strip_prefix("docs/") else {
        return format!("{REPO_FILES}{in_repo}{frag}");
    };
    if in_docs == INDEX {
        return format!("{base}/docs{frag}");
    }
    if let Some(slug) = in_docs.strip_suffix(".md")
        && embedded::PAGES.iter().any(|(p, _)| *p == in_docs)
    {
        return if slug == FAQ {
            format!("{base}/faq{frag}")
        } else {
            format!("{base}/docs/{slug}{frag}")
        };
    }
    if image(in_docs).is_some() {
        return format!("{base}/assets/{in_docs}");
    }
    format!("{REPO_FILES}{in_repo}{frag}")
}

/// The Markdown `md` of the file `from` as HTML, and its first heading.
fn render(md: &str, from: &str, base: &str) -> (Option<String>, String) {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let mut events: Vec<Event<'_>> = Parser::new_ext(md, opts).collect();
    let mut title = None;
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut i = 0;
    while i < events.len() {
        match &events[i] {
            Event::Start(Tag::Heading { level, .. }) => {
                let level = *level;
                let mut text = String::new();
                let mut j = i + 1;
                while j < events.len() && !matches!(events[j], Event::End(TagEnd::Heading(_))) {
                    if let Event::Text(t) | Event::Code(t) = &events[j] {
                        text.push_str(t);
                    }
                    j += 1;
                }
                if level == HeadingLevel::H1 && title.is_none() {
                    title = Some(text.clone());
                }
                let mut id = anchor(&text);
                let n = seen.entry(id.clone()).or_insert(0);
                if *n > 0 {
                    id = format!("{id}-{n}");
                }
                *n += 1;
                events[i] = Event::Start(Tag::Heading {
                    level,
                    id: Some(CowStr::from(id)),
                    classes: Vec::new(),
                    attrs: Vec::new(),
                });
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                events[i] = Event::Start(Tag::Link {
                    link_type: *link_type,
                    dest_url: CowStr::from(rewrite(dest_url, from, base)),
                    title: title.clone(),
                    id: id.clone(),
                });
            }
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                events[i] = Event::Start(Tag::Image {
                    link_type: *link_type,
                    dest_url: CowStr::from(rewrite(dest_url, from, base)),
                    title: title.clone(),
                    id: id.clone(),
                });
            }
            _ => {}
        }
        i += 1;
    }
    // a wide table scrolls inside its own box
    let events = events.into_iter().flat_map(|e| match e {
        Event::Start(Tag::Table(_)) => {
            vec![Event::Html(CowStr::from("<div class=\"table\">")), e]
        }
        Event::End(TagEnd::Table) => vec![e, Event::Html(CowStr::from("</div>"))],
        e => vec![e],
    });
    let mut out = String::with_capacity(md.len() * 2);
    html::push_html(&mut out, events);
    (title, out)
}

/// One line of Markdown as inline HTML (no surrounding paragraph).
fn inline(md: &str, base: &str) -> String {
    let (_, html) = render(md, "config.md", base);
    let html = html.trim();
    html.strip_prefix("<p>")
        .and_then(|h| h.strip_suffix("</p>"))
        .unwrap_or(html)
        .to_owned()
}

/// The cells of a table row; `\|` is a bar inside a cell.
fn cells(line: &str) -> Vec<String> {
    let line = line.trim().trim_start_matches('|').trim_end_matches('|');
    let mut out = vec![String::new()];
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                chars.next();
                out.last_mut().expect("a cell").push('|');
            }
            '|' => out.push(String::new()),
            c => out.last_mut().expect("a cell").push(c),
        }
    }
    out.into_iter().map(|c| c.trim().to_owned()).collect()
}

/// The rows of every `| Key | Environment variable | Default | Meaning | Original name |`
/// table of `docs/config.md`.
fn key_docs(md: &str, base: &str) -> Vec<(String, KeyDoc)> {
    let mut out = Vec::new();
    let mut in_table = false;
    for line in md.lines() {
        if !line.trim_start().starts_with('|') {
            in_table = false;
            continue;
        }
        let row = cells(line);
        if row.first().is_some_and(|c| c == "Key") && row.len() >= 4 {
            in_table = row[1].starts_with("Environment");
            continue;
        }
        if !in_table || row.len() < 4 || row[0].starts_with("---") {
            continue;
        }
        let bare = |c: &str| c.trim_matches('`').to_owned();
        let original = row.get(4).map_or(String::new(), |c| bare(c));
        out.push((
            bare(&row[0]),
            KeyDoc {
                env: bare(&row[1]),
                default: bare(&row[2]),
                meaning: inline(&row[3], base),
                original: if original == "—" {
                    String::new()
                } else {
                    original
                },
            },
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_between_files_become_links_between_pages() {
        assert_eq!(
            rewrite("install.md#upgrading", "config.md", ""),
            "/docs/install#upgrading"
        );
        assert_eq!(
            rewrite("../config.md", "engine/shops.md", "/s"),
            "/s/docs/config"
        );
        assert_eq!(rewrite("faq.md", "install.md", ""), "/faq");
        assert_eq!(
            rewrite("images/silvermere-logo.png", "install.md", ""),
            "/assets/images/silvermere-logo.png"
        );
        assert_eq!(
            rewrite("../CONTRIBUTING.md", "testing.md", ""),
            format!("{REPO_FILES}CONTRIBUTING.md")
        );
        assert_eq!(rewrite("#keys", "config.md", ""), "#keys");
        assert_eq!(
            rewrite("https://example.org/x.md", "config.md", ""),
            "https://example.org/x.md"
        );
    }

    #[test]
    fn headings_get_the_anchors_links_use() {
        let (title, html) = render("# The `config` command\n\n## Keys\n\n## Keys\n", "x.md", "");
        assert_eq!(title.as_deref(), Some("The config command"));
        assert!(html.contains("<h1 id=\"the-config-command\">"), "{html}");
        assert!(html.contains("<h2 id=\"keys\">") && html.contains("<h2 id=\"keys-1\">"));
    }

    #[test]
    fn every_page_has_a_title_and_the_keys_are_read() {
        let site = Site::build("");
        assert!(site.page("install").is_some());
        assert!(site.page(FAQ).is_some(), "docs/faq.md is a page");
        for p in &site.pages {
            assert!(!p.title.is_empty() && !p.html.is_empty(), "{}", p.slug);
        }
        let telnet = site.key("server.telnet_bind").expect("in docs/config.md");
        assert_eq!(telnet.env, "SILVERMERE_SERVER__TELNET_BIND");
        assert_eq!(telnet.default, "\"0.0.0.0:2323\"");
        assert!(telnet.meaning.contains("Telnet"));
        // a link in a meaning is a link to a page
        let rlogin = site.key("server.rlogin_bind").unwrap();
        assert!(
            rlogin.meaning.contains("href=\"/docs/auth#"),
            "{}",
            rlogin.meaning
        );
    }
}
