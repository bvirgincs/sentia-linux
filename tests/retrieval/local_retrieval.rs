use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sentia_retrieval::{
    Indexer, QueryEngine, QueryRequest, RetrievalConfig, RetrievalError, SourceConfig, SourceKindConfig,
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(path: &str) -> PathBuf {
    repo_root().join("tests/retrieval/fixtures").join(path)
}

struct Sandbox {
    path: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = repo_root()
            .join("target/retrieval-tests")
            .join(format!("{}-{}", name, nonce));
        if path.exists() {
            fs::remove_dir_all(&path).expect("cleanup stale sandbox");
        }
        fs::create_dir_all(&path).expect("create sandbox");
        Self { path }
    }

    fn join(&self, relative: &str) -> PathBuf {
        self.path.join(relative)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        if self.path.exists() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn write_text(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, body).expect("write file");
}

fn copy_fixture(src_relative: &str, dst: &Path) {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::copy(fixture(src_relative), dst).expect("copy fixture");
}

fn config_for(source: Vec<SourceConfig>, max_file_bytes: usize, max_total_bytes: usize) -> RetrievalConfig {
    RetrievalConfig {
        schema_version: 1,
        max_file_count: 5_000,
        max_file_bytes,
        max_total_bytes,
        max_query_results: 8,
        source,
    }
}

fn filesystem_source(name: &str, root: &Path, trust: &str) -> SourceConfig {
    SourceConfig {
        name: name.to_string(),
        kind: SourceKindConfig::Filesystem,
        trust_label: Some(trust.to_string()),
        roots: vec![root.to_path_buf()],
        path: None,
        allow_pages: vec![],
        include_extensions: vec!["md".to_string(), "txt".to_string()],
        max_depth: 8,
        allow_private_roots: true,
        package_name: None,
        package_version: None,
    }
}

#[test]
fn indexes_fixture_docs_and_returns_citations_with_trust_labels() {
    let sandbox = Sandbox::new("citations");
    let docs_root = sandbox.join("docs");
    copy_fixture("docs/sentia-overview.md", &docs_root.join("sentia-overview.md"));
    copy_fixture("docs/router-boundary.md", &docs_root.join("router-boundary.md"));

    let index_path = sandbox.join("index.sqlite");
    let config = config_for(
        vec![filesystem_source("sentia-docs", &docs_root, "SENTIA_PACKAGED_DOC")],
        262_144,
        10 * 1024 * 1024,
    );

    let report = Indexer::new(&index_path).rebuild(&config).expect("build index");
    assert!(report.inserted_total >= 2);

    let response = QueryEngine::new(&index_path)
        .query(QueryRequest {
            query: "offline AI assistance".into(),
            limit: 4,
            max_content_chars: 240,
        })
        .expect("query index");

    assert!(response
        .results
        .iter()
        .any(|result| result.citation.path.ends_with("sentia-overview.md")));
    assert!(response
        .results
        .iter()
        .all(|result| result.citation.trust_label == "SENTIA_PACKAGED_DOC"));
    assert!(response
        .authority_boundary
        .contains("cannot grant tool authority"));
}

#[test]
fn indexes_real_repo_readme_when_explicitly_allowlisted() {
    let sandbox = Sandbox::new("repo-readme");
    let index_path = sandbox.join("index.sqlite");

    let config = config_for(
        vec![SourceConfig {
            name: "repo-doc".into(),
            kind: SourceKindConfig::Filesystem,
            trust_label: Some("SENTIA_REPO_DOC".into()),
            roots: vec![repo_root().join("README.md")],
            path: None,
            allow_pages: vec![],
            include_extensions: vec!["md".into()],
            max_depth: 2,
            allow_private_roots: true,
            package_name: Some("sentia-repo".into()),
            package_version: Some("dev".into()),
        }],
        262_144,
        2 * 1024 * 1024,
    );

    Indexer::new(&index_path)
        .rebuild(&config)
        .expect("index repo readme");

    let response = QueryEngine::new(&index_path)
        .query(QueryRequest {
            query: "offline AI assistance".into(),
            limit: 3,
            max_content_chars: 180,
        })
        .expect("query repo readme");

    assert!(response
        .results
        .iter()
        .any(|result| result.citation.path.ends_with("README.md")));
}

#[test]
fn prevents_symlink_escape_from_allowlisted_root() {
    let sandbox = Sandbox::new("symlink");
    let allowed_root = sandbox.join("allowed");
    let private_root = sandbox.join("private");
    fs::create_dir_all(&allowed_root).expect("allowed root");
    fs::create_dir_all(&private_root).expect("private root");

    write_text(
        &allowed_root.join("normal.md"),
        "normal retrieval content with safe words",
    );
    write_text(
        &private_root.join("credentials.txt"),
        "DO_NOT_INDEX_SECRET material",
    );

    symlink(
        private_root.join("credentials.txt"),
        allowed_root.join("secret-link.txt"),
    )
    .expect("create symlink");

    let index_path = sandbox.join("index.sqlite");
    let config = config_for(
        vec![filesystem_source("allowlisted-docs", &allowed_root, "SENTIA_PACKAGED_DOC")],
        262_144,
        2 * 1024 * 1024,
    );

    let report = Indexer::new(&index_path).rebuild(&config).expect("index with symlink");
    assert!(report.sources[0].scanned_files >= 2);

    let response = QueryEngine::new(&index_path)
        .query(QueryRequest {
            query: "DO_NOT_INDEX_SECRET".into(),
            limit: 2,
            max_content_chars: 120,
        })
        .expect("query index");

    assert_eq!(response.result_count, 0);
}

#[test]
fn enforces_file_size_limit_and_index_budget() {
    let sandbox = Sandbox::new("size");
    let docs_root = sandbox.join("docs");
    fs::create_dir_all(&docs_root).expect("docs root");

    write_text(&docs_root.join("small.md"), "small content");
    write_text(
        &docs_root.join("large.md"),
        &"LARGE_TOKEN ".repeat(256),
    );

    let index_path = sandbox.join("index.sqlite");
    let config = config_for(
        vec![filesystem_source("bounded-docs", &docs_root, "SENTIA_PACKAGED_DOC")],
        128,
        1024,
    );

    let report = Indexer::new(&index_path).rebuild(&config).expect("index bounded");
    assert!(report.sources[0].skipped_oversize >= 1);

    let response = QueryEngine::new(&index_path)
        .query(QueryRequest {
            query: "LARGE_TOKEN".into(),
            limit: 5,
            max_content_chars: 160,
        })
        .expect("query bounded");
    assert_eq!(response.result_count, 0);
}

#[test]
fn tokenizes_user_fts_syntax_safely() {
    let sandbox = Sandbox::new("fts-syntax");
    let docs_root = sandbox.join("docs");
    fs::create_dir_all(&docs_root).expect("docs root");
    write_text(
        &docs_root.join("syntax.md"),
        "sentia router context and citations for local retrieval",
    );

    let index_path = sandbox.join("index.sqlite");
    let config = config_for(
        vec![filesystem_source("syntax-docs", &docs_root, "SENTIA_PACKAGED_DOC")],
        262_144,
        2 * 1024 * 1024,
    );

    Indexer::new(&index_path)
        .rebuild(&config)
        .expect("index syntax fixture");

    let response = QueryEngine::new(&index_path)
        .query(QueryRequest {
            query: "sentia OR \"router\" NEAR(context,3)".into(),
            limit: 4,
            max_content_chars: 120,
        })
        .expect("query syntax");

    assert!(response.result_count >= 1);
    assert!(response.fts_query.contains("\"sentia\""));
    assert!(response.fts_query.contains("\"router\""));
    assert!(!response.fts_query.contains("\"near\""));
    assert!(response.fts_query.contains("AND"));
}

#[test]
fn supports_incremental_refresh_and_delete() {
    let sandbox = Sandbox::new("refresh");
    let docs_root = sandbox.join("docs");
    fs::create_dir_all(&docs_root).expect("docs root");
    let doc_path = docs_root.join("dynamic.md");
    write_text(&doc_path, "alpha-keyword appears here");

    let index_path = sandbox.join("index.sqlite");
    let config = config_for(
        vec![filesystem_source("refresh-docs", &docs_root, "SENTIA_PACKAGED_DOC")],
        262_144,
        2 * 1024 * 1024,
    );

    let indexer = Indexer::new(&index_path);
    indexer.rebuild(&config).expect("first index");

    let alpha = QueryEngine::new(&index_path)
        .query(QueryRequest {
            query: "alpha-keyword".into(),
            limit: 3,
            max_content_chars: 160,
        })
        .expect("query alpha");
    assert!(alpha.result_count >= 1);

    write_text(&doc_path, "beta-keyword replaced alpha-keyword content");
    let second = indexer.rebuild(&config).expect("second index");
    assert!(second.updated_total >= 1);

    let beta = QueryEngine::new(&index_path)
        .query(QueryRequest {
            query: "beta-keyword".into(),
            limit: 3,
            max_content_chars: 160,
        })
        .expect("query beta");
    assert!(beta.result_count >= 1);

    fs::remove_file(&doc_path).expect("remove document");
    let third = indexer.rebuild(&config).expect("third index");
    assert!(third.removed_total >= 1);

    let after_delete = QueryEngine::new(&index_path)
        .query(QueryRequest {
            query: "beta-keyword".into(),
            limit: 3,
            max_content_chars: 160,
        })
        .expect("query after delete");
    assert_eq!(after_delete.result_count, 0);
}

#[test]
fn indexes_dpkg_metadata_with_package_versions() {
    let sandbox = Sandbox::new("dpkg");
    let status_path = sandbox.join("status");
    copy_fixture("dpkg/status.sample", &status_path);

    let config = config_for(
        vec![SourceConfig {
            name: "dpkg-installed".into(),
            kind: SourceKindConfig::DpkgStatus,
            trust_label: Some("LOCAL_DPKG_STATUS".into()),
            roots: vec![],
            path: Some(status_path.clone()),
            allow_pages: vec![],
            include_extensions: vec![],
            max_depth: 1,
            allow_private_roots: true,
            package_name: None,
            package_version: None,
        }],
        262_144,
        5 * 1024 * 1024,
    );

    let index_path = sandbox.join("index.sqlite");
    Indexer::new(&index_path)
        .rebuild(&config)
        .expect("index dpkg metadata");

    let response = QueryEngine::new(&index_path)
        .query(QueryRequest {
            query: "sentia-router".into(),
            limit: 5,
            max_content_chars: 200,
        })
        .expect("query dpkg metadata");

    assert_eq!(response.result_count, 1);
    let citation = &response.results[0].citation;
    assert_eq!(citation.package_name.as_deref(), Some("sentia-router"));
    assert_eq!(citation.package_version.as_deref(), Some("1.2.3-1"));
}

#[test]
fn ingests_manpages_without_executing_macros() {
    let sandbox = Sandbox::new("manpages");
    let man_root = sandbox.join("man1");
    fs::create_dir_all(&man_root).expect("man root");
    copy_fixture("man/sentiactl.1", &man_root.join("sentiactl.1"));

    let config = config_for(
        vec![SourceConfig {
            name: "selected-man".into(),
            kind: SourceKindConfig::ManDirectory,
            trust_label: Some("DEBIAN_MANPAGE".into()),
            roots: vec![man_root.clone()],
            path: None,
            allow_pages: vec!["sentiactl".into()],
            include_extensions: vec![],
            max_depth: 4,
            allow_private_roots: true,
            package_name: None,
            package_version: None,
        }],
        262_144,
        5 * 1024 * 1024,
    );

    let index_path = sandbox.join("index.sqlite");
    Indexer::new(&index_path)
        .rebuild(&config)
        .expect("index manpage");

    let expected = QueryEngine::new(&index_path)
        .query(QueryRequest {
            query: "diagnose".into(),
            limit: 4,
            max_content_chars: 180,
        })
        .expect("query manpage");
    assert!(expected.result_count >= 1);

    let forbidden = QueryEngine::new(&index_path)
        .query(QueryRequest {
            query: "passwd".into(),
            limit: 4,
            max_content_chars: 180,
        })
        .expect("query macro token");
    assert_eq!(forbidden.result_count, 0);
}

#[test]
fn rejects_insecure_or_corrupt_indexes_with_explicit_errors() {
    let sandbox = Sandbox::new("permissions");
    let index_path = sandbox.join("index.sqlite");

    write_text(&index_path, "not a sqlite database");
    fs::set_permissions(&index_path, fs::Permissions::from_mode(0o640)).expect("set perms");

    let err = QueryEngine::new(&index_path)
        .query(QueryRequest {
            query: "sentia".into(),
            limit: 3,
            max_content_chars: 100,
        })
        .expect_err("expected corrupt db failure");

    match err {
        RetrievalError::CorruptDatabase { .. } => {}
        other => panic!("expected corruption error, got {other:?}"),
    }

    let docs_root = sandbox.join("docs");
    fs::create_dir_all(&docs_root).expect("docs root");
    write_text(&docs_root.join("doc.md"), "sentia entry");
    let valid_index_path = sandbox.join("valid-index.sqlite");

    let config = config_for(
        vec![filesystem_source("docs", &docs_root, "SENTIA_PACKAGED_DOC")],
        262_144,
        5 * 1024 * 1024,
    );
    Indexer::new(&valid_index_path)
        .rebuild(&config)
        .expect("create valid index");

    fs::set_permissions(&valid_index_path, fs::Permissions::from_mode(0o666)).expect("loosen perms");

    let err = QueryEngine::new(&valid_index_path)
        .query(QueryRequest {
            query: "sentia".into(),
            limit: 2,
            max_content_chars: 120,
        })
        .expect_err("expected insecure permissions failure");

    match err {
        RetrievalError::InsecureIndexPermissions(message) => {
            assert!(message.contains("group/other writable"));
        }
        other => panic!("expected insecure permissions error, got {other:?}"),
    }
}
