// 네이티브·CLI가 공유하는 엄격한 문서 명령과 읽기 전용 접점을 제공한다.
use super::{err, KnowledgeService, Result};
use serde::Deserialize;
use serde_json::{json, Value};
#[derive(Deserialize)]
#[serde(
    tag = "command",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Command {
    GetReportProfile {
        project_id: String,
    },
    SaveReportProfile {
        profile: Value,
        expected_revision: i64,
    },
    CreateReportRequest {
        request: Value,
    },
    GetReportContext {
        request_id: String,
    },
    ReadReportSource {
        request_id: String,
        index: usize,
        lease_token: String,
    },
    SubmitReportResult {
        request_id: String,
        lease_token: String,
        draft: Value,
        proofs: Value,
    },
    GetReportDraft {
        request_id: String,
    },
    ValidateReportDraft {
        request_id: String,
        body: String,
    },
    PreviewReportSave {
        request_id: String,
        destination: std::path::PathBuf,
    },
    SaveReportDraft {
        request_id: String,
        body: String,
        destination: std::path::PathBuf,
        expected_sha256: Option<String>,
        overwrite: bool,
    },
    GetDocumentConfig {},
    SaveDocumentConfig {
        config: Value,
        expected_revision: u64,
    },
    UpsertDocumentSource {
        source: Value,
        expected_revision: u64,
    },
    ArchiveDocumentSource {
        source_id: String,
        expected_revision: u64,
    },
    GetDocumentSourceStates {},
    ListUnlinkedDocuments {
        #[serde(default)]
        page: u32,
    },
    SetDocumentProjectLink {
        project_id: String,
        document_id: String,
        linked: bool,
        expected_project_revision: i64,
    },
    ScanDocumentSources {
        #[serde(default)]
        source_ids: Vec<String>,
        #[serde(default)]
        force: bool,
    },
    ListProjectDocuments {
        project_id: String,
        #[serde(default)]
        page: u32,
    },
    ReadDocumentVersion {
        version_id: String,
        #[serde(default = "one")]
        start: usize,
        #[serde(default = "two_hundred")]
        limit: usize,
    },
    CreateAnalysisJob {
        request: Value,
    },
    GetAnalysisJob {
        job_id: String,
    },
    ListAnalysisJobs {
        project_id: String,
    },
    ClaimAnalysisJob {
        job_id: String,
        owner: String,
    },
    HeartbeatAnalysisJob {
        job_id: String,
        lease_token: String,
    },
    SubmitAnalysisResult {
        job_id: String,
        lease_token: String,
        result: Value,
    },
    ParseAnalysisJob {
        job_id: String,
    },
    GetImportPreview {
        job_id: String,
    },
    ReviewImportCandidate {
        job_id: String,
        candidate_id: String,
        review_state: String,
        edited: Option<Value>,
        reason: String,
        expected_project_revision: i64,
    },
    ApplyImport {
        selection: Value,
    },
    UndoImport {
        batch_id: String,
        expected_project_revision: i64,
    },
    ListImportBatches {
        project_id: String,
    },
    GetProjectProgress {
        project_id: String,
        basis_id: Option<String>,
    },
    SaveProgressBasis {
        basis: Value,
        expected_project_revision: i64,
    },
    UpdateProgressTask {
        project_id: String,
        basis_id: String,
        version: i64,
        task_id: String,
        change: Value,
        expected_project_revision: i64,
    },
    ActivateManagedProgress {
        project_id: String,
        basis_id: String,
        expected_project_revision: i64,
    },
    GetProjectHistory {
        project_id: String,
        basis_id: String,
        from: String,
        through: String,
    },
    PreviewLegacyArchive {
        project_id: String,
        from: String,
        through: String,
    },
    ArchiveLegacyHistory {
        project_id: String,
        from: String,
        through: String,
        expected_project_revision: i64,
        preview_hash: String,
    },
}
fn one() -> usize {
    1
}
fn two_hundred() -> usize {
    200
}
pub fn read_only(input: &Value) -> bool {
    matches!(
        input["command"].as_str(),
        Some(
            "get_document_config"
                | "get_report_profile"
                | "get_document_source_states"
                | "list_project_documents"
                | "read_document_version"
                | "get_analysis_job"
                | "list_analysis_jobs"
                | "get_import_preview"
                | "list_import_batches"
                | "get_project_progress"
                | "get_project_history"
                | "preview_legacy_archive"
                | "list_unlinked_documents"
        )
    )
}
impl KnowledgeService {
    pub fn execute_command(&self, input: Value, preview: bool, _safe: bool) -> Result<Value> {
        if preview && !read_only(&input) {
            return Err("읽기 전용 미리보기에서는 문서와 진행 현황을 변경할 수 없습니다.".into());
        }
        match serde_json::from_value::<Command>(input).map_err(err)? {
            Command::GetReportProfile { project_id } => self.get_report_profile(&project_id),
            Command::SaveReportProfile {
                profile,
                expected_revision,
            } => self.save_report_profile(profile, expected_revision),
            Command::CreateReportRequest { request } => self.create_report_request(request),
            Command::GetReportContext { request_id } => self.get_report_context(&request_id),
            Command::ReadReportSource {
                request_id,
                index,
                lease_token,
            } => self.read_report_source(&request_id, index, &lease_token),
            Command::SubmitReportResult {
                request_id,
                lease_token,
                draft,
                proofs,
            } => self.submit_report_result(&request_id, &lease_token, draft, proofs),
            Command::GetReportDraft { request_id } => self.get_report_draft(&request_id),
            Command::ValidateReportDraft { request_id, body } => {
                self.validate_report_draft(&request_id, &body)
            }
            Command::PreviewReportSave {
                request_id,
                destination,
            } => self.preview_report_save(&request_id, &destination),
            Command::SaveReportDraft {
                request_id,
                body,
                destination,
                expected_sha256,
                overwrite,
            } => self.save_report_draft(
                &request_id,
                &body,
                &destination,
                expected_sha256.as_deref(),
                overwrite,
            ),
            Command::GetDocumentConfig {} => self.get_document_config(),
            Command::SaveDocumentConfig {
                config,
                expected_revision,
            } => self.save_document_config(config, expected_revision),
            Command::UpsertDocumentSource {
                source,
                expected_revision,
            } => self.upsert_document_source(source, expected_revision),
            Command::ArchiveDocumentSource {
                source_id,
                expected_revision,
            } => self.archive_document_source(&source_id, expected_revision),
            Command::GetDocumentSourceStates {} => self.source_states(),
            Command::ListUnlinkedDocuments { page } => self.list_unlinked_documents(page),
            Command::SetDocumentProjectLink {
                project_id,
                document_id,
                linked,
                expected_project_revision,
            } => self.set_document_project_link(
                &project_id,
                &document_id,
                linked,
                expected_project_revision,
            ),
            Command::ScanDocumentSources { source_ids, force } => {
                self.scan_document_sources(&source_ids, force)
            }
            Command::ListProjectDocuments { project_id, page } => {
                self.list_project_documents(&project_id, page)
            }
            Command::ReadDocumentVersion {
                version_id,
                start,
                limit,
            } => self.read_document_version(&version_id, start, limit),
            Command::CreateAnalysisJob { request } => self.create_analysis_job(request),
            Command::GetAnalysisJob { job_id } => self.get_analysis_job(&job_id),
            Command::ListAnalysisJobs { project_id } => self.list_analysis_jobs(&project_id),
            Command::ClaimAnalysisJob { job_id, owner } => self.claim_analysis_job(&job_id, &owner),
            Command::HeartbeatAnalysisJob {
                job_id,
                lease_token,
            } => self.heartbeat_analysis_job(&job_id, &lease_token),
            Command::SubmitAnalysisResult {
                job_id,
                lease_token,
                result,
            } => self.submit_analysis_result(&job_id, &lease_token, result),
            Command::ParseAnalysisJob { job_id } => self.parse_analysis_job(&job_id),
            Command::GetImportPreview { job_id } => self.get_import_preview(&job_id),
            Command::ReviewImportCandidate {
                job_id,
                candidate_id,
                review_state,
                edited,
                reason,
                expected_project_revision,
            } => self.review_import_candidate(
                &job_id,
                &candidate_id,
                &review_state,
                edited,
                &reason,
                expected_project_revision,
            ),
            Command::ApplyImport { selection } => self.apply_import(selection),
            Command::UndoImport {
                batch_id,
                expected_project_revision,
            } => self.undo_import(&batch_id, expected_project_revision),
            Command::ListImportBatches { project_id } => self.list_import_batches(&project_id),
            Command::GetProjectProgress {
                project_id,
                basis_id,
            } => self.get_project_progress(&project_id, basis_id.as_deref()),
            Command::SaveProgressBasis {
                basis,
                expected_project_revision,
            } => self.save_progress_basis(basis, expected_project_revision),
            Command::UpdateProgressTask {
                project_id,
                basis_id,
                version,
                task_id,
                change,
                expected_project_revision,
            } => self.update_progress_task(
                &project_id,
                &basis_id,
                version,
                &task_id,
                change,
                expected_project_revision,
            ),
            Command::ActivateManagedProgress {
                project_id,
                basis_id,
                expected_project_revision,
            } => self.activate_managed_progress(&project_id, &basis_id, expected_project_revision),
            Command::GetProjectHistory {
                project_id,
                basis_id,
                from,
                through,
            } => self.get_project_history(&project_id, &basis_id, &from, &through),
            Command::PreviewLegacyArchive {
                project_id,
                from,
                through,
            } => self.preview_legacy_archive(&project_id, &from, &through),
            Command::ArchiveLegacyHistory {
                project_id,
                from,
                through,
                expected_project_revision,
                preview_hash,
            } => self.archive_legacy_history(
                &project_id,
                &from,
                &through,
                expected_project_revision,
                &preview_hash,
            ),
        }
    }
}
pub fn cli_request(args: &[String]) -> Result<Option<(Value, bool)>> {
    let Some(group) = args.first().map(String::as_str) else {
        return Ok(None);
    };
    if matches!(group, "knowledge" | "knowledge-readonly") {
        if args.len() != 2 {
            return Err("usage: knowledge[-readonly] JSON".into());
        }
        return Ok(Some((
            serde_json::from_str(&args[1]).map_err(err)?,
            group == "knowledge-readonly",
        )));
    }
    if !matches!(group, "documents" | "analysis" | "progress" | "reports") {
        return Ok(None);
    }
    let command = match (group, args.get(1).map(String::as_str).unwrap_or("")) {
        ("reports", "profile") => "get_report_profile",
        ("reports", "context") => "get_report_context",
        ("reports", "claim") => "claim_analysis_job",
        ("reports", "heartbeat") => "heartbeat_analysis_job",
        ("reports", "read-source") => "read_report_source",
        ("reports", "submit") => "submit_report_result",
        ("documents", "config") => "get_document_config",
        ("documents", "save-config") => "save_document_config",
        ("documents", "source-upsert") => "upsert_document_source",
        ("documents", "source-archive") => "archive_document_source",
        ("documents", "states") => "get_document_source_states",
        ("documents", "scan") => "scan_document_sources",
        ("documents", "list") => "list_project_documents",
        ("documents", "unlinked") => "list_unlinked_documents",
        ("documents", "link") => "set_document_project_link",
        ("documents", "read") => "read_document_version",
        ("analysis", "create") => "create_analysis_job",
        ("analysis", "get") => "get_analysis_job",
        ("analysis", "list") => "list_analysis_jobs",
        ("analysis", "claim") => "claim_analysis_job",
        ("analysis", "heartbeat") => "heartbeat_analysis_job",
        ("analysis", "submit") => "submit_analysis_result",
        ("analysis", "parse") => "parse_analysis_job",
        ("analysis", "preview") => "get_import_preview",
        ("analysis", "review") => "review_import_candidate",
        ("progress", "get") => "get_project_progress",
        ("progress", "save-basis") => "save_progress_basis",
        ("progress", "task") => "update_progress_task",
        ("progress", "activate") => "activate_managed_progress",
        ("progress", "history") => "get_project_history",
        ("progress", "apply") => "apply_import",
        ("progress", "undo") => "undo_import",
        ("progress", "batches") => "list_import_batches",
        ("progress", "legacy-preview") => "preview_legacy_archive",
        ("progress", "legacy-archive") => "archive_legacy_history",
        _ => return Err("unknown knowledge command".into()),
    };
    let mut input = match args.get(2).map(String::as_str) {
        None => json!({}),
        Some("--stdin") if args.len() == 3 => {
            use std::io::Read;
            const LIMIT: u64 = 8 * 1024 * 1024;
            let mut bytes = Vec::new();
            std::io::stdin()
                .take(LIMIT + 1)
                .read_to_end(&mut bytes)
                .map_err(err)?;
            if bytes.len() as u64 > LIMIT {
                return Err("input too large".into());
            }
            serde_json::from_slice(&bytes).map_err(err)?
        }
        Some("--input") if args.len() == 4 => {
            let path = &args[3];
            let meta = std::fs::metadata(path).map_err(err)?;
            if meta.len() > 8 * 1024 * 1024 {
                return Err("input file too large".into());
            }
            let text = std::fs::read_to_string(path).map_err(err)?;
            serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(err)?
        }
        Some(text) if args.len() == 3 => serde_json::from_str(text).map_err(err)?,
        _ => return Err(
            "usage: documents|analysis|progress|reports COMMAND [JSON | --input FILE | --stdin]"
                .into(),
        ),
    };
    if !input.is_object() || input.get("command").is_some() {
        return Err("invalid command input".into());
    }
    input["command"] = json!(command);
    Ok(Some((input, false)))
}
