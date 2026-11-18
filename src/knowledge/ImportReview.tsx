// 근거 후보의 검토·선택·반영을 분리하고 실패한 편집 내용을 보존한다.
import {useEffect,useRef,useState} from 'react';
import type {KnowledgeApi} from './api';
import type {ImportCandidate,ImportPreview,Json} from './contracts';
const names:Record<string,string>={workEntry:'업무 기록',historicalSnapshot:'과거 진행 수치',currentTaskUpdate:'현재 작업 상태',basisDefinition:'진행 기준',projectLink:'문서 연결'};
const modes:Record<string,string[]>={history:['workEntry','historicalSnapshot'],current:['currentTaskUpdate'],basis:['basisDefinition'],links:['projectLink']};
type Edit={text:string;date:string;reason:string;open:boolean};
function changes(c:ImportCandidate):Record<string,Json>{return c.proposedChanges&&typeof c.proposedChanges==='object'&&!Array.isArray(c.proposedChanges)?c.proposedChanges:{};}
export default function ImportReview({api,preview,onApplied}:{api:KnowledgeApi;preview:ImportPreview;onApplied:()=>void}){
 const [current,setCurrent]=useState(preview),[mode,setMode]=useState('history'),[selected,setSelected]=useState<string[]>([]),[edits,setEdits]=useState<Record<string,Edit>>({}),[error,setError]=useState(''),[busy,setBusy]=useState(false),[notice,setNotice]=useState(''),[correction,setCorrection]=useState(false);
 const key=useRef<string|null>(null),basisIds=useRef<Record<string,string>>({});
 useEffect(()=>{setCurrent(preview);},[preview]);
 function edit(c:ImportCandidate){setEdits(old=>({...old,[c.candidateId]:old[c.candidateId]??{text:String(changes(c).text??''),date:c.workDate??c.effectiveDate??'',reason:'',open:true}}));}
 function update(id:string,patch:Partial<Edit>){setEdits(old=>({...old,[id]:{...old[id],...patch}}));}
 async function review(c:ImportCandidate,state:'accepted'|'rejected'){
  setBusy(true);setError('');try{
   const draft=edits[c.candidateId];let edited:Record<string,unknown>|null=null;
   if(draft?.open&&(draft.text!==String(changes(c).text??'')||draft.date!==(c.workDate??c.effectiveDate??''))){edited={...(c.kind==='workEntry'?{workDate:draft.date,proposedChanges:{...changes(c),text:draft.text}}:{effectiveDate:draft.date})};if(!draft.reason.trim())throw Error('수정 이유를 입력해 주세요.');}
   if(c.kind==='basisDefinition'&&!changes(c).basisId){basisIds.current[c.candidateId]??=crypto.randomUUID();edited={proposedChanges:{...changes(c),basisId:basisIds.current[c.candidateId],version:1}};}
   const result=await api.execute<ImportPreview>('review_import_candidate',{jobId:current.jobId,candidateId:c.candidateId,reviewState:state,edited,reason:draft?.reason||'선택한 원문 근거를 확인했습니다.',expectedProjectRevision:current.expectedProjectRevision});setCurrent(result);key.current=null;
  }catch(e){setError(String(e));}finally{setBusy(false);}
 }
 const allowed=current.candidates.filter(c=>!c.applied&&modes[mode].includes(c.kind));const canApply=selected.length>0&&selected.every(id=>allowed.some(c=>c.candidateId===id&&c.reviewState==='accepted'))&&!current.conflicts.length;
 async function apply(){if(!canApply||busy)return;setBusy(true);setError('');key.current??=crypto.randomUUID();try{await api.execute('apply_import',{selection:{jobId:current.jobId,candidateIds:selected,expectedProjectRevision:current.expectedProjectRevision,idempotencyKey:key.current,mode,correction}});setNotice('선택한 내용을 반영했습니다.');setSelected([]);key.current=null;onApplied();setCurrent(await api.preview(current.jobId));}catch(e){setError(String(e));}finally{setBusy(false);}}
 return <div className="import-review">
  <div className="knowledge-toolbar"><label>반영 대상 <select aria-label="반영 대상" value={mode} onChange={e=>{setMode(e.target.value);setSelected([]);key.current=null;}}><option value="history">과거 업무·진행 수치</option><option value="current">현재 작업 상태</option><option value="basis">진행 기준 생성</option><option value="links">문서 연결</option></select></label><span className="small">요청 {current.jobId}</span></div>
  <p className="small">{mode==='history'?'과거 기록을 가져옵니다. 현재 체크리스트 상태는 유지됩니다.':mode==='current'?'선택한 현재 작업 상태를 변경합니다. 기준과 작업 버전을 확인해 주세요.':'선택한 기준 또는 문서 연결을 반영합니다.'}</p>
  {error&&<p className="alert error" role="alert">{error}</p>}{notice&&<p className="knowledge-status" role="status">{notice}</p>}{current.conflicts.length>0&&<p className="warning-text">자료나 진행 기준이 변경되어 재검토가 필요합니다. {current.conflicts.join(', ')}</p>}
  {allowed.map(c=>{const draft=edits[c.candidateId],change=changes(c);return <article className="import-candidate" key={c.candidateId}>
   <div className="knowledge-toolbar"><label className="candidate-select"><input type="checkbox" aria-label={names[c.kind]+' 후보 선택'} checked={selected.includes(c.candidateId)} disabled={busy} onChange={e=>{setSelected(old=>e.target.checked?[...old,c.candidateId]:old.filter(id=>id!==c.candidateId));key.current=null;}}/>{names[c.kind]}</label><span className="status-pill neutral">{c.reviewState==='accepted'?'검토 승인':c.reviewState==='rejected'?'제외':'검토 대기'}</span></div>
   <p>{String(change.text??change.name??change.taskId??'원장 snapshot')}</p><p className="small">업무일 {c.workDate??c.effectiveDate??'미확인'} · 문서 작성일 {c.authoredDate??'미기록'}</p>
   {c.kind==='historicalSnapshot'&&<p className="small">집계 범위 {change.coverage==='complete'?'전체 원장':'불완전'} · 부분 업무 기록은 진행률로 합산하지 않습니다.</p>}
   {c.kind==='currentTaskUpdate'&&<p className="warning-text">{String(change.taskId)} → {String(change.state)} · 작업 버전 {String(change.expectedTaskRevision)}</p>}
   <details className="source-path"><summary>원문 근거 {c.sourceSpans.length}개</summary>{c.sourceSpans.map((s,i)=><p key={i}>문서 버전 {s.versionId} · {'lines' in s.locator?`${s.locator.lines.start}~${s.locator.lines.end}행`:s.locator.jsonPointer} · SHA-256 {s.sha256}</p>)}</details>
   {draft?.open&&<div className="candidate-editor">{c.kind==='workEntry'&&<label className="field">업무 내용<textarea aria-label="업무 내용" rows={3} value={draft.text} onChange={e=>update(c.candidateId,{text:e.target.value})}/></label>}<div className="knowledge-fields"><label className="field">업무일<input type="date" value={draft.date} onChange={e=>update(c.candidateId,{date:e.target.value})}/></label><label className="field">수정 이유<input value={draft.reason} onChange={e=>update(c.candidateId,{reason:e.target.value})}/></label></div></div>}
   <div className="source-actions"><button type="button" className="subtle-button" disabled={busy} onClick={()=>void review(c,'accepted')}>후보 승인</button><button type="button" className="text-button" disabled={busy} onClick={()=>void review(c,'rejected')}>후보 제외</button>{['workEntry','historicalSnapshot'].includes(c.kind)&&<button type="button" className="text-button" onClick={()=>edit(c)}>후보 수정</button>}</div>
  </article>;})}
  {allowed.length===0&&<p className="small">이 반영 대상에 해당하는 후보가 없습니다. 다른 대상의 후보는 별도로 검토할 수 있습니다.</p>}
  {mode==='history'&&<label className="candidate-select"><input type="checkbox" checked={correction} onChange={e=>{setCorrection(e.target.checked);key.current=null;}}/>과거 수치를 명시적 보정으로 반영</label>}
  <div className="knowledge-actions"><span className="small">선택 {selected.length}개</span><button type="button" className="primary-button" disabled={!canApply||busy} onClick={()=>void apply()}>{busy?'반영 처리 중':'선택 반영'}</button></div>
 </div>;
}
