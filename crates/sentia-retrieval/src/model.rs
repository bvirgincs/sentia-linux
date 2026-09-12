use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Filesystem,
    DpkgStatus,
    ManDirectory,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceReport {
    pub name: String,
    pub kind: SourceKind,
    pub trust_label: String,
    pub configured_roots: Vec<String>,
    pub scanned_files: usize,
    pub indexed_documents: usize,
    pub inserted: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub removed: usize,
    pub skipped_outside_allowlist: usize,
    pub skipped_oversize: usize,
    pub skipped_budget: usize,
    pub skipped_unreadable: usize,
}

impl SourceReport {
    pub fn new(name: String, kind: SourceKind, trust_label: String, configured_roots: Vec<String>) -> Self {
        Self {
            name,
            kind,
            trust_label,
            configured_roots,
            scanned_files: 0,
            indexed_documents: 0,
            inserted: 0,
            updated: 0,
            unchanged: 0,
            removed: 0,
            skipped_outside_allowlist: 0,
            skipped_oversize: 0,
            skipped_budget: 0,
            skipped_unreadable: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexReport {
    pub protocol_id: &'static str,
    pub router_contract_id: &'static str,
    pub index_path: String,
    pub authority_boundary: &'static str,
    pub max_file_count: usize,
    pub max_file_bytes: usize,
    pub max_total_bytes: usize,
    pub indexed_total: usize,
    pub inserted_total: usize,
    pub updated_total: usize,
    pub removed_total: usize,
    pub skipped_total: usize,
    pub sources: Vec<SourceReport>,
}

#[derive(Debug, Clone)]
pub struct QueryRequest {
    pub query: String,
    pub limit: usize,
    pub max_content_chars: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Citation {
    pub source_name: String,
    #[serde(rename = "source_kind")]
    pub source_kind: SourceKind,
    pub path: String,
    pub package_name: Option<String>,
    pub package_version: Option<String>,
    pub trust_label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryResult {
    pub rank: usize,
    pub score: f64,
    pub bounded_content: String,
    pub citation: Citation,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryResponse {
    pub protocol_id: &'static str,
    pub router_contract_id: &'static str,
    pub authority_boundary: &'static str,
    pub query: String,
    pub fts_query: String,
    pub result_count: usize,
    pub results: Vec<QueryResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouterContextDocument {
    pub bounded_content: String,
    pub citation: Citation,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouterContextResponse {
    pub protocol_id: &'static str,
    pub router_contract_id: &'static str,
    pub authority_boundary: &'static str,
    pub documents: Vec<RouterContextDocument>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorEnvelope {
    pub code: String,
    pub message: String,
}
