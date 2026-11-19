// 근태 설정과 독립된 문서 폴더·역할·간격 편집 및 연결 상태 화면을 제공한다.
import {useEffect,useId,useLayoutEffect,useRef,useState,type FormEvent} from 'react';
import {FolderPlus,FolderOpen,RefreshCw} from 'lucide-react';
import type {Project} from '../contracts';
import type {KnowledgeApi,SourceState} from './api';
import type {DocumentConfigView,DocumentSource,PublishTarget} from './contracts';
import ReportSettings from './ReportSettings';
const roles=[['handover','핸드오버'],['dailyLog','일자별 작업기록'],['checklist','기준 체크리스트'],['review','검토·검증 결과'],['reference','참고 문서'],['publicationReceipt','게시 영수증']];
const states:Record<string,string>={online:'연결 정상',partial:'일부만 읽음',offline:'오프라인',denied:'접근 권한 없음',timeout:'연결 시간 초과',unknown:'아직 확인 전',error:'연결 확인 실패'};
const receiptState:Record<string,string>={published_verified:'완료 영수증 일치',devdockPending:'DevDock 게시 확인 필요',pending_final_verification:'최종 영수증 대기',prepared:'기존 게시 준비 중',conflict:'배치표와 영수증 불일치',unsupported:'지원하지 않는 배치표',unverifiedReceipt:'완료 영수증 확인 필요',unknown:'확인 전'};
const message=(e:unknown)=>String(e).replace(/^Error: /,'');
const stamp=(v?:string)=>v?new Date(v).toLocaleString('ko-KR',{hour12:false}):'아직 확인 전';
export type DocumentSettingsRequest={projectId:string;location:DocumentSource['location'];sourceId?:string};
function newSource(projectId?:string,location:DocumentSource['location']='local'):DocumentSource{
 return {sourceId:crypto.randomUUID(),scope:projectId?'project':'global',projectId:projectId??null,location,root:'',roles:location==='nas'?['dailyLog','checklist']:['handover'],enabled:true,include:[],exclude:[]};
}
export default function DocumentSettings({api,projects,refreshVersion,editorRequest,onEditorRequestHandled}:{api:KnowledgeApi;projects:Project[];refreshVersion:number;editorRequest?:DocumentSettingsRequest;onEditorRequestHandled?:()=>void}){
 const [config,setConfig]=useState<DocumentConfigView|null>(null),[source,setSource]=useState<DocumentSource|null>(null),[interval,setIntervalValue]=useState('30'),[connection,setConnection]=useState<SourceState[]>([]),[error,setError]=useState(''),[notice,setNotice]=useState(''),[busy,setBusy]=useState(false),[probing,setProbing]=useState<string|null>(null),[remoteRevision,setRemoteRevision]=useState<number|null>(null);
 const [reportOpen,setReportOpen]=useState(false);
 const [sourceError,setSourceError]=useState('');
 const editorId=useId();
 const addButton=useRef<HTMLButtonElement>(null),locationInput=useRef<HTMLSelectElement>(null),pathInput=useRef<HTMLInputElement>(null);
 const opener=useRef<HTMLButtonElement|null>(null);
 const focusField=useRef<'location'|'path'|'opener'|undefined>(undefined);
 const dirty=useRef(false),intervalDirty=useRef(false),revision=useRef<number|null>(null);
 useEffect(()=>{let active=true;void Promise.all([api.config(),api.sourceStates()]).then(([cfg,states])=>{if(!active)return;setConnection(states.sources);if(!dirty.current){setConfig(cfg);setIntervalValue(String(cfg.nasIntervalSec));revision.current=cfg.revision;}else if(cfg.revision!==revision.current)setRemoteRevision(cfg.revision);},e=>{if(active)setError(message(e));});return()=>{active=false;};},[api,refreshVersion]);
 function accept(cfg:DocumentConfigView){setConfig(cfg);if(!intervalDirty.current)setIntervalValue(String(cfg.nasIntervalSec));revision.current=cfg.revision;dirty.current=intervalDirty.current||source!==null;setRemoteRevision(null);}
 function edit(value:DocumentSource,button:HTMLButtonElement|null,field:'location'|'path'){
  dirty.current=true;
  opener.current=button;
  focusField.current=field;
  setSource(structuredClone(value));
  setSourceError('');
  setError('');
  setNotice('');
 }
 useEffect(()=>{
  if(!editorRequest||!config)return;
  const match=config.sources.find(item=>item.sourceId===editorRequest.sourceId&&item.projectId===editorRequest.projectId&&!item.archived);
  edit(match??newSource(editorRequest.projectId,editorRequest.location),addButton.current,match?'path':'location');
  onEditorRequestHandled?.();
 },[config,editorRequest,onEditorRequestHandled]);
 useLayoutEffect(()=>{
  if(!focusField.current)return;
  if(focusField.current==='opener'){
   if(busy)return;
   focusField.current=undefined;
   (opener.current?.isConnected?opener.current:addButton.current)?.focus();
   return;
  }
  if(!source)return;
  const input=focusField.current==='path'?pathInput.current:locationInput.current;
  focusField.current=undefined;
  input?.focus({preventScroll:true});
  input?.scrollIntoView?.({block:'center'});
 },[source,busy]);
 function closeEditor(){
  setSource(null);
  setSourceError('');
  dirty.current=intervalDirty.current;
  focusField.current='opener';
 }
 function change(value:Partial<DocumentSource>){dirty.current=true;setSource(prev=>prev?{...prev,...value}:null);}
 async function act(action:()=>Promise<DocumentConfigView>,success:string,saveInterval=false){setBusy(true);setError('');try{const cfg=await action();if(saveInterval)intervalDirty.current=false;accept(cfg);setNotice(success);return true;}catch(e){setError(message(e));return false;}finally{setBusy(false);}}
 async function saveSource(e:FormEvent){
  e.preventDefault();
  if(!config||!source||busy)return;
  if(!source.root.trim()){setSourceError('문서 폴더 경로를 입력해 주세요.');pathInput.current?.focus();return;}
  setBusy(true);
  setSourceError('');
  try{
   const cfg=await api.upsertSource({...source,root:source.root.trim(),publishTargets:source.scope==='project'&&source.location==='nas'?source.publishTargets?.map(t=>({...t,path:t.path.trim()})):undefined},config.revision);
   accept(cfg);
   setNotice('문서 폴더를 저장했습니다.');
   closeEditor();
  }catch(e){setSourceError(message(e));}
  finally{setBusy(false);}
 }
 async function probe(id:string){setProbing(id);setError('');try{const value=await api.scan([id],true);const result=value&&typeof value==='object'&&!Array.isArray(value)&&Array.isArray(value.sources)?value.sources:[];const unsuccessful=result.some(s=>s&&typeof s==='object'&&!Array.isArray(s)&&s.state!=='online');setConnection((await api.sourceStates()).sources);setNotice(unsuccessful?'연결 상태를 확인했습니다. 마지막 정상 캐시를 유지합니다.':'문서 자료를 다시 읽었습니다.');}catch(e){setError(message(e));}finally{setProbing(null);}}
 if(!config)return <div className="knowledge-content">{error?<p role="alert" className="alert error">{error}</p>:<p role="status" className="small">문서 연결 설정을 불러오는 중입니다.</p>}</div>;
 const existing=config.sources.find(s=>s.sourceId===source?.sourceId);
 return <div className="knowledge-content document-settings">
  <p className="knowledge-intro">핸드오버와 프로젝트 작업기록을 연결합니다. 자료를 읽은 뒤 검토한 내용을 진행 현황에 반영합니다.</p>
  {error&&<div className="alert error" role="alert">{error}</div>}{notice&&<p className="knowledge-status" role="status">{notice}</p>}
  {remoteRevision!==null&&<p className="warning-text small">다른 곳에서 문서 설정이 변경되었습니다. 입력 내용은 유지됩니다. <button type="button" className="text-button" onClick={()=>{dirty.current=false;void api.config().then(accept,e=>setError(message(e)));}}>최신 문서 설정 불러오기</button></p>}
  <form className="document-interval" onSubmit={e=>{e.preventDefault();const n=Number(interval);if(!Number.isInteger(n)||n<5||n>3600){setError('NAS 확인 간격은 5~3600초의 정수로 입력해 주세요.');return;}void act(()=>api.saveConfig({...config,nasIntervalSec:n}),'문서 확인 간격을 저장했습니다.',true);}}>
   <label className="field">NAS 확인 간격 (초)<input aria-label="NAS 확인 간격 (초)" type="number" min={5} max={3600} value={interval} onChange={e=>{dirty.current=true;intervalDirty.current=true;setIntervalValue(e.target.value);}}/><small>로컬 자료와 화면은 5초마다 확인합니다.</small></label><button className="subtle-button" type="submit" disabled={busy}>문서 확인 간격 저장</button>
  </form>
  <div className="knowledge-toolbar"><h3>연결한 문서 폴더 <span className="count">{config.sources.filter(s=>!s.archived).length}</span></h3><button type="button" ref={addButton} className="subtle-button" disabled={busy} aria-expanded={Boolean(source&&!existing)} aria-controls={source?editorId:undefined} onClick={e=>edit(newSource(),e.currentTarget,'location')}><FolderPlus size={15}/>문서 폴더 추가</button></div>
  {source&&<form className="document-source-editor" id={editorId} onSubmit={saveSource}>
   <h3>{existing?'문서 연결 수정':'새 문서 폴더'}</h3>
   {existing&&<p className="small editor-path">{existing.root}</p>}
   <div className="knowledge-fields"><label className="field">저장 위치<select ref={locationInput} value={source.location} onChange={e=>change({location:e.target.value as DocumentSource['location'],roles:e.target.value==='nas'?['dailyLog','checklist']:['handover']})}><option value="local">로컬 폴더</option><option value="nas">NAS 공유 폴더</option></select></label><label className="field">연결 범위<select value={source.scope==='global'?'global':source.projectId??''} onChange={e=>change({scope:e.target.value==='global'?'global':'project',projectId:e.target.value==='global'?null:e.target.value})}><option value="global">공통 핸드오버·문서</option>{projects.filter(p=>!p.archived).map(p=><option key={p.id} value={p.id}>{p.name}</option>)}</select></label></div>
   <label className="field">문서 폴더 경로<input aria-label="문서 폴더 경로" ref={pathInput} value={source.root} required placeholder={source.location==='nas'?'\\\\server\\공유폴더\\프로젝트':'문서 폴더의 전체 경로'} onChange={e=>change({root:e.target.value})}/><small>NAS는 Windows에서 연결된 공유 폴더의 UNC 경로를 사용합니다.</small></label>
   <fieldset className="document-roles"><legend>문서 역할</legend>{roles.map(([id,label])=><label key={id}><input type="checkbox" checked={source.roles.includes(id)} onChange={e=>change({roles:e.target.checked?[...source.roles,id]:source.roles.filter(r=>r!==id)})}/>{label}</label>)}</fieldset>
   {source.location==='nas'&&<button type="button" className="text-button" onClick={()=>change({include:['일자별 작업기록','개발 진행 체크리스트','검토·검증 결과','프로젝트 개발 지침','프로젝트_문서안내.md'],roles:['dailyLog','checklist','review','reference','publicationReceipt']})}>NAS 문서 분류 제안 적용</button>}
   {source.location==='nas'&&source.scope==='project'&&<fieldset className="publish-targets">
    <legend>NAS 게시 문서</legend><p className="small">기존 배치표에 있는 Markdown 상대 경로를 등록해 주세요.</p>
    {(source.publishTargets??[]).map((target,i)=><div className="publish-target" key={i}>
      <label className="field">문서 역할<select value={target.role} onChange={e=>change({publishTargets:source.publishTargets?.map((t,index)=>index===i?{...t,role:e.target.value as PublishTarget['role']}:t)})}>
        <option value="checklist">진행 체크리스트</option><option value="dailyLog">일자별 작업기록</option>
        <option value="status">현재 상태</option><option value="index">문서 안내</option>
      </select></label>
      <label className="field">게시 상대 경로<input required value={target.path} onChange={e=>change({publishTargets:source.publishTargets?.map((t,index)=>index===i?{...t,path:e.target.value}:t)})}/></label>
      <button type="button" className="text-button" onClick={()=>change({publishTargets:source.publishTargets?.filter((_,index)=>index!==i)})}>대상 제거</button>
    </div>)}
    <button type="button" className="subtle-button" disabled={(source.publishTargets?.length??0)>=12}
      onClick={()=>change({publishTargets:[...(source.publishTargets??[]),{role:'checklist',path:''}]})}>게시 대상 추가</button>
   </fieldset>}
   <div className="knowledge-fields"><label className="field">포함할 하위 경로<textarea rows={3} value={(source.include??[]).join('\n')} placeholder="비워 두면 모든 지원 문서" onChange={e=>change({include:e.target.value.split('\n').filter(Boolean)})}/><small>경로 접두어를 한 줄에 하나씩 입력합니다.</small></label><label className="field">제외할 하위 경로<textarea rows={3} value={(source.exclude??[]).join('\n')} onChange={e=>change({exclude:e.target.value.split('\n').filter(Boolean)})}/><small>빌드·임시 폴더와 외부 연결 폴더는 탐색에서 제외합니다.</small></label></div>
   {sourceError&&<p className="alert error document-editor-error" role="alert">{sourceError}</p>}
   <div className="knowledge-actions"><button type="button" className="subtle-button" disabled={busy} onClick={closeEditor}>문서 편집 취소</button><button className="primary-button" type="submit" disabled={busy||source.roles.length===0}>문서 폴더 저장</button></div>
  </form>}
  {config.sources.filter(s=>!s.archived).map(s=>{const state=connection.find(v=>v.sourceId===s.sourceId);return <article className="document-source-row" key={s.sourceId}>
   <div className="document-source-heading"><FolderOpen size={18}/><strong>{s.scope==='global'?'공통 핸드오버·문서':projects.find(p=>p.id===s.projectId)?.name??'연결 프로젝트 확인 필요'}</strong><span className={'status-pill '+(state?.state==='online'?'good':state&&state.state!=='unknown'?'warn':'neutral')}>{s.enabled?states[state?.state??'unknown']??'확인 필요':'사용 중지'}</span></div>
   <div className="source-meta"><span>{s.location==='nas'?'NAS 작업기록':'로컬 문서'}</span><span>{s.roles.map(r=>roles.find(v=>v[0]===r)?.[1]??r).join(', ')}</span></div>
   <details className="source-path"><summary>문서 위치와 자료 상태</summary><p>{s.root}</p><p>원문 확인 {stamp(state?.lastSuccessAt)}</p><p>자료 기준일은 개별 문서에서 확인합니다.</p>{state?.publication&&<p>게시 원장 · {receiptState[state.publication.receiptState]??state.publication.receiptState} · 문서 집합 감사 전</p>}{state?.nextRetryAt&&state.state!=='online'&&<p>다음 연결 확인 {stamp(state.nextRetryAt)}</p>}</details>
   <div className="source-actions"><button type="button" className="subtle-button" disabled={probing!==null||!s.enabled} onClick={()=>void probe(s.sourceId)}><RefreshCw size={13} className={probing===s.sourceId?'spin':''}/>{probing===s.sourceId?'연결 확인 중':'연결 확인'}</button><button type="button" className="text-button" disabled={busy} aria-expanded={source?.sourceId===s.sourceId} aria-controls={source?.sourceId===s.sourceId?editorId:undefined} onClick={e=>edit(s,e.currentTarget,'path')}>문서 연결 수정</button><button type="button" className="text-button" disabled={busy} onClick={()=>void act(()=>api.upsertSource({...s,enabled:!s.enabled},config.revision),s.enabled?'문서 자동 확인을 중지했습니다.':'문서 자동 확인을 다시 시작했습니다.')}>{s.enabled?'사용 중지':'다시 사용'}</button></div>
  </article>;})}
  {!config.sources.some(s=>!s.archived)&&<p className="small">연결한 폴더가 없습니다. 공통 핸드오버나 프로젝트 NAS 문서 폴더를 추가해 주세요.</p>}
  <p className="small">기존 캐시와 반영 이력은 유지됩니다.</p>
  {projects.some(p=>!p.archived)&&<div className="knowledge-content"><button type="button" className="subtle-button" aria-expanded={reportOpen} onClick={()=>setReportOpen(value=>!value)}>일일보고 설정</button>{reportOpen&&<ReportSettings api={api} projects={projects.filter(p=>!p.archived)} sources={config.sources}/>}</div>}
 </div>;
}
