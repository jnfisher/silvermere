//! The pages' HTML: one layout, the home page, a documentation page, the settings pages
//! and the login form. Everything a page shows of the outside world goes through
//! [`escape`].

use std::fmt::Write as _;

use crate::settings::{Kind, Lock, SettingValue, SettingsView, Source};
use crate::site::{FAQ, KeyDoc, Site, escape};

/// The style sheet, served as `/assets/style.css`.
pub const STYLE: &str = include_str!("style.css");
/// The settings filter, served as `/assets/app.js`.
pub const SCRIPT: &str = include_str!("app.js");

/// Which entry of the top navigation is the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Home,
    Docs,
    Faq,
    Settings,
    None,
}

/// What the layout needs besides the body.
pub struct Chrome<'a> {
    pub site: &'a Site,
    /// The server's version.
    pub version: &'a str,
    /// The sysop signed in, with the token his forms carry.
    pub user: Option<(&'a str, &'a str)>,
    /// A copy with no server behind it (the public documentation): no sign-in.
    pub exported: bool,
}

/// A whole page.
pub fn layout(c: &Chrome<'_>, title: &str, nav: Nav, body: &str) -> String {
    let base = &c.site.base;
    let link = |href: &str, text: &str, here: bool| {
        let class = if here { " class=\"here\"" } else { "" };
        format!("<a href=\"{base}{href}\"{class}>{text}</a>")
    };
    let account = match (c.user, c.exported) {
        (Some((name, csrf)), _) => format!(
            "<form method=\"post\" action=\"{base}/logout\" class=\"account\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
             <span>{}</span> <button type=\"submit\" class=\"link\">Sign out</button></form>",
            escape(csrf),
            escape(name)
        ),
        (None, false) => format!(
            "<span class=\"account\">{}</span>",
            link("/login", "Sign in", false)
        ),
        (None, true) => String::new(),
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title} · Silvermere</title>\n\
         <link rel=\"stylesheet\" href=\"{base}/assets/style.css\">\n\
         <link rel=\"icon\" href=\"{base}/assets/images/silvermere-logo.png\">\n\
         </head>\n<body>\n<header>\n\
         <a class=\"brand\" href=\"{base}/\"><img src=\"{base}/assets/images/silvermere-logo.png\" \
         alt=\"Silvermere\" width=\"166\" height=\"48\"></a>\n\
         <nav>{}{}{}{}</nav>\n{account}\n</header>\n{body}\n\
         <footer>Silvermere {} · these pages were built with the server and describe this version · \
         <a href=\"{base}/docs/web\">about these pages</a> · \
         <a href=\"https://github.com/jnfisher/silvermere\">source</a></footer>\n\
         <script src=\"{base}/assets/app.js\"></script>\n</body>\n</html>\n",
        link("/", "Home", nav == Nav::Home),
        link("/docs", "Documentation", nav == Nav::Docs),
        link("/faq", "FAQ", nav == Nav::Faq),
        link("/settings", "Settings", nav == Nav::Settings),
        escape(c.version),
        title = escape(title),
    )
}

/// The home page: the logo and where to go next.
pub fn home(c: &Chrome<'_>) -> String {
    let base = &c.site.base;
    let card = |href: &str, title: &str, text: &str| {
        format!("<a class=\"card\" href=\"{base}{href}\"><h3>{title}</h3><p>{text}</p></a>")
    };
    let settings = if c.exported {
        card(
            "/settings",
            "Settings reference",
            "Every setting, its default and what it does.",
        )
    } else {
        card(
            "/settings",
            "Settings",
            "See and change every setting of this server. Sysops sign in with their game account.",
        )
    };
    let body = format!(
        "<main class=\"home\">\n<img class=\"hero\" src=\"{base}/assets/images/silvermere-logo.png\" \
         alt=\"Silvermere\" width=\"720\" height=\"208\">\n\
         <p class=\"lead\">A game server for the classic text realm, with its own accounts, \
         a database and no BBS required.</p>\n\
         <h2>Set it up</h2>\n<div class=\"cards\">{}{}{}{}</div>\n\
         <h2>Run it</h2>\n<div class=\"cards\">{settings}{}{}{}</div>\n</main>",
        card(
            "/docs/install",
            "Linux and Docker",
            "Install the release, run it as a service or in a container, upgrade it."
        ),
        card(
            "/docs/windows",
            "Windows",
            "Run the Windows build and keep it running."
        ),
        card(
            "/docs/auth#rlogin-a-bbs-door",
            "Behind a BBS",
            "Open the game as a door: callers go straight in over rlogin."
        ),
        card(
            "/docs/import",
            "Bring an existing game",
            "Import the world and the players of an original installation."
        ),
        card("/faq", "FAQ", "Short answers to what sysops ask first."),
        card(
            "/docs/sysop",
            "Sysop commands",
            "What a sysop can do in the game and from the command line."
        ),
        card(
            "/docs",
            "All documentation",
            "Configuration, the admin API, backups, metrics and the reference pages."
        ),
    );
    layout(c, "Home", Nav::Home, &body)
}

/// The sidebar of the documentation, with `current` marked.
fn docs_sidebar(site: &Site, current: Option<&str>) -> String {
    let mut out = String::from("<aside>\n");
    for (g, name) in site.groups().enumerate() {
        let _ = writeln!(out, "<h4>{name}</h4>\n<ul>");
        for p in site.pages.iter().filter(|p| p.group == g && p.slug != FAQ) {
            let class = if current == Some(p.slug.as_str()) {
                " class=\"here\""
            } else {
                ""
            };
            let _ = writeln!(
                out,
                "<li><a href=\"{}/docs/{}\"{class}>{}</a></li>",
                site.base,
                p.slug,
                escape(&p.title)
            );
        }
        out.push_str("</ul>\n");
    }
    out.push_str("</aside>");
    out
}

/// One documentation page beside the sidebar.
pub fn doc(c: &Chrome<'_>, slug: &str) -> Option<String> {
    let page = c.site.page(slug)?;
    let (nav, side) = if slug == FAQ {
        (Nav::Faq, String::new())
    } else {
        (Nav::Docs, docs_sidebar(c.site, Some(slug)))
    };
    let class = if side.is_empty() { "single" } else { "split" };
    let body = format!(
        "<div class=\"{class}\">\n{side}\n<main class=\"doc\">\n{}\n</main>\n</div>",
        page.html
    );
    Some(layout(c, &page.title, nav, &body))
}

/// The documentation's index: every page by group.
pub fn docs_index(c: &Chrome<'_>) -> String {
    let mut list = String::new();
    for (g, name) in c.site.groups().enumerate() {
        let _ = writeln!(list, "<h2>{name}</h2>\n<ul>");
        for p in c
            .site
            .pages
            .iter()
            .filter(|p| p.group == g && p.slug != FAQ)
        {
            let _ = writeln!(
                list,
                "<li><a href=\"{}/docs/{}\">{}</a></li>",
                c.site.base,
                p.slug,
                escape(&p.title)
            );
        }
        list.push_str("</ul>\n");
    }
    let body = format!(
        "<div class=\"split\">\n{}\n<main class=\"doc\">\n<h1>Documentation</h1>\n{list}</main>\n</div>",
        docs_sidebar(c.site, None)
    );
    layout(c, "Documentation", Nav::Docs, &body)
}

/// A short page with a message (not found, not built yet).
pub fn notice(c: &Chrome<'_>, title: &str, text: &str) -> String {
    let base = &c.site.base;
    let body = format!(
        "<main class=\"narrow\"><h1>{}</h1><p>{}</p>\
         <p class=\"hint\">Go to <a href=\"{base}/\">Home</a>, the \
         <a href=\"{base}/docs\">documentation</a>, the <a href=\"{base}/faq\">FAQ</a>, the \
         <a href=\"{base}/settings\">settings</a>, or see \
         <a href=\"{base}/docs/install#connecting\">how to connect</a>.</p></main>",
        escape(title),
        escape(text)
    );
    layout(c, title, Nav::None, &body)
}

/// The sign-in form.
pub fn login(c: &Chrome<'_>, error: Option<&str>, username: &str) -> String {
    let error = error.map_or(String::new(), |e| {
        format!("<p class=\"banner error\">{}</p>", escape(e))
    });
    let body = format!(
        "<main class=\"narrow\">\n<h1>Sign in</h1>\n\
         <p>The settings are the sysop's. Sign in with the name and password of a game \
         account that has the sysop role.</p>\n{error}\n\
         <form method=\"post\" action=\"{}/login\" class=\"login\">\n\
         <label>Account <input name=\"username\" value=\"{}\" autocomplete=\"username\" required autofocus></label>\n\
         <label>Password <input name=\"password\" type=\"password\" autocomplete=\"current-password\" required></label>\n\
         <button type=\"submit\">Sign in</button>\n</form>\n\
         <p class=\"hint\">No sysop account yet? On the server: <code>silvermere account create NAME --sysop</code> \
         (<a href=\"{base}/docs/auth#command-line\">account commands</a>). \
         <a href=\"{base}/docs/web#settings\">About signing in</a>.</p>\n\
         </main>",
        c.site.base,
        escape(username),
        base = c.site.base
    );
    layout(c, "Sign in", Nav::Settings, &body)
}

/// The section a key is listed under: its first part, or `general` for a key with none.
pub fn section(key: &str) -> &str {
    key.split_once('.').map_or("general", |(s, _)| s)
}

/// The sections in the order the keys first name them.
fn sections<'a>(keys: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for k in keys {
        let s = section(k);
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

fn settings_sidebar(sections: &[&str]) -> String {
    let mut out = String::from(
        "<aside>\n<input id=\"filter\" type=\"search\" placeholder=\"Filter settings\" \
         aria-label=\"Filter settings\">\n<ul>\n",
    );
    for s in sections {
        let _ = writeln!(out, "<li><a href=\"#{s}\">[{s}]</a></li>");
    }
    out.push_str("</ul>\n</aside>");
    out
}

/// Why a setting is locked, with where to change it instead.
fn lock_text(base: &str, key: &str, lock: &Lock) -> String {
    let wins = format!("<a href=\"{base}/docs/config#precedence\">wins over the file</a>");
    match lock {
        Lock::Secret if key.starts_with("admin.") => format!(
            "Holds secrets, so it is neither shown nor changed here. Make a token on the \
             server with <code>silvermere admin token new</code>: see \
             <a href=\"{base}/docs/admin-api#tokens\">Tokens</a>."
        ),
        Lock::Secret => format!(
            "Can hold a password, so it is neither shown nor changed here. Change it on the \
             server: see <a href=\"{base}/docs/config#the-config-command\">Configuration</a>."
        ),
        Lock::DataDir => format!(
            "Where the settings file itself lives. Change it with <code>--data-dir</code> or \
             <code>SILVERMERE_DATA_DIR</code>: see \
             <a href=\"{base}/docs/config#precedence\">Configuration</a>."
        ),
        Lock::Env(var) => format!(
            "Set by the environment variable <code>{}</code>, which {wins}. Remove it to \
             change the value here.",
            escape(var)
        ),
        Lock::Flag => format!("Set by a flag on the server's command line, which {wins}."),
    }
}

/// The original game's option a key replaces, linked to the page that lists them all.
fn original_option(base: &str, original: &str) -> String {
    format!(
        " · original option <a href=\"{base}/docs/config-defaults\"><code>{}</code></a>",
        escape(original)
    )
}

/// The field a value is edited in.
fn field(v: &SettingValue) -> String {
    let name = escape(&v.key);
    let off = if v.locked.is_some() { " disabled" } else { "" };
    match v.kind {
        Kind::Bool => {
            let sel = |want: &str| if v.value == want { " selected" } else { "" };
            format!(
                "<select name=\"{name}\"{off}><option value=\"true\"{}>true</option>\
                 <option value=\"false\"{}>false</option></select>",
                sel("true"),
                sel("false")
            )
        }
        Kind::Integer => format!(
            "<input name=\"{name}\" value=\"{}\" inputmode=\"numeric\" pattern=\"-?[0-9]*\"{off}>",
            escape(&v.value)
        ),
        Kind::Text | Kind::Toml => format!(
            "<input name=\"{name}\" value=\"{}\" spellcheck=\"false\"{off}>",
            escape(&v.value)
        ),
    }
}

/// The settings editor: one form per section.
pub fn settings(c: &Chrome<'_>, view: &SettingsView, banner: Option<(&str, bool)>) -> String {
    let csrf = c.user.map_or("", |(_, t)| t);
    let secs = sections(view.values.iter().map(|v| v.key.as_str()));
    let mut main = String::from("<h1>Settings</h1>\n");
    if let Some((text, error)) = banner {
        let class = if error { "banner error" } else { "banner ok" };
        let _ = writeln!(main, "<p class=\"{class}\">{}</p>", escape(text));
    }
    if view.pending {
        main.push_str(
            "<p class=\"banner warn\">The settings file has changes this server has not \
             loaded: settings are read once, at startup. Restart the server to apply them \
             (<a href=\"BASE/docs/config#the-config-command\">how</a>).</p>\n"
                .replace("BASE", &c.site.base)
                .as_str(),
        );
    }
    let _ = writeln!(
        main,
        "<p class=\"hint\">Saved to <code>{}</code>. A value set by an environment variable \
         or a flag <a href=\"{base}/docs/config#precedence\">wins over the file</a> and cannot be \
         changed here. <a href=\"{base}/docs/web#settings\">How this page works</a>.</p>",
        escape(&view.file),
        base = c.site.base
    );
    for s in &secs {
        let _ = writeln!(
            main,
            "<section id=\"{s}\">\n<form method=\"post\" action=\"{}/settings\">\n\
             <input type=\"hidden\" name=\"csrf\" value=\"{}\">\n\
             <input type=\"hidden\" name=\"section\" value=\"{s}\">\n\
             <div class=\"bar\"><h2>[{s}]</h2><button type=\"submit\">Save [{s}]</button></div>",
            c.site.base,
            escape(csrf)
        );
        for v in view.values.iter().filter(|v| section(&v.key) == *s) {
            let doc = c.site.key(&v.key);
            let badge = match &v.source {
                Source::Default => String::new(),
                Source::File => "<span class=\"badge\">set in the file</span>".to_owned(),
                Source::Env(var) => {
                    format!("<span class=\"badge lock\">set by {}</span>", escape(var))
                }
                Source::Flag => "<span class=\"badge lock\">set by a flag</span>".to_owned(),
            };
            let locked = v.locked.as_ref().map_or(String::new(), |why| {
                format!(
                    "<p class=\"locked\">{}</p>",
                    lock_text(&c.site.base, &v.key, why)
                )
            });
            let original = doc
                .filter(|d| !d.original.is_empty())
                .map_or(String::new(), |d| {
                    original_option(&c.site.base, &d.original)
                });
            let _ = writeln!(
                main,
                "<div class=\"setting\" data-key=\"{key}\">\n\
                 <label for=\"f-{key}\"><code>{key}</code> {badge}</label>\n\
                 <div class=\"value\">{}</div>\n\
                 <div class=\"about\"><p>{}</p>{locked}\
                 <p class=\"meta\">default {}{original}</p></div>\n</div>",
                field(v).replacen("name=", &format!("id=\"f-{}\" name=", escape(&v.key)), 1),
                doc.map_or("", |d| d.meaning.as_str()),
                if v.default.is_empty() {
                    "<em>empty</em>".to_owned()
                } else {
                    format!("<code>{}</code>", escape(&v.default))
                },
                key = escape(&v.key),
            );
        }
        main.push_str("</form>\n</section>\n");
    }
    let body = format!(
        "<div class=\"split\">\n{}\n<main class=\"settings\">\n{main}</main>\n</div>",
        settings_sidebar(&secs)
    );
    layout(c, "Settings", Nav::Settings, &body)
}

/// The settings as a reference table, for a copy with no server behind it.
pub fn settings_reference(c: &Chrome<'_>) -> String {
    let keys: &[(String, KeyDoc)] = &c.site.keys;
    let secs = sections(keys.iter().map(|(k, _)| k.as_str()));
    let mut main = format!(
        "<h1>Settings reference</h1>\n<p class=\"hint\">Every setting of this version. On a \
         running server the same page is an editor; see <a href=\"{}/docs/config\">Configuration</a> \
         for the file, the environment variables and the flags.</p>\n",
        c.site.base
    );
    for s in &secs {
        let _ = writeln!(
            main,
            "<section id=\"{s}\">\n<div class=\"bar\"><h2>[{s}]</h2></div>"
        );
        for (key, d) in keys.iter().filter(|(k, _)| section(k) == *s) {
            let original = if d.original.is_empty() {
                String::new()
            } else {
                original_option(&c.site.base, &d.original)
            };
            let _ = writeln!(
                main,
                "<div class=\"setting\" data-key=\"{key}\">\n<label><code>{key}</code></label>\n\
                 <div class=\"value\"><code>{}</code></div>\n\
                 <div class=\"about\"><p>{}</p><p class=\"meta\"><code>{}</code>{original}</p></div>\n</div>",
                escape(&d.default),
                d.meaning,
                escape(&d.env),
                key = escape(key),
            );
        }
        main.push_str("</section>\n");
    }
    let body = format!(
        "<div class=\"split\">\n{}\n<main class=\"settings\">\n{main}</main>\n</div>",
        settings_sidebar(&secs)
    );
    layout(c, "Settings reference", Nav::Settings, &body)
}
