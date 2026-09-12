use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::read::GzDecoder;
use rusqlite::{params, Connection, OpenFlags, Transaction};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::config::{RetrievalConfig, SourceConfig, SourceKindConfig};
use crate::error::{Result, RetrievalError};
use crate::model::{IndexReport, SourceKind, SourceReport};
use crate::path_guard::Allowlist;
use crate::{AUTHORITY_BOUNDARY, COMMON_PROTOCOL_ID, ROUTER_CONTEXT_ID};

#[derive(Debug, Clone)]
struct CollectedDocument {
    path: String,
    source_name: String,
    source_kind: SourceKind,
    trust_label: String,
    package_name: Option<String>,
    package_version: Option<String>,
    content: String,
    content_hash: String,
    byte_len: usize,
    modified_unix: i64,
}

#[derive(Debug, Clone)]
struct Budget {
    max_file_count: usize,
    max_total_bytes: usize,
    consumed_files: usize,
    consumed_bytes: usize,
}

impl Budget {
    fn new(max_file_count: usize, max_total_bytes: usize) -> Self {
        Self {
            max_file_count,
            max_total_bytes,
            consumed_files: 0,
            consumed_bytes: 0,
        }
    }

    fn try_consume(&mut self, bytes: usize) -> bool {
        if self.consumed_files >= self.max_file_count {
            return false;
        }

        if self.consumed_bytes.saturating_add(bytes) > self.max_total_bytes {
            return false;
        }

        self.consumed_files += 1;
        self.consumed_bytes += bytes;
        true
    }
}

pub struct Indexer {
    index_path: PathBuf,
}

impl Indexer {
    pub fn new(index_path: impl Into<PathBuf>) -> Self {
        Self {
            index_path: index_path.into(),
        }
    }

    pub fn rebuild(&self, config: &RetrievalConfig) -> Result<IndexReport> {
        verify_non_root_identity()?;
        config.validate()?;
        ensure_parent_directory(&self.index_path)?;

        let mut conn = open_index_rw(&self.index_path)?;
        initialize_schema(&conn)?;

        let mut budget = Budget::new(config.max_file_count, config.max_total_bytes);
        let mut sources = Vec::new();

        let tx = conn.transaction()?;
        for source in &config.source {
            let mut source_report = SourceReport::new(
                source.name.clone(),
                source.kind.as_model(),
                source.trust_label(),
                source.configured_roots(),
            );

            let docs = collect_documents_for_source(source, config, &mut budget, &mut source_report)?;
            apply_source_documents(&tx, source, docs, &mut source_report)?;
            sources.push(source_report);
        }
        tx.commit()?;

        quick_check(&conn, &self.index_path)?;
        set_secure_permissions(&self.index_path)?;

        let mut report = IndexReport {
            protocol_id: COMMON_PROTOCOL_ID,
            router_contract_id: ROUTER_CONTEXT_ID,
            index_path: self.index_path.display().to_string(),
            authority_boundary: AUTHORITY_BOUNDARY,
            max_file_count: config.max_file_count,
            max_file_bytes: config.max_file_bytes,
            max_total_bytes: config.max_total_bytes,
            indexed_total: 0,
            inserted_total: 0,
            updated_total: 0,
            removed_total: 0,
            skipped_total: 0,
            sources,
        };

        for source in &report.sources {
            report.indexed_total += source.indexed_documents;
            report.inserted_total += source.inserted;
            report.updated_total += source.updated;
            report.removed_total += source.removed;
            report.skipped_total += source.skipped_budget
                + source.skipped_outside_allowlist
                + source.skipped_oversize
                + source.skipped_unreadable;
        }

        Ok(report)
    }
}

fn verify_non_root_identity() -> Result<()> {
    // Indexing should run as the dedicated unprivileged sentia-index identity.
    if unsafe { libc::geteuid() } == 0 {
        return Err(RetrievalError::RootIndexDenied);
    }
    Ok(())
}

fn ensure_parent_directory(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| RetrievalError::InvalidConfig("index path must have a parent directory".into()))?;

    fs::create_dir_all(parent).map_err(|source| RetrievalError::Io {
        path: parent.to_path_buf(),
        source,
    })?;

    Ok(())
}

fn open_index_rw(index_path: &Path) -> Result<Connection> {
    Connection::open_with_flags(
        index_path,
        OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| map_sqlite_error(index_path, err))
}

fn initialize_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA foreign_keys=ON;

         CREATE TABLE IF NOT EXISTS index_metadata (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );

         CREATE TABLE IF NOT EXISTS documents (
             id INTEGER PRIMARY KEY,
             path TEXT NOT NULL UNIQUE,
             source_name TEXT NOT NULL,
             source_kind TEXT NOT NULL,
             trust_label TEXT NOT NULL,
             package_name TEXT,
             package_version TEXT,
             content TEXT NOT NULL,
             content_hash TEXT NOT NULL,
             byte_len INTEGER NOT NULL,
             modified_unix INTEGER NOT NULL,
             indexed_unix INTEGER NOT NULL
         );

         CREATE INDEX IF NOT EXISTS documents_source_idx ON documents(source_name);
         CREATE INDEX IF NOT EXISTS documents_package_idx ON documents(package_name, package_version);

         CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(
             content,
             content='documents',
             content_rowid='id',
             tokenize='unicode61 remove_diacritics 2'
         );

         CREATE TRIGGER IF NOT EXISTS documents_ai AFTER INSERT ON documents BEGIN
             INSERT INTO documents_fts(rowid, content) VALUES (new.id, new.content);
         END;

         CREATE TRIGGER IF NOT EXISTS documents_ad AFTER DELETE ON documents BEGIN
             INSERT INTO documents_fts(documents_fts, rowid, content)
             VALUES('delete', old.id, old.content);
         END;

         CREATE TRIGGER IF NOT EXISTS documents_au AFTER UPDATE ON documents BEGIN
             INSERT INTO documents_fts(documents_fts, rowid, content)
             VALUES('delete', old.id, old.content);
             INSERT INTO documents_fts(rowid, content) VALUES (new.id, new.content);
         END;",
    )
    .map_err(RetrievalError::from)?;

    conn.execute(
        "INSERT OR REPLACE INTO index_metadata(key, value) VALUES ('schema_version', '1')",
        [],
    )
    .map_err(RetrievalError::from)?;

    conn.execute(
        "INSERT OR REPLACE INTO index_metadata(key, value) VALUES ('authority_boundary', ?1)",
        [AUTHORITY_BOUNDARY],
    )
    .map_err(RetrievalError::from)?;

    Ok(())
}

fn quick_check(conn: &Connection, index_path: &Path) -> Result<()> {
    let result: String = conn
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(|err| map_sqlite_error(index_path, err))?;

    if result != "ok" {
        return Err(RetrievalError::CorruptDatabase {
            path: index_path.to_path_buf(),
            details: result,
        });
    }

    Ok(())
}

fn set_secure_permissions(index_path: &Path) -> Result<()> {
    let permissions = fs::Permissions::from_mode(0o640);
    fs::set_permissions(index_path, permissions).map_err(|source| RetrievalError::Io {
        path: index_path.to_path_buf(),
        source,
    })?;
    Ok(())
}

fn collect_documents_for_source(
    source: &SourceConfig,
    config: &RetrievalConfig,
    budget: &mut Budget,
    report: &mut SourceReport,
) -> Result<Vec<CollectedDocument>> {
    match source.kind {
        SourceKindConfig::Filesystem => collect_filesystem_documents(source, config, budget, report),
        SourceKindConfig::DpkgStatus => collect_dpkg_documents(source, config, budget, report),
        SourceKindConfig::ManDirectory => collect_man_documents(source, config, budget, report),
    }
}

fn collect_filesystem_documents(
    source: &SourceConfig,
    config: &RetrievalConfig,
    budget: &mut Budget,
    report: &mut SourceReport,
) -> Result<Vec<CollectedDocument>> {
    let mut docs = Vec::new();

    for root in &source.roots {
        if !root.exists() {
            report.skipped_unreadable += 1;
            continue;
        }

        let allowlist = match Allowlist::for_root(root) {
            Ok(allowlist) => allowlist,
            Err(_) => {
                report.skipped_unreadable += 1;
                continue;
            }
        };

        if root.is_file() {
            maybe_collect_document(
                source,
                config,
                budget,
                report,
                root,
                &allowlist,
                false,
                &mut docs,
            );
            continue;
        }

        for entry in WalkDir::new(root)
            .follow_links(false)
            .max_depth(source.max_depth)
            .into_iter()
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    report.skipped_unreadable += 1;
                    continue;
                }
            };

            if !entry.file_type().is_file() && !entry.file_type().is_symlink() {
                continue;
            }

            maybe_collect_document(
                source,
                config,
                budget,
                report,
                entry.path(),
                &allowlist,
                false,
                &mut docs,
            );
        }
    }

    Ok(docs)
}

fn collect_man_documents(
    source: &SourceConfig,
    config: &RetrievalConfig,
    budget: &mut Budget,
    report: &mut SourceReport,
) -> Result<Vec<CollectedDocument>> {
    let mut docs = Vec::new();
    let allowed_pages = source
        .allow_pages
        .iter()
        .map(|page| page.to_ascii_lowercase())
        .collect::<HashSet<_>>();

    for root in &source.roots {
        if !root.exists() {
            report.skipped_unreadable += 1;
            continue;
        }

        let allowlist = match Allowlist::for_root(root) {
            Ok(allowlist) => allowlist,
            Err(_) => {
                report.skipped_unreadable += 1;
                continue;
            }
        };

        for entry in WalkDir::new(root)
            .follow_links(false)
            .max_depth(source.max_depth)
            .into_iter()
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    report.skipped_unreadable += 1;
                    continue;
                }
            };

            if !entry.file_type().is_file() && !entry.file_type().is_symlink() {
                continue;
            }

            if !is_manpage_file(entry.path()) {
                continue;
            }

            if !allowed_pages.is_empty() {
                let page_name = extract_manpage_name(entry.path());
                if !allowed_pages.contains(&page_name) {
                    continue;
                }
            }

            maybe_collect_document(
                source,
                config,
                budget,
                report,
                entry.path(),
                &allowlist,
                true,
                &mut docs,
            );
        }
    }

    Ok(docs)
}

fn collect_dpkg_documents(
    source: &SourceConfig,
    config: &RetrievalConfig,
    budget: &mut Budget,
    report: &mut SourceReport,
) -> Result<Vec<CollectedDocument>> {
    let mut docs = Vec::new();
    let Some(path) = source.path.as_ref() else {
        return Err(RetrievalError::InvalidConfig(format!(
            "dpkg_status source {} is missing path",
            source.name
        )));
    };

    let metadata = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(_) => {
            report.skipped_unreadable += 1;
            return Ok(docs);
        }
    };

    if metadata.len() as usize > config.max_file_bytes {
        report.skipped_oversize += 1;
        return Ok(docs);
    }

    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => {
            report.skipped_unreadable += 1;
            return Ok(docs);
        }
    };

    let modified = metadata
        .modified()
        .ok()
        .and_then(unix_seconds)
        .unwrap_or_else(now_unix_seconds);

    for package in parse_dpkg_status(&text) {
        report.scanned_files += 1;

        let package_name = package
            .get("Package")
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".into());
        let package_version = package
            .get("Version")
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".into());
        let status = package
            .get("Status")
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();

        if !status.contains("installed") {
            continue;
        }

        let mut content = String::new();
        for key in [
            "Package",
            "Version",
            "Architecture",
            "Priority",
            "Section",
            "Depends",
            "Description",
        ] {
            if let Some(value) = package.get(key) {
                content.push_str(key);
                content.push_str(": ");
                content.push_str(value.trim());
                content.push('\n');
            }
        }

        if content.trim().is_empty() {
            continue;
        }

        let byte_len = content.len();
        if byte_len > config.max_file_bytes {
            report.skipped_oversize += 1;
            continue;
        }

        if !budget.try_consume(byte_len) {
            report.skipped_budget += 1;
            continue;
        }

        let synthetic_path = format!("{}#{}", path.display(), package_name);
        docs.push(CollectedDocument {
            path: synthetic_path,
            source_name: source.name.clone(),
            source_kind: SourceKind::DpkgStatus,
            trust_label: source.trust_label(),
            package_name: Some(package_name),
            package_version: Some(package_version),
            content_hash: hash_content(&content),
            content,
            byte_len,
            modified_unix: modified,
        });
    }

    Ok(docs)
}

fn maybe_collect_document(
    source: &SourceConfig,
    config: &RetrievalConfig,
    budget: &mut Budget,
    report: &mut SourceReport,
    candidate: &Path,
    allowlist: &Allowlist,
    is_manpage: bool,
    docs: &mut Vec<CollectedDocument>,
) {
    report.scanned_files += 1;

    if !source.include_extensions.is_empty() && !matches_extension(candidate, &source.include_extensions) {
        return;
    }

    let resolved = match allowlist.resolve_if_allowed(candidate) {
        Ok(Some(path)) => path,
        Ok(None) => {
            report.skipped_outside_allowlist += 1;
            return;
        }
        Err(_) => {
            report.skipped_unreadable += 1;
            return;
        }
    };

    let metadata = match fs::metadata(&resolved) {
        Ok(meta) => meta,
        Err(_) => {
            report.skipped_unreadable += 1;
            return;
        }
    };

    if !metadata.is_file() {
        return;
    }

    if metadata.len() as usize > config.max_file_bytes {
        report.skipped_oversize += 1;
        return;
    }

    let (content, byte_len) = if is_manpage {
        match read_manpage_without_macro_execution(&resolved) {
            Ok(content) => {
                let byte_len = content.len();
                (content, byte_len)
            }
            Err(_) => {
                report.skipped_unreadable += 1;
                return;
            }
        }
    } else {
        let bytes = match fs::read(&resolved) {
            Ok(bytes) => bytes,
            Err(_) => {
                report.skipped_unreadable += 1;
                return;
            }
        };
        let content = String::from_utf8_lossy(&bytes).to_string();
        let byte_len = content.len();
        (content, byte_len)
    };

    if content.trim().is_empty() {
        return;
    }

    if byte_len > config.max_file_bytes {
        report.skipped_oversize += 1;
        return;
    }

    if !budget.try_consume(byte_len) {
        report.skipped_budget += 1;
        return;
    }

    let modified_unix = metadata
        .modified()
        .ok()
        .and_then(unix_seconds)
        .unwrap_or_else(now_unix_seconds);

    docs.push(CollectedDocument {
        path: resolved.display().to_string(),
        source_name: source.name.clone(),
        source_kind: source.kind.as_model(),
        trust_label: source.trust_label(),
        package_name: source.package_name.clone(),
        package_version: source.package_version.clone(),
        content_hash: hash_content(&content),
        content,
        byte_len,
        modified_unix,
    });
}

fn apply_source_documents(
    tx: &Transaction<'_>,
    source: &SourceConfig,
    docs: Vec<CollectedDocument>,
    report: &mut SourceReport,
) -> Result<()> {
    let existing = load_existing_hashes(tx, &source.name)?;
    let mut seen_paths = HashSet::new();

    let mut upsert = tx.prepare(
        "INSERT INTO documents (
             path,
             source_name,
             source_kind,
             trust_label,
             package_name,
             package_version,
             content,
             content_hash,
             byte_len,
             modified_unix,
             indexed_unix
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(path) DO UPDATE SET
             source_name=excluded.source_name,
             source_kind=excluded.source_kind,
             trust_label=excluded.trust_label,
             package_name=excluded.package_name,
             package_version=excluded.package_version,
             content=excluded.content,
             content_hash=excluded.content_hash,
             byte_len=excluded.byte_len,
             modified_unix=excluded.modified_unix,
             indexed_unix=excluded.indexed_unix",
    )?;

    let indexed_at = now_unix_seconds();

    for doc in docs {
        report.indexed_documents += 1;
        seen_paths.insert(doc.path.clone());

        if let Some(existing_hash) = existing.get(&doc.path) {
            if existing_hash == &doc.content_hash {
                report.unchanged += 1;
                continue;
            }
            report.updated += 1;
        } else {
            report.inserted += 1;
        }

        upsert.execute(params![
            doc.path,
            doc.source_name,
            source_kind_to_string(doc.source_kind),
            doc.trust_label,
            doc.package_name,
            doc.package_version,
            doc.content,
            doc.content_hash,
            doc.byte_len as i64,
            doc.modified_unix,
            indexed_at,
        ])?;
    }

    let stale_paths = existing
        .keys()
        .filter(|path| !seen_paths.contains(*path))
        .cloned()
        .collect::<Vec<_>>();

    let mut delete_stmt = tx.prepare("DELETE FROM documents WHERE source_name = ?1 AND path = ?2")?;
    for stale in stale_paths {
        delete_stmt.execute(params![source.name, stale])?;
        report.removed += 1;
    }

    Ok(())
}

fn load_existing_hashes(tx: &Transaction<'_>, source_name: &str) -> Result<HashMap<String, String>> {
    let mut stmt = tx.prepare("SELECT path, content_hash FROM documents WHERE source_name = ?1")?;
    let rows = stmt.query_map([source_name], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut values = HashMap::new();
    for row in rows {
        let (path, hash) = row?;
        values.insert(path, hash);
    }

    Ok(values)
}

fn parse_dpkg_status(content: &str) -> Vec<HashMap<String, String>> {
    let mut packages = Vec::new();

    for stanza in content.split("\n\n") {
        let stanza = stanza.trim();
        if stanza.is_empty() {
            continue;
        }

        let mut fields: HashMap<String, String> = HashMap::new();
        let mut current_key: Option<String> = None;

        for line in stanza.lines() {
            if line.starts_with(' ') || line.starts_with('\t') {
                if let Some(key) = &current_key {
                    if let Some(existing) = fields.get_mut(key) {
                        existing.push('\n');
                        existing.push_str(line.trim());
                    }
                }
                continue;
            }

            let Some((raw_key, raw_value)) = line.split_once(':') else {
                continue;
            };

            let key = raw_key.trim().to_string();
            let value = raw_value.trim().to_string();
            current_key = Some(key.clone());
            fields.insert(key, value);
        }

        if !fields.is_empty() {
            packages.push(fields);
        }
    }

    packages
}

fn is_manpage_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    name.ends_with(".1")
        || name.ends_with(".2")
        || name.ends_with(".3")
        || name.ends_with(".4")
        || name.ends_with(".5")
        || name.ends_with(".6")
        || name.ends_with(".7")
        || name.ends_with(".8")
        || name.ends_with(".1.gz")
        || name.ends_with(".2.gz")
        || name.ends_with(".3.gz")
        || name.ends_with(".4.gz")
        || name.ends_with(".5.gz")
        || name.ends_with(".6.gz")
        || name.ends_with(".7.gz")
        || name.ends_with(".8.gz")
}

fn extract_manpage_name(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_default();

    let without_gz = name.strip_suffix(".gz").unwrap_or(&name);
    without_gz
        .split('.')
        .next()
        .unwrap_or(without_gz)
        .to_ascii_lowercase()
}

fn read_manpage_without_macro_execution(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|source| RetrievalError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let raw = if path
        .extension()
        .map(|ext| ext.eq_ignore_ascii_case("gz"))
        .unwrap_or(false)
    {
        let mut decoder = GzDecoder::new(bytes.as_slice());
        let mut decoded = String::new();
        decoder
            .read_to_string(&mut decoded)
            .map_err(|source| RetrievalError::io(path, source))?;
        decoded
    } else {
        String::from_utf8_lossy(&bytes).to_string()
    };

    Ok(sanitize_manpage(&raw))
}

fn sanitize_manpage(raw: &str) -> String {
    let mut out = String::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Never execute roff macros; keep literal text lines only.
        if trimmed.starts_with('.') || trimmed.starts_with('\'') {
            continue;
        }

        let clean = trimmed
            .replace("\\-", "-")
            .replace("\\fB", "")
            .replace("\\fI", "")
            .replace("\\fR", "")
            .replace("\\(aq", "'");

        out.push_str(clean.trim());
        out.push('\n');
    }

    out
}

fn source_kind_to_string(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Filesystem => "filesystem",
        SourceKind::DpkgStatus => "dpkg_status",
        SourceKind::ManDirectory => "man_directory",
    }
}

fn matches_extension(path: &Path, allowed: &[String]) -> bool {
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    allowed.iter().any(|ext| {
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        file_name == ext
            || file_name.ends_with(&format!(".{}", ext))
            || file_name.ends_with(ext.as_str())
    })
}

fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    hex_encode(digest.as_slice())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn now_unix_seconds() -> i64 {
    unix_seconds(SystemTime::now()).unwrap_or(0)
}

fn unix_seconds(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs() as i64)
}

fn map_sqlite_error(index_path: &Path, error: rusqlite::Error) -> RetrievalError {
    if let rusqlite::Error::SqliteFailure(err, message) = &error {
        if err.code == rusqlite::ErrorCode::DatabaseCorrupt
            || err.code == rusqlite::ErrorCode::NotADatabase
        {
            return RetrievalError::CorruptDatabase {
                path: index_path.to_path_buf(),
                details: message
                    .clone()
                    .unwrap_or_else(|| format!("sqlite error code {:?}", err.code)),
            };
        }
    }

    RetrievalError::Sqlite(error)
}

#[cfg(test)]
mod tests {
    use super::{parse_dpkg_status, sanitize_manpage};

    #[test]
    fn parses_dpkg_paragraphs_with_multiline_fields() {
        let parsed = parse_dpkg_status(
            "Package: sentia\nVersion: 1.0\nDescription: first line\n second line\n\nPackage: demo\nVersion: 2\nStatus: install ok installed\n",
        );

        assert_eq!(parsed.len(), 2);
        assert!(parsed[0]["Description"].contains("second line"));
    }

    #[test]
    fn strips_manpage_macros_without_execution() {
        let sanitized = sanitize_manpage(".TH TEST 1\nNormal text\n.so /etc/passwd\n' comment\n");
        assert_eq!(sanitized.trim(), "Normal text");
    }
}
