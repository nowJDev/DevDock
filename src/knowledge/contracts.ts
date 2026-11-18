// 프로젝트 문서·진행 기준·근거와 반영 이력의 화면 계약을 정의한다.
export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
export interface DocumentSource {
  sourceId: string; scope: 'global' | 'project'; projectId?: string | null;
  location: 'local' | 'nas'; root: string; roles: string[]; enabled: boolean;
  include?: string[]; exclude?: string[]; archived?: boolean;
}
export interface DocumentConfigView { revision: number; nasIntervalSec: number; localIntervalSec: 5; sources: DocumentSource[] }
export interface SourceSpan { versionId: string; sha256: string; locator: { lines: { start: number; end: number } } | { jsonPointer: string } }
export type TaskState = 'pending' | 'inProgress' | 'blocked' | 'reportedComplete' | 'cancelled';
export interface BasisTask { taskId: string; title: string; revision: number; state: TaskState; parentId?: string | null; evidence?: SourceSpan[]; verified?: boolean }
export interface ProgressBasis { basisId: string; version: number; projectId: string; kind: 'summary' | 'development' | 'feature' | 'acceptance' | 'legacy'; name: string; tasks: BasisTask[]; authoritativeSourceIds: string[] }
export interface Counts { total: number; verified: number; reportedComplete: number; inProgress: number; blocked: number }
export interface ProjectProgressView { projectId: string; revision: number; mode: 'legacy' | 'managed'; basis: ProgressBasis | null; bases: ProgressBasis[]; counts: Counts; reportRatio: number | null; verifiedRatio: number | null; sourceFreshness: string }
export interface DocumentVersion { versionId: string; documentId: string; sourceId: string; path: string; sha256: string; bytes: number; capturedAt: string; authoredDate?: string | null; kind: string }
export type CandidateKind = 'workEntry' | 'historicalSnapshot' | 'currentTaskUpdate' | 'basisDefinition' | 'projectLink';
export interface ImportCandidate { candidateId: string; fingerprint: string; kind: CandidateKind; workDate?: string; authoredDate?: string; effectiveDate?: string; sourceSpans: SourceSpan[]; proposedChanges: Json; reviewState: 'pending' | 'accepted' | 'rejected' | 'conflict' }
export interface ImportPreview { jobId: string; projectId: string; basisId?: string; expectedProjectRevision: number; inputVersionIds: string[]; candidates: ImportCandidate[]; conflicts: string[] }
export interface AnalysisJob { jobId: string; projectId: string; kind: 'progressAnalysis' | 'dailyReport' | 'documentPublishDraft'; state: string; inputVersionIds: string[]; expectedProjectRevision: number; basisId?: string; createdAt: string }
export interface HistoryPoint { date: string; basisVersion: number; counts: Counts | null; origin: 'observed' | 'reconstructed' | 'correction'; coverage: string; sourceSpans: SourceSpan[]; capturedAt: string }
export interface ProjectHistory { projectId: string; basisId: string; from: string; through: string; points: HistoryPoint[]; workEntries: Json[] }
