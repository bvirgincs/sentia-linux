use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OpenFlags};

use crate::error::{Result, RetrievalError};
use crate::model::{
    Citation, QueryRequest, QueryResponse, QueryResult, RouterContextDocument, RouterContextResponse,
    SourceKind,
};
use crate::{AUTHORITY_BOUNDARY, COMMON_PROTOCOL_ID, ROUTER_CONTEXT_ID};

pub struct QueryEngine {
    index_path: PathBuf,
}

impl QueryEngine {
    pub fn new(index_path: impl Into<PathBuf>) -> Self {
        Self {
            index_path: index_path.into(),
        }
    }

    pub fn query(&self, request: QueryRequest) -> Result<QueryResponse> {
        if request.query.trim().is_empty() {
            return Err(RetrievalError::EmptyQuery);
        }

        verify_read_only_permissions(&self.index_path)?;
        let conn = open_index_ro(&self.index_path)?;
        quick_check(&conn, &self.index_path)?;

        let fts_query = to_safe_fts_query(&request.query)?;
        let limit = request.limit.max(1).min(64);
        let token_window = snippet_tokens_for_chars(request.max_content_chars);

        let mut stmt = conn.prepare(
            "SELECT
                d.path,
                d.source_name,
                d.source_kind,
                d.trust_label,
                d.package_name,
                d.package_version,
                snippet(documents_fts, 0, '', '', ' … ', ?1) AS snip,
                bm25(documents_fts) AS score
             FROM documents_fts
             JOIN documents d ON d.id = documents_fts.rowid
             WHERE documents_fts MATCH ?2
             ORDER BY score
             LIMIT ?3",
        )?;

        let rows = stmt.query_map(params![token_window, fts_query, limit as i64], |row| {
            let source_kind = match row.get::<_, String>(2)?.as_str() {
                "filesystem" => SourceKind::Filesystem,
                "dpkg_status" => SourceKind::DpkgStatus,
                "man_directory" => SourceKind::ManDirectory,
                _ => SourceKind::Filesystem,
            };

            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                source_kind,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, f64>(7).unwrap_or(0.0),
            ))
        })?;

        let mut results = Vec::new();
        for (index, row) in rows.enumerate() {
            let (
                path,
                source_name,
                source_kind,
                trust_label,
                package_name,
                package_version,
                snippet,
                score,
            ) = row?;

            results.push(QueryResult {
                rank: index + 1,
                score,
                bounded_content: bound_content(&snippet, request.max_content_chars),
                citation: Citation {
                    source_name,
                    source_kind,
                    path,
                    package_name,
                    package_version,
                    trust_label,
                },
            });
        }

        Ok(QueryResponse {
            protocol_id: COMMON_PROTOCOL_ID,
            router_contract_id: ROUTER_CONTEXT_ID,
            authority_boundary: AUTHORITY_BOUNDARY,
            query: request.query,
            fts_query,
            result_count: results.len(),
            results,
        })
    }

    pub fn query_router_context(&self, request: QueryRequest) -> Result<RouterContextResponse> {
        let response = self.query(request)?;
        let documents = response
            .results
            .into_iter()
            .map(|result| RouterContextDocument {
                bounded_content: result.bounded_content,
                citation: result.citation,
            })
            .collect::<Vec<_>>();

        Ok(RouterContextResponse {
            protocol_id: COMMON_PROTOCOL_ID,
            router_contract_id: ROUTER_CONTEXT_ID,
            authority_boundary: AUTHORITY_BOUNDARY,
            documents,
        })
    }
}

fn verify_read_only_permissions(index_path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(index_path).map_err(|source| RetrievalError::Io {
        path: index_path.to_path_buf(),
        source,
    })?;

    if metadata.file_type().is_symlink() {
        return Err(RetrievalError::InsecureIndexPermissions(
            "index path cannot be a symlink".into(),
        ));
    }

    let mode = metadata.mode() & 0o777;
    if mode & 0o022 != 0 {
        return Err(RetrievalError::InsecureIndexPermissions(format!(
            "index permissions must not be group/other writable (found {:o})",
            mode
        )));
    }

    Ok(())
}

fn open_index_ro(index_path: &Path) -> Result<Connection> {
    Connection::open_with_flags(
        index_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| map_sqlite_error(index_path, error))
}

fn quick_check(conn: &Connection, index_path: &Path) -> Result<()> {
    let result: String = conn
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(|error| map_sqlite_error(index_path, error))?;

    if result != "ok" {
        return Err(RetrievalError::CorruptDatabase {
            path: index_path.to_path_buf(),
            details: result,
        });
    }

    Ok(())
}

fn snippet_tokens_for_chars(max_chars: usize) -> usize {
    let chars = max_chars.max(64).min(2_000);
    (chars / 8).max(12)
}

fn to_safe_fts_query(input: &str) -> Result<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in input.chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens.retain(|token| {
        !token.is_empty()
            && !matches!(token.as_str(), "and" | "or" | "not" | "near")
            && !token.chars().all(|ch| ch.is_ascii_digit())
    });

    if tokens.is_empty() {
        return Err(RetrievalError::EmptyQuery);
    }

    let mut safe = String::new();
    for (index, token) in tokens.iter().enumerate() {
        if index > 0 {
            safe.push_str(" AND ");
        }
        safe.push('"');
        safe.push_str(&token.replace('"', "\"\""));
        safe.push('"');
    }

    Ok(safe)
}

fn bound_content(input: &str, max_chars: usize) -> String {
    let max_chars = max_chars.max(64).min(2_000);
    if input.chars().count() <= max_chars {
        return input.trim().to_string();
    }

    let mut out = String::new();
    for (i, ch) in input.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }

    out.trim().to_string()
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
    use super::to_safe_fts_query;

    #[test]
    fn tokenizes_unsafe_fts_input() {
        let query = to_safe_fts_query("sentia OR \"router\" NEAR(test,2)").unwrap();
        assert_eq!(query, "\"sentia\" AND \"router\" AND \"test\"");
    }
}
