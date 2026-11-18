// 프로젝트마다 보고, NAS 탐색기와 문서 업데이트를 독립적으로 연다.
import {useEffect,useState} from 'react';
import {ClipboardList,FolderOpen,FilePenLine} from 'lucide-react';
import Dialog from '../components/Dialog';
import type {Project} from '../contracts';
import type {KnowledgeApi} from './api';
import type {DocumentSource,ReportProfile} from './contracts';
import DailyReport from './DailyReport';
import PublishReview from './PublishReview';

export type Action = 'report'|'open'|'publish';
export type ProjectAction = {action:Action;project:Pick<Project,'id'|'name'>;opener:HTMLElement};
const actions = [
  {action:'report',icon:ClipboardList,label:'일일보고'},
  {action:'open',icon:FolderOpen,label:'NAS 문서 열기'},
  {action:'publish',icon:FilePenLine,label:'문서 업데이트'},
] as const;
export function ProjectActions({project,onAction}:{
  project:Pick<Project,'id'|'name'>;onAction:(value:ProjectAction)=>void;
}){
  return <div className="project-actions" role="group" aria-label={project.name+' 문서 작업'}>
    {actions.map(({action,label,icon:Icon})=><button className="subtle-button" key={action} type="button"
      aria-label={project.name+' '+label} onClick={event=>onAction({action,project,opener:event.currentTarget})}>
      <Icon size={14} aria-hidden="true"/>{label}
    </button>)}
  </div>;
}
export function ProjectActionDialog({api,value,onClose,onSettings}:{
  api:KnowledgeApi;value:ProjectAction;onClose:()=>void;onSettings:()=>void;
}){
  const {action,project,opener}=value;
  const [sources,setSources]=useState<DocumentSource[]|null>(null);
  const [sourceId,setSourceId]=useState(''),[shortcut,setShortcut]=useState('root');
  const [busy,setBusy]=useState(false),[error,setError]=useState('');
  useEffect(()=>{
    if(action==='report')return;
    let active=true;
    void Promise.all([api.config(),api.execute<ReportProfile>('get_report_profile',{projectId:project.id})]).then(([config,profile])=>{
      if(!active)return;
      const options=config.sources.filter(s=>s.projectId===project.id&&s.scope==='project'&&s.location==='nas'&&s.enabled&&!s.archived);
      setSources(options);
      setSourceId(options.find(s=>s.sourceId===profile.defaultNasFolderSourceId)?.sourceId??options[0]?.sourceId??'');
    },e=>{if(active)setError(String(e).replace(/^Error: /,''));});
    return()=>{active=false;};
  },[api,action,project.id]);
  if(action==='report')return <DailyReport api={api} project={project} returnFocus={opener} onClose={onClose}/>;
  if(action==='publish'&&sources?.length)return <PublishReview api={api} project={project} sources={sources}
    opener={opener} defaultSourceId={sourceId} onClose={onClose}/>;
  return <Dialog title={(action==='open'?'NAS 문서 열기':'문서 업데이트')+' · '+project.name} returnFocus={opener} onClose={onClose}>
    {error&&<p className="alert error" role="alert">{error}</p>}
    {!sources&&!error&&<p role="status">NAS 문서 연결을 불러오는 중입니다.</p>}
    {sources?.length===0&&<>
      <p>이 프로젝트에 연결한 NAS 문서 폴더가 없습니다.</p>
      <div className="modal-actions"><button className="primary-button" onClick={onSettings}>문서 연결 설정 열기</button></div>
    </>}
    {action==='open'&&!!sources?.length&&<form onSubmit={e=>{
      e.preventDefault();setBusy(true);setError('');
      void api.execute('open_project_nas_folder',{projectId:project.id,sourceId,shortcut})
        .then(onClose,e=>setError(String(e).replace(/^Error: /,''))).finally(()=>setBusy(false));
    }}>
      <label className="field">NAS 문서 폴더<select disabled={busy} value={sourceId} onChange={e=>setSourceId(e.target.value)}>
        {sources.map(source=><option value={source.sourceId} key={source.sourceId}>{source.root}</option>)}
      </select></label>
      <label className="field">바로가기<select disabled={busy} value={shortcut} onChange={e=>setShortcut(e.target.value)}>
        <option value="root">전체 폴더</option><option value="dailyLog">일자별 작업기록</option>
        <option value="checklist">개발 진행 체크리스트</option><option value="review">검토·검증 결과</option>
      </select></label>
      <div className="modal-actions"><button className="primary-button" disabled={busy}>
        {busy?'폴더 연결 확인 중':'탐색기에서 열기'}
      </button></div>
    </form>}
  </Dialog>;
}
