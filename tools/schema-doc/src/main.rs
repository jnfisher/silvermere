//! Renders `docs/schema.md` from `migrations/sqlite/*.sql` and `migrations/postgres/*.sql`.
//!
//! The migrations are written one column per line with the comment after `--` and the
//! table comment directly above `CREATE TABLE`; this reads that shape (no SQL parser),
//! checks that both dialects declare the same tables, columns, constraints and indexes,
//! and writes one document with each column's type in both dialects. `--check` compares
//! with the committed file instead and exits 1 when it is stale.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Column {
    name: String,
    sql_type: String,
    nullable: bool,
    primary_key: bool,
    references: Option<String>,
    comment: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Index {
    name: String,
    unique: bool,
    columns: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Table {
    name: String,
    description: Vec<String>,
    columns: Vec<Column>,
    constraints: Vec<String>,
    indexes: Vec<Index>,
}

#[derive(Debug, Default)]
struct Schema {
    tables: Vec<Table>,
}

const TABLE_CONSTRAINT_WORDS: &[&str] = &["PRIMARY", "UNIQUE", "FOREIGN", "CHECK", "CONSTRAINT"];
const TYPE_END_WORDS: &[&str] = &[
    "NOT",
    "NULL",
    "PRIMARY",
    "REFERENCES",
    "DEFAULT",
    "UNIQUE",
    "COLLATE",
];

fn parse(sql: &str, file: &str) -> Result<Schema, String> {
    let mut schema = Schema::default();
    let mut pending: Vec<String> = Vec::new();
    let mut current: Option<Table> = None;
    for (lineno, raw) in sql.lines().enumerate() {
        let line = raw.trim_end();
        let where_ = || format!("{file}:{}", lineno + 1);
        if let Some(table) = current.as_mut() {
            if line.starts_with(");") {
                schema.tables.push(current.take().expect("table"));
                continue;
            }
            let body = line.trim();
            if body.is_empty() {
                continue;
            }
            let (def, comment) = match body.split_once("--") {
                Some((d, c)) => (d.trim(), c.trim().to_string()),
                None => (body, String::new()),
            };
            let def = def.trim_end_matches(',').trim();
            let mut words = def.split_whitespace();
            let first = words
                .next()
                .ok_or_else(|| format!("{}: empty definition", where_()))?;
            if TABLE_CONSTRAINT_WORDS.contains(&first) {
                table.constraints.push(def.to_string());
                continue;
            }
            let rest: Vec<&str> = words.collect();
            let type_words: Vec<&str> = rest
                .iter()
                .take_while(|w| !TYPE_END_WORDS.contains(&w.to_uppercase().as_str()))
                .copied()
                .collect();
            if type_words.is_empty() {
                return Err(format!("{}: column `{first}` has no type", where_()));
            }
            let upper = def.to_uppercase();
            let primary_key = upper.contains("PRIMARY KEY");
            let nullable = !upper.contains("NOT NULL") && !primary_key;
            let references = def.find("REFERENCES ").map(|i| {
                let tail = &def[i + "REFERENCES ".len()..];
                let end = tail.find(')').map_or(tail.len(), |e| e + 1);
                tail[..end].replace(" (", ".").replace(')', "")
            });
            if comment.is_empty() {
                return Err(format!("{}: column `{first}` has no comment", where_()));
            }
            table.columns.push(Column {
                name: first.to_string(),
                sql_type: type_words.join(" "),
                nullable,
                primary_key,
                references,
                comment,
            });
            continue;
        }
        if let Some(rest) = line.strip_prefix("CREATE TABLE ") {
            let name = rest
                .split_whitespace()
                .next()
                .ok_or_else(|| format!("{}: CREATE TABLE without a name", where_()))?;
            if pending.is_empty() {
                return Err(format!(
                    "{}: table `{name}` has no comment above it",
                    where_()
                ));
            }
            current = Some(Table {
                name: name.to_string(),
                description: std::mem::take(&mut pending),
                ..Table::default()
            });
            continue;
        }
        if line.starts_with("CREATE UNIQUE INDEX ") || line.starts_with("CREATE INDEX ") {
            let unique = line.starts_with("CREATE UNIQUE");
            let after = line
                .trim_start_matches("CREATE UNIQUE INDEX ")
                .trim_start_matches("CREATE INDEX ");
            let (name, rest) = after
                .split_once(" ON ")
                .ok_or_else(|| format!("{}: index without ON", where_()))?;
            let (table_name, cols) = rest
                .split_once(' ')
                .ok_or_else(|| format!("{}: index without columns", where_()))?;
            let cols = cols.trim().trim_end_matches(';').to_string();
            let table = schema
                .tables
                .iter_mut()
                .find(|t| t.name == table_name)
                .ok_or_else(|| format!("{}: index on unknown table `{table_name}`", where_()))?;
            table.indexes.push(Index {
                name: name.to_string(),
                unique,
                columns: cols,
            });
            pending.clear();
            continue;
        }
        if let Some(c) = line.strip_prefix("--") {
            let c = c.trim();
            if c.starts_with("====") || c.is_empty() {
                pending.clear();
            } else {
                pending.push(c.to_string());
            }
            continue;
        }
        pending.clear();
    }
    if current.is_some() {
        return Err(format!("{file}: unterminated CREATE TABLE"));
    }
    Ok(schema)
}

/// Both dialects must declare the same tables, columns (names, nullability, keys,
/// comments), constraints and indexes; only the types may differ.
fn check_same_shape(a: &Schema, b: &Schema) -> Result<(), String> {
    let names = |s: &Schema| s.tables.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
    if names(a) != names(b) {
        return Err(format!(
            "table lists differ:\n  sqlite: {:?}\n  postgres: {:?}",
            names(a),
            names(b)
        ));
    }
    for (ta, tb) in a.tables.iter().zip(&b.tables) {
        let cols = |t: &Table| {
            t.columns
                .iter()
                .map(|c| {
                    (
                        c.name.clone(),
                        c.nullable,
                        c.primary_key,
                        c.references.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        if cols(ta) != cols(tb) {
            return Err(format!(
                "columns of `{}` differ between the dialects",
                ta.name
            ));
        }
        let constraints = |t: &Table| {
            t.constraints
                .iter()
                .map(|c| c.replace(" ", ""))
                .collect::<Vec<_>>()
        };
        if constraints(ta) != constraints(tb) {
            return Err(format!(
                "constraints of `{}` differ between the dialects",
                ta.name
            ));
        }
        let indexes = |t: &Table| {
            t.indexes
                .iter()
                .map(|i| (i.name.clone(), i.unique))
                .collect::<Vec<_>>()
        };
        if indexes(ta) != indexes(tb) {
            return Err(format!(
                "indexes of `{}` differ between the dialects",
                ta.name
            ));
        }
    }
    Ok(())
}

fn render(sqlite: &Schema, postgres: &Schema, sources: &[String]) -> String {
    let mut out = String::new();
    out.push_str("# silvermere database schema\n\n");
    out.push_str(
        "<!-- Generated by tools/schema-doc from the migrations; do not edit. Regenerate with\n",
    );
    out.push_str("     `cargo run --manifest-path tools/schema-doc/Cargo.toml`; CI checks it is current. -->\n\n");
    out.push_str("Rendered from ");
    out.push_str(
        &sources
            .iter()
            .map(|s| format!("`{s}`"))
            .collect::<Vec<_>>()
            .join(" and "),
    );
    out.push_str("; the queries that use ");
    out.push_str("these tables are the repositories of `crates/silvermere-db`.\n\n");
    out.push_str(&format!("{} tables.\n\n", sqlite.tables.len()));
    out.push_str("| Table | Columns | Description |\n|---|---|---|\n");
    for t in &sqlite.tables {
        let first = t.description.join(" ");
        let first = first.split(". ").next().unwrap_or("").trim_end_matches('.');
        out.push_str(&format!(
            "| [`{}`](#{}) | {} | {} |\n",
            t.name,
            t.name.replace('_', "-"),
            t.columns.len(),
            first
        ));
    }
    out.push('\n');
    let pg: BTreeMap<&str, &Table> = postgres
        .tables
        .iter()
        .map(|t| (t.name.as_str(), t))
        .collect();
    for t in &sqlite.tables {
        let p = pg[t.name.as_str()];
        out.push_str(&format!("## {}\n\n", t.name));
        out.push_str(&t.description.join("\n"));
        out.push_str(
            "\n\n| Column | SQLite | PostgreSQL | Key | Comment |\n|---|---|---|---|---|\n",
        );
        for (c, pc) in t.columns.iter().zip(&p.columns) {
            let mut key = Vec::new();
            if c.primary_key {
                key.push("PK".to_string());
            }
            if let Some(r) = &c.references {
                key.push(format!("→ {r}"));
            }
            if c.nullable {
                key.push("nullable".to_string());
            }
            out.push_str(&format!(
                "| `{}` | {} | {} | {} | {} |\n",
                c.name,
                c.sql_type,
                pc.sql_type,
                key.join(", "),
                c.comment.replace('|', "\\|")
            ));
        }
        if !t.constraints.is_empty() {
            out.push_str("\nConstraints:\n\n");
            for c in &t.constraints {
                out.push_str(&format!("- `{c}`\n"));
            }
        }
        if !t.indexes.is_empty() {
            out.push_str("\nIndexes:\n\n");
            for (i, pi) in t.indexes.iter().zip(&p.indexes) {
                let kind = if i.unique { "unique" } else { "index" };
                if i.columns == pi.columns {
                    out.push_str(&format!("- `{}` {kind} on `{}`\n", i.name, i.columns));
                } else {
                    out.push_str(&format!(
                        "- `{}` {kind} on `{}` (PostgreSQL: `{}`)\n",
                        i.name, i.columns, pi.columns
                    ));
                }
            }
        }
        out.push('\n');
    }
    out
}

fn migration_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "sql"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("{}: no .sql files", dir.display()));
    }
    Ok(files)
}

fn load(dir: &Path, root: &Path) -> Result<(Schema, Vec<String>), String> {
    let mut all = Schema::default();
    let mut names = Vec::new();
    for file in migration_files(dir)? {
        let rel = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .display()
            .to_string();
        let sql = fs::read_to_string(&file).map_err(|e| format!("{rel}: {e}"))?;
        let schema = parse(&sql, &rel)?;
        all.tables.extend(schema.tables);
        names.push(rel);
    }
    Ok((all, names))
}

fn run() -> Result<bool, String> {
    let check = std::env::args().any(|a| a == "--check");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let (sqlite, mut sources) = load(&root.join("migrations/sqlite"), &root)?;
    let (postgres, pg_sources) = load(&root.join("migrations/postgres"), &root)?;
    sources.extend(pg_sources);
    check_same_shape(&sqlite, &postgres)?;
    let rendered = render(&sqlite, &postgres, &sources);
    let target = root.join("docs/schema.md");
    if check {
        let current = fs::read_to_string(&target).unwrap_or_default();
        if current != rendered {
            eprintln!(
                "docs/schema.md is stale: run `cargo run --manifest-path tools/schema-doc/Cargo.toml`"
            );
            return Ok(false);
        }
        println!("docs/schema.md is current ({} tables)", sqlite.tables.len());
    } else {
        fs::write(&target, rendered).map_err(|e| format!("{}: {e}", target.display()))?;
        println!(
            "wrote {} ({} tables)",
            target.display(),
            sqlite.tables.len()
        );
    }
    Ok(true)
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("schema-doc: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "-- header\n\n-- A table.\n-- More.\nCREATE TABLE t (\n    id INTEGER PRIMARY KEY AUTOINCREMENT, -- the id\n    name TEXT NOT NULL COLLATE NOCASE,      -- the name\n    parent INTEGER REFERENCES p (id) ON DELETE CASCADE, -- parent\n    PRIMARY KEY (id, name)\n);\nCREATE UNIQUE INDEX t_name ON t (name);\n";

    #[test]
    fn parses_the_shape() {
        let s = parse(SAMPLE, "x.sql").unwrap();
        assert_eq!(s.tables.len(), 1);
        let t = &s.tables[0];
        assert_eq!(t.description, vec!["A table.", "More."]);
        assert_eq!(t.columns.len(), 3);
        assert_eq!(t.columns[0].sql_type, "INTEGER");
        assert!(t.columns[0].primary_key);
        assert_eq!(t.columns[1].sql_type, "TEXT");
        assert!(!t.columns[1].nullable);
        assert!(t.columns[2].nullable);
        assert_eq!(t.columns[2].references.as_deref(), Some("p.id"));
        assert_eq!(t.constraints, vec!["PRIMARY KEY (id, name)"]);
        assert_eq!(t.indexes[0].columns, "(name)");
        assert!(t.indexes[0].unique);
    }

    #[test]
    fn a_column_without_a_comment_is_an_error() {
        let err = parse("-- c\nCREATE TABLE t (\n    id INTEGER\n);\n", "x.sql").unwrap_err();
        assert!(err.contains("no comment"), "{err}");
    }
}
