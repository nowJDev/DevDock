// 프로젝트 문서·진행 기준·근거와 반영 이력의 화면 계약을 정의한다.
export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
export const knowledgeReadCommands = new Set(['get_document_config','get_report_profile','list_report_handover_files','get_document_source_states','list_unlinked_documents','list_project_documents','read_document_version','get_analysis_job','list_analysis_jobs','get_import_preview','list_import_batches','get_project_progress','get_project_history','preview_legacy_archive']);
export interface ReportProfile {
  projectId: string; revision: number; workHeading: string; templateMode: 'default' | 'custom';
  customTemplate: string | null; skillPath: string; localSaveDirectory: string | null; defaultNasFolderSourceId: string | null;
}
export interface PublishTarget {role:'dailyLog'|'checklist'|'status'|'index';path:string}
export interface DocumentSource {
  sourceId: string; scope: 'global' | 'project'; projectId?: string | null;
  location: 'local' | 'nas'; root: string; roles: string[]; enabled: boolean;
  include?: string[]; exclude?: string[]; archived?: boolean; publishTargets?: PublishTarget[];
}
export interface DocumentConfigView { revision: number; nasIntervalSec: number; localIntervalSec: 5; sources: DocumentSource[] }
export interface SourceSpan { versionId: string; sha256: string; locator: { lines: { start: number; end: number } } | { jsonPointer: string } }
export type TaskState = 'pending' | 'inProgress' | 'blocked' | 'reportedComplete' | 'cancelled';
export interface VerificationEvidence { subjectRevision: number; success: boolean; kind: 'automated' | 'manual' | 'review'; sourceSpan: SourceSpan; fixture: boolean }
export interface BasisTask { taskId: string; title: string; revision: number; state: TaskState; parentId?: string | null; sourceSpan?: SourceSpan | null; verificationEvidence?: VerificationEvidence[]; verified?: boolean }
export interface ProgressBasis { basisId: string; version: number; versions?:number[]; projectId: string; kind: 'summary' | 'development' | 'feature' | 'acceptance' | 'legacy'; name: string; tasks: BasisTask[]; authoritativeSourceIds: string[]; sourceVersionIds: string[] }
export interface Counts { total: number; verified: number; reportedComplete: number; inProgress: number; blocked: number }
export interface ProjectProgressView { projectId: string; revision: number; mode: 'legacy' | 'managed'; basis: ProgressBasis | null; bases: ProgressBasis[]; counts: Counts; reportRatio: number | null; verifiedRatio: number | null; sourceFreshness: string }
export interface DocumentVersion { versionId: string; documentId: string; sourceId: string; path: string; sha256: string; bytes: number; capturedAt: string; authoredDate?: string | null; kind: string }
export type CandidateKind = 'workEntry' | 'historicalSnapshot' | 'currentTaskUpdate' | 'basisDefinition' | 'projectLink';
export interface ImportCandidate { candidateId: string; fingerprint: string; kind: CandidateKind; workDate?: string; authoredDate?: string; effectiveDate?: string; sourceSpans: SourceSpan[]; proposedChanges: Json; applied?:boolean; reviewState: 'pending' | 'accepted' | 'rejected' | 'conflict' }
export interface ImportPreview { jobId: string; projectId: string; basisId?: string; expectedProjectRevision: number; inputVersionIds: string[]; candidates: ImportCandidate[]; conflicts: string[] }
export interface AnalysisJob { jobId: string; projectId: string; kind: 'progressAnalysis' | 'dailyReport' | 'documentPublishDraft'; requestId?:string; state: string; inputVersionIds: string[]; expectedProjectRevision: number; basisId?: string; createdAt: string }
export interface HistoryPoint { date: string; basisVersion: number; counts: Counts | null; origin: 'observed' | 'reconstructed' | 'correction'; coverage: string; sourceSpans: SourceSpan[]; capturedAt: string }
export interface ProjectHistory { projectId: string; basisId: string; from: string; through: string; points: HistoryPoint[]; workEntries: Json[]; comparisons?: HistoryPoint[] }
