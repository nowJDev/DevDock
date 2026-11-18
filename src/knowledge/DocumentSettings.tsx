// 근태 설정과 독립된 문서 폴더·역할·간격 편집 및 연결 상태 화면을 제공한다.
import {useEffect,useRef,useState,type FormEvent} from 'react';
import {FolderPlus,FolderOpen,RefreshCw} from 'lucide-react';
import type {Project} from '../contracts';
import type {KnowledgeApi,SourceState} from './api';
import type {DocumentConfigView,DocumentSource} from './contracts';
const roles=[['handover','핸드오버'],['dailyLog','일자별 작업기록'],['checklist','기준 체크리스트'],['review','검토·검증 결과'],['reference','참고 문서'],['publicationReceipt','게시 영수증']];
const states:Record<string,string>={online:'연결 정상',partial:'일부만 읽음',offline:'오프라인',denied:'접근 권한 없음',timeout:'연결 시간 초과',unknown:'아직 확인 전',error:'연결 확인 실패'};
const message=(e:unknown)=>String(e).replace(/^Error: /,'');
const stamp=(v?:string)=>v?new Date(v).toLocaleString('ko-KR',{hour12:false}):'아직 확인 전';
export default function DocumentSettings({api,projects,refreshVersion}:{api:KnowledgeApi;projects:Project[];refreshVersion:number}){
 const [config,setConfig]=useState<DocumentConfigView|null>(null),[source,setSource]=useState<DocumentSource|null>(null),[interval,setIntervalValue]=useState('30'),[connection,setConnection]=useState<SourceState[]>([]),[error,setError]=useState(''),[notice,setNotice]=useState(''),[busy,setBusy]=useState(false),[probing,setProbing]=useState<string|null>(null),[remoteRevision,setRemoteRevision]=useState<number|null>(null);
 const dirty=useRef(false),intervalDirty=useRef(false),revision=useRef<number|null>(null);
 useEffect(()=>{let active=true;void Promise.all([api.config(),api.sourceStates()]).then(([cfg,states])=>{if(!active)return;setConnection(states.sources);if(!dirty.current){setConfig(cfg);setIntervalValue(String(cfg.nasIntervalSec));revision.current=cfg.revision;}else if(cfg.revision!==revision.current)setRemoteRevision(cfg.revision);},e=>{if(active)setError(message(e));});return()=>{active=false;};},[api,refreshVersion]);
 function accept(cfg:DocumentConfigView){setConfig(cfg);if(!intervalDirty.current)setIntervalValue(String(cfg.nasIntervalSec));revision.current=cfg.revision;dirty.current=intervalDirty.current||source!==null;setRemoteRevision(null);}
 function edit(value:DocumentSource){dirty.current=true;setSource(structuredClone(value));setError('');setNotice('');}
 function change(value:Partial<DocumentSource>){dirty.current=true;setSource(prev=>prev?{...prev,...value}:null);}
 async function act(action:()=>Promise<DocumentConfigView>,success:string,saveInterval=false){setBusy(true);setError('');try{const cfg=await action();if(saveInterval)intervalDirty.current=false;accept(cfg);setNotice(success);return true;}catch(e){setError(message(e));return false;}finally{setBusy(false);}}
 async function saveSource(e:FormEvent){e.preventDefault();if(!config||!source)return;const ok=await act(()=>api.upsertSource({...source,root:source.root.trim()},config.revision),'문서 폴더를 저장했습니다.');if(ok){setSource(null);dirty.current=intervalDirty.current;}}
 async function probe(id:string){setProbing(id);setError('');try{const value=await api.scan([id],true);const result=value&&typeof value==='object'&&!Array.isArray(value)&&Array.isArray(value.sources)?value.sources:[];const unsuccessful=result.some(s=>s&&typeof s==='object'&&!Array.isArray(s)&&s.state!=='online');setConnection((await api.sourceStates()).sources);setNotice(unsuccessful?'연결 상태를 확인했습니다. 마지막 정상 캐시를 유지합니다.':'문서 자료를 다시 읽었습니다.');}catch(e){setError(message(e));}finally{setProbing(null);}}
 if(!config)return <div className="knowledge-content">{error?<p role="alert" className="alert error">{error}</p>:<p role="status" className="small">문서 연결 설정을 불러오는 중입니다.</p>}</div>;
 return <div className="knowledge-content document-settings">
  <p className="knowledge-intro">핸드오버와 프로젝트 작업기록을 연결합니다. 자료를 읽은 뒤 검토한 내용을 진행 현황에 반영합니다.</p>
  {error&&<div className="alert error" role="alert">{error}</div>}{notice&&<p className="knowledge-status" role="status">{notice}</p>}
  {remoteRevision!==null&&<p className="warning-text small">다른 곳에서 문서 설정이 변경되었습니다. 입력 내용은 유지됩니다. <button type="button" className="text-button" onClick={()=>{dirty.current=false;void api.config().then(accept,e=>setError(message(e)));}}>최신 문서 설정 불러오기</button></p>}
  <form className="document-interval" onSubmit={e=>{e.preventDefault();const n=Number(interval);if(!Number.isInteger(n)||n<5||n>3600){setError('NAS 확인 간격은 5~3600초의 정수로 입력해 주세요.');return;}void act(()=>api.saveConfig({...config,nasIntervalSec:n}),'문서 확인 간격을 저장했습니다.',true);}}>
   <label className="field">NAS 확인 간격 (초)<input aria-label="NAS 확인 간격 (초)" type="number" min={5} max={3600} value={interval} onChange={e=>{dirty.current=true;intervalDirty.current=true;setIntervalValue(e.target.value);}}/><small>로컬 자료와 화면은 5초마다 확인합니다.</small></label><button className="subtle-button" type="submit" disabled={busy}>문서 확인 간격 저장</button>
  </form>
  <div className="knowledge-toolbar"><h3>연결한 문서 폴더 <span className="count">{config.sources.filter(s=>!s.archived).length}</span></h3><button type="button" className="subtle-button" disabled={busy} onClick={()=>edit({sourceId:crypto.randomUUID(),scope:'global',projectId:null,location:'local',root:'',roles:['handover'],enabled:true,include:[],exclude:[]})}><FolderPlus size={15}/>문서 폴더 추가</button></div>
  {config.sources.filter(s=>!s.archived).map(s=>{const state=connection.find(v=>v.sourceId===s.sourceId);return <article className="document-source-row" key={s.sourceId}>
   <div className="document-source-heading"><FolderOpen size={18}/><strong>{s.scope==='global'?'공통 핸드오버·문서':projects.find(p=>p.id===s.projectId)?.name??'연결 프로젝트 확인 필요'}</strong><span className={'status-pill '+(state?.state==='online'?'good':state&&state.state!=='unknown'?'warn':'neutral')}>{s.enabled?states[state?.state??'unknown']??'확인 필요':'사용 중지'}</span></div>
   <div className="source-meta"><span>{s.location==='nas'?'NAS 작업기록':'로컬 문서'}</span><span>{s.roles.map(r=>roles.find(v=>v[0]===r)?.[1]??r).join(', ')}</span></div>
   <details className="source-path"><summary>문서 위치와 자료 상태</summary><p>{s.root}</p><p>원문 확인 {stamp(state?.lastSuccessAt)}</p><p>자료 기준일은 개별 문서에서 확인합니다.</p>{state?.nextRetryAt&&state.state!=='online'&&<p>다음 연결 확인 {stamp(state.nextRetryAt)}</p>}</details>
   <div className="source-actions"><button type="button" className="subtle-button" disabled={probing!==null||!s.enabled} onClick={()=>void probe(s.sourceId)}><RefreshCw size={13} className={probing===s.sourceId?'spin':''}/>{probing===s.sourceId?'연결 확인 중':'연결 확인'}</button><button type="button" className="text-button" onClick={()=>edit(s)}>문서 연결 수정</button><button type="button" className="text-button" disabled={busy} onClick={()=>void act(()=>api.upsertSource({...s,enabled:!s.enabled},config.revision),s.enabled?'문서 자동 확인을 중지했습니다.':'문서 자동 확인을 다시 시작했습니다.')}>{s.enabled?'사용 중지':'다시 사용'}</button></div>
  </article>;})}
  {!config.sources.some(s=>!s.archived)&&<p className="small">연결한 폴더가 없습니다. 공통 핸드오버나 프로젝트 NAS 문서 폴더를 추가해 주세요.</p>}
  <p className="small">기존 캐시와 반영 이력은 유지됩니다.</p>
  {source&&<form className="document-source-editor" onSubmit={saveSource}>
   <h3>{config.sources.some(s=>s.sourceId===source.sourceId)?'문서 연결 수정':'새 문서 폴더'}</h3>
   <div className="knowledge-fields"><label className="field">저장 위치<select value={source.location} onChange={e=>change({location:e.target.value as DocumentSource['location'],roles:e.target.value==='nas'?['dailyLog','checklist']:['handover']})}><option value="local">로컬 폴더</option><option value="nas">NAS 공유 폴더</option></select></label><label className="field">연결 범위<select value={source.scope==='global'?'global':source.projectId??''} onChange={e=>change({scope:e.target.value==='global'?'global':'project',projectId:e.target.value==='global'?null:e.target.value})}><option value="global">공통 핸드오버·문서</option>{projects.filter(p=>!p.archived).map(p=><option key={p.id} value={p.id}>{p.name}</option>)}</select></label></div>
   <label className="field">문서 폴더 경로<input aria-label="문서 폴더 경로" value={source.root} required placeholder={source.location==='nas'?'\\\\server\\공유폴더\\프로젝트':'문서 폴더의 전체 경로'} onChange={e=>change({root:e.target.value})}/><small>NAS는 Windows에서 연결된 공유 폴더의 UNC 경로를 사용합니다.</small></label>
   <fieldset className="document-roles"><legend>문서 역할</legend>{roles.map(([id,label])=><label key={id}><input type="checkbox" checked={source.roles.includes(id)} onChange={e=>change({roles:e.target.checked?[...source.roles,id]:source.roles.filter(r=>r!==id)})}/>{label}</label>)}</fieldset>
   {source.location==='nas'&&<button type="button" className="text-button" onClick={()=>change({include:['일자별 작업기록','개발 진행 체크리스트','검토·검증 결과','프로젝트 개발 지침','프로젝트_문서안내.md'],roles:['dailyLog','checklist','review','reference','publicationReceipt']})}>NAS 문서 분류 제안 적용</button>}
   <div className="knowledge-fields"><label className="field">포함할 하위 경로<textarea rows={3} value={(source.include??[]).join('\n')} placeholder="비워 두면 모든 지원 문서" onChange={e=>change({include:e.target.value.split('\n').filter(Boolean)})}/><small>경로 접두어를 한 줄에 하나씩 입력합니다.</small></label><label className="field">제외할 하위 경로<textarea rows={3} value={(source.exclude??[]).join('\n')} onChange={e=>change({exclude:e.target.value.split('\n').filter(Boolean)})}/><small>빌드·임시 폴더와 외부 연결 폴더는 탐색에서 제외합니다.</small></label></div>
   <div className="knowledge-actions"><button type="button" className="subtle-button" onClick={()=>{setSource(null);dirty.current=intervalDirty.current;}}>문서 편집 취소</button><button className="primary-button" type="submit" disabled={busy||source.roles.length===0}>문서 폴더 저장</button></div>
  </form>}
 </div>;
}
