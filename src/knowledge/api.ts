// 문서 관리 명령을 공유 코어에 전달하며 브라우저 미리보기의 쓰기를 제한한다.
import {call} from '../api';
import {knowledgeReadCommands} from './contracts';
import type {AnalysisJob, DocumentConfigView, DocumentSource, DocumentVersion, ImportPreview, Json, ProjectHistory, ProjectProgressView} from './contracts';
export interface SourceState { sourceId: string; state: string; checkedAt?: string; lastSuccessAt?: string; nextRetryAt?: string; errors?: string[] }
export interface KnowledgeApi {
  execute<T = Json>(command: string, args?: Record<string, unknown>): Promise<T>;
  config(): Promise<DocumentConfigView>;
  saveConfig(config: DocumentConfigView): Promise<DocumentConfigView>;
  upsertSource(source: DocumentSource, expectedRevision: number): Promise<DocumentConfigView>;
  archiveSource(sourceId: string, expectedRevision: number): Promise<DocumentConfigView>;
  sourceStates(): Promise<{sources: SourceState[]}>;
  scan(sourceIds?: string[], force?: boolean): Promise<Json>;
  documents(projectId: string, page?: number): Promise<{documents: DocumentVersion[]; page: number; pageSize: number}>;
  read(versionId: string, start?: number, limit?: number): Promise<{versionId: string; sha256: string; start: number; totalLines: number; text: string}>;
  progress(projectId: string, basisId?: string): Promise<ProjectProgressView>;
  history(projectId: string, basisId: string, from: string, through: string): Promise<ProjectHistory>;
  jobs(projectId: string): Promise<{jobs: AnalysisJob[]}>;
  createAnalysis(projectId: string, inputVersionIds: string[], basisId?: string): Promise<AnalysisJob>;
  parse(jobId: string): Promise<ImportPreview>;
  preview(jobId: string): Promise<ImportPreview>;
}
export const knowledgeApi: KnowledgeApi = {
  execute: async <T,>(command: string, args: Record<string, unknown> = {}): Promise<T> => {
    const native = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
    if (!native && !knowledgeReadCommands.has(command)) throw new Error('읽기 전용 미리보기입니다. 변경은 DevDock 앱에서 진행해 주세요.');
    return call<T>('knowledge_command', {operation: {...args, command}});
  },
  config: () => knowledgeApi.execute('get_document_config'),
  saveConfig: config => knowledgeApi.execute('save_document_config', {config, expectedRevision: config.revision}),
  upsertSource: (source, expectedRevision) => knowledgeApi.execute('upsert_document_source', {source, expectedRevision}),
  archiveSource: (sourceId, expectedRevision) => knowledgeApi.execute('archive_document_source', {sourceId, expectedRevision}),
  sourceStates: () => knowledgeApi.execute('get_document_source_states'),
  scan: (sourceIds = [], force = true) => knowledgeApi.execute('scan_document_sources', {sourceIds, force}),
  documents: (projectId, page = 0) => knowledgeApi.execute('list_project_documents', {projectId, page}),
  read: (versionId, start = 1, limit = 200) => knowledgeApi.execute('read_document_version', {versionId, start, limit}),
  progress: (projectId, basisId) => knowledgeApi.execute('get_project_progress', {projectId, basisId: basisId ?? null}),
  history: (projectId, basisId, from, through) => knowledgeApi.execute('get_project_history', {projectId, basisId, from, through}),
  jobs: projectId => knowledgeApi.execute('list_analysis_jobs', {projectId}),
  createAnalysis: (projectId, inputVersionIds, basisId) => knowledgeApi.execute('create_analysis_job', {request: {projectId, inputVersionIds, basisId: basisId ?? null}}),
  parse: jobId => knowledgeApi.execute('parse_analysis_job', {jobId}),
  preview: jobId => knowledgeApi.execute('get_import_preview', {jobId}),
};
