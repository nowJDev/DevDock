// NAS 게시 초안의 원문, 변경될 파일과 게시·복구 결과를 검토하는 화면이다.
import {useEffect,useId,useRef,useState} from 'react';
import Dialog from '../components/Dialog';
import type {Project} from '../contracts';
import type {KnowledgeApi} from './api';
import type {DocumentSource,DocumentVersion,ProgressBasis,SourceSpan} from './contracts';
import './publishReview.css';

type TaskChange={taskId:string;state:string;expectedChecked:boolean;sourceSpans:SourceSpan[]};
type Patch={path:string;body?:string;taskChanges:TaskChange[];sourceSpans:SourceSpan[]};
type Job={jobId:string;state:string;inputVersionIds:string[];patches?:Patch[]};
type Change={path:string;role:string;oldText:string;newText:string};
type Run={
  runId:string;planHash:string;state:string;files:Change[];selectedPaths?:string[]|null;
  result?:{files:{path:string;state:string;message?:string}[]}|null;error?:string|null;
};
const pendingRun=(state:string)=>state==='applying'||state==='restoring';
const restorable=(state:string)=>['filesVerified','recoveryRequired','restoreConflicts','partiallyRestored'].includes(state);
const roleLabels:Record<string,string>={checklist:'진행 체크리스트',dailyLog:'일자별 작업기록',status:'현재 상태',index:'문서 안내'};
const stateLabels:Record<string,string>={
  previewReady:'게시 미리보기',applying:'게시 중',filesVerified:'선택 파일 확인',
  recoveryRequired:'복구 확인 필요',restoreConflicts:'후속 수정 보존',partiallyRestored:'선택 복구 확인',restoring:'복구 중',restored:'복구 확인',failed:'실행 중단',
  verified:'새 파일 확인',restoredFile:'이전본 확인',pending:'대기',uncertain:'확인 필요',conflict:'충돌',
  reportedComplete:'완료 보고',inProgress:'진행 중',blocked:'막힘',
};
const toggle=(values:string[],value:string,on:boolean)=>on?[...values,value]:values.filter(v=>v!==value);
const message=(error:unknown)=>String(error).replace(/^Error: /,'');

export default function PublishReview({api,project,sources,defaultSourceId,opener,onClose,initialRun,onSettings,onProgressSettings}:{
  api:KnowledgeApi;project:Pick<Project,'id'|'name'>;sources:DocumentSource[];defaultSourceId?:string;
  opener:HTMLElement|null;onClose:()=>void;initialRun?:string;onSettings?:(sourceId:string)=>void;onProgressSettings?:()=>void;
}){
  const [sourceId,setSourceId]=useState(defaultSourceId??sources[0]?.sourceId??'');
  const [docs,setDocs]=useState<DocumentVersion[]>([]),[bases,setBases]=useState<ProgressBasis[]>([]);
  const [basisId,setBasisId]=useState(''),[inputIds,setInputIds]=useState<string[]>([]);
  const [targetPaths,setTargetPaths]=useState<string[]>([]);
  const [jobs,setJobs]=useState<Job[]>([]),[job,setJob]=useState<Job|null>(null);
  const [runs,setRuns]=useState<{runId:string;state:string}[]>([]),[run,setRun]=useState<Run|null>(null);
  const [taskKeys,setTaskKeys]=useState<string[]>([]),[patchPaths,setPatchPaths]=useState<string[]>([]);
  const [selected,setSelected]=useState<string[]>([]),[confirm,setConfirm]=useState<'apply'|'restore'|null>(null);
  const [busy,setBusy]=useState(''),[error,setError]=useState(''),[notice,setNotice]=useState('');
  const [loading,setLoading]=useState(true),[loadError,setLoadError]=useState(''),[historyError,setHistoryError]=useState('');
  const [loadAttempt,reload]=useState(0),[unconfirmedError,setUnconfirmedError]=useState('');
  const [analysisError,setAnalysisError]=useState('');
  const actionInFlight=useRef(false),requirementsId=useId();
  const source=sources.find(s=>s.sourceId===sourceId);
  const targets=source?.publishTargets??[];
  const needsChecklistBasis=!basisId&&targets.some(t=>t.role==='checklist'&&targetPaths.includes(t.path));
  const needsStatusBasis=!basisId&&targets.some(t=>t.role==='status'&&targetPaths.includes(t.path));
  const draftRequirements=[
    !targets.length?'설정에서 이 NAS 폴더의 게시 문서를 지정해 주세요.':!targetPaths.length?'게시 대상을 1개 이상 선택해 주세요.':'',
    !inputIds.length||inputIds.length>16?'원문을 1~16개 선택해 주세요.':'',
    needsChecklistBasis?'체크리스트를 갱신하려면 진행 기준이 필요합니다.':'',
    needsStatusBasis?'현재 상태 문서를 갱신하려면 진행 기준이 필요합니다.':'',
  ].filter(Boolean);

  useEffect(()=>{
    let active=true;
    void api.execute<{runs:typeof runs}>('list_document_publishes',{projectId:project.id}).then(value=>{
      if(active){setRuns(value.runs??[]);setHistoryError('');}
    },e=>{if(active)setHistoryError(message(e));});
    return()=>{active=false;};
  },[api,project.id,loadAttempt]);
  useEffect(()=>{
    let active=true;
    const initialize=initialRun
      ?api.execute<Run>('get_document_publish_status',{runId:initialRun}).then(value=>{
        if(active){
          setRun(value);
          setSelected(value.state==='previewReady'?value.files.map(f=>f.path):value.selectedPaths??[]);
        }
      })
      :Promise.all([api.documents(project.id),api.progress(project.id),api.jobs(project.id)]).then(([d,p,j])=>{
        if(!active)return;
        setDocs(d.documents.filter(doc=>doc.kind!=='generatedPublication'));
        setBases(p.bases);setBasisId(p.basis?.basisId??'');
        setJobs(j.jobs.filter(x=>x.kind==='documentPublishDraft'));
      });
    void initialize.catch(e=>{if(active)setLoadError(message(e));}).finally(()=>{if(active)setLoading(false);});
    return()=>{active=false;};
  },[api,project.id,initialRun,loadAttempt]);
  useEffect(()=>{
    setTargetPaths((source?.publishTargets??[]).map(target=>target.path));
  },[source]);
  useEffect(()=>{
    if(!job||job.patches||run||['expired','failed'].includes(job.state))return;
    let active=true;
    let timer:ReturnType<typeof setTimeout>;
    async function check(){
      try{
        const current=await api.execute<Job>('get_analysis_job',{jobId:job!.jobId});
        if(!active)return;
        setAnalysisError('');
        if(current.patches){
          setJob(current);
          setPatchPaths(current.patches.map(p=>p.path));
          setTaskKeys(current.patches.flatMap(p=>p.taskChanges.map(t=>p.path+'::'+t.taskId)));
          return;
        }
        if(['expired','failed'].includes(current.state)){setJob(current);return;}
      }catch(e){if(active)setAnalysisError(message(e));}
      if(active)timer=setTimeout(()=>void check(),5000);
    }
    void check();
    return()=>{active=false;clearTimeout(timer);};
  },[api,job,run]);
  useEffect(()=>{
    if(!run||!pendingRun(run.state))return;
    let active=true;
    const timer=setTimeout(()=>void api.execute<Run>('get_document_publish_status',{runId:run.runId}).then(
      value=>{
        if(!active)return;
        setError('');
        setRun(value);
        if(!pendingRun(value.state)){
          setSelected(old=>old.filter(path=>value.selectedPaths?.includes(path)));
          setRuns(old=>[{runId:value.runId,state:value.state},...old.filter(entry=>entry.runId!==value.runId)].slice(0,20));
        }
      },e=>{if(active){setError(message(e));setRun({...run});}},
    ),3000);
    return()=>{active=false;clearTimeout(timer);};
  },[api,run]);

  async function act(label:string,action:()=>Promise<void>){
    if(actionInFlight.current)return;
    actionInFlight.current=true;
    setError('');setNotice('');setBusy(label);
    try{await action();}catch(e){setError(message(e));}finally{actionInFlight.current=false;setBusy('');}
  }
  function reviewRun(next:Run,preserveSelection=false){
    const eligible=next.state==='previewReady'?next.files.map(f=>f.path):next.selectedPaths??[];
    setRun(next);
    setSelected(old=>preserveSelection?old.filter(path=>eligible.includes(path)):eligible);
    setConfirm(null);
    setRuns(old=>[{runId:next.runId,state:next.state},...old.filter(entry=>entry.runId!==next.runId)].slice(0,20));
  }
  function loadAgain(){
    setLoading(true);setLoadError('');reload(n=>n+1);
  }
  const request=()=>void act('Codex 게시 초안 요청 중…',async()=>{
    const next=await api.execute<Job>('create_publish_draft_request',{request:{
      projectId:project.id,sourceId,inputVersionIds:inputIds,basisId:basisId||null,paths:targetPaths,
    }});
    setJob(next);
    setJobs(old=>[next,...old.filter(entry=>entry.jobId!==next.jobId)]);
  });
  const prepare=()=>void act('변경 미리보기 준비 중…',async()=>{
    if(!job)return;
    reviewRun(await api.execute('prepare_document_publish',{
      jobId:job.jobId,paths:patchPaths,
      taskKeys:taskKeys.filter(key=>patchPaths.some(path=>key.startsWith(path+'::'))),
      inputVersionIds:job.inputVersionIds,
    }));
  });
  async function verifyOutcome(current:Run,originalError:string){
    try{
      const status=await api.execute<Run>('get_document_publish_status',{runId:current.runId});
      setUnconfirmedError('');
      // 게시 전 상태가 확인되면 사용자가 검토 중이던 선택을 유지한다.
      const samePreview=status.state==='previewReady'&&status.planHash===current.planHash;
      if(samePreview)setRun(status);
      else reviewRun(status,true);
      setError(originalError);
    }catch(e){
      setUnconfirmedError(originalError);
      setError(originalError+' 게시 상태를 확인하지 못했습니다. '+message(e));
    }
  }
  function apply(){
    if(!run||!confirm||unconfirmedError)return;
    const operation=confirm;
    void act(operation==='apply'?'NAS 게시 중…':'NAS 복구 중…',async()=>{
      try{
        reviewRun(await api.execute<Run>(operation==='apply'?'apply_document_publish':'restore_document_publish',
          {runId:run.runId,planHash:run.planHash,paths:selected}),true);
      }catch(e){await verifyOutcome(run,message(e));}
    });
  }
  function openJob(next:Job|null){
    setJob(next);setError('');setAnalysisError('');setNotice('');
  }
  function closeRun(){
    setRun(null);openJob(null);
  }
  const effectiveFiles=run?.state==='previewReady'?run.files:run?.files.filter(f=>run.selectedPaths?.includes(f.path));
  const runPending=!!run&&pendingRun(run.state);
  const footerError=[...new Set([error,run?.error||'',run?'':analysisError,loadError].filter(Boolean))].join(' ');
  return <Dialog wide className="publish-dialog" title={'문서 업데이트 · '+project.name} returnFocus={opener} onClose={onClose}>
    <div className="knowledge-content publish-dialog-body" aria-busy={!!busy||runPending||loading}>
      {!job&&!run&&!initialRun&&<>
        {!sources.length?<p className="small">설정에서 이 프로젝트의 NAS 문서 폴더와 게시 대상을 등록해 주세요.</p>:<>
          <label className="field">게시할 NAS 폴더<select disabled={!!busy} value={sourceId} onChange={e=>setSourceId(e.target.value)}>
            {sources.map(s=><option key={s.sourceId} value={s.sourceId}>{s.root}</option>)}
          </select></label>
          <fieldset className="document-roles publish-choices" disabled={!!busy}><legend>게시 대상</legend>
            {targets.map(t=><label key={t.path} className="candidate-select"><input type="checkbox" checked={targetPaths.includes(t.path)}
              onChange={e=>setTargetPaths(toggle(targetPaths,t.path,e.target.checked))}/><span>{roleLabels[t.role]} · {t.path}</span></label>)}
            {!targets.length&&<p className="small">설정에서 이 NAS 폴더의 게시 문서를 지정해 주세요.</p>}
          </fieldset>
          <label className="field">게시 문서의 진행 기준<select disabled={!!busy||loading} value={basisId} onChange={e=>setBasisId(e.target.value)}>
            <option value="">기준 선택</option>{bases.map(b=><option key={b.basisId} value={b.basisId}>{b.name} · 버전 {b.version}</option>)}
          </select></label>
          <fieldset className="document-roles publish-choices" disabled={!!busy}><legend>Codex에 전달할 원문</legend>
            {docs.map(d=><label key={d.versionId} className="candidate-select"><input type="checkbox" checked={inputIds.includes(d.versionId)}
              onChange={e=>setInputIds(toggle(inputIds,d.versionId,e.target.checked))}/><span>{d.path}</span></label>)}
            {!loading&&!loadError&&!docs.length&&<p className="small">프로젝트 문서 탭에서 근거 자료를 연결해 주세요.</p>}
          </fieldset>
        </>}
        {jobs.length>0&&<><h3>이전 초안</h3>{jobs.map(j=><div className="analysis-job" key={j.jobId}><span>{j.jobId}</span>
          <button className="subtle-button" disabled={!!busy} onClick={()=>openJob({...j,patches:undefined})}>초안 열기</button></div>)}</>}
        {runs.length>0&&<><h3>게시와 복구 기록</h3>{runs.map(r=><div className="analysis-job" key={r.runId}>
          <span>{stateLabels[r.state]??r.state}<small>{r.runId}</small></span>
          <button className="subtle-button" disabled={!!busy} onClick={()=>void act('게시 기록 확인 중…',async()=>reviewRun(await api.execute('get_document_publish_status',{runId:r.runId})))}>게시 기록 열기</button>
        </div>)}</>}
        {historyError&&<p className="small warning-text">게시와 복구 기록을 불러오지 못했습니다. {historyError}</p>}
      </>}
      {job&&!run&&<>
        <p className="small source-path">Codex 요청 ID {job.jobId}</p>
        {!job.patches&&<p role="status">{['expired','failed'].includes(job.state)?'초안 요청을 다시 만들어 주세요.':'현재 Codex에 요청 ID를 전달해 주세요.'}</p>}
        {job.patches?.map(p=><section className="document-source-row" key={p.path}>
          <label className="candidate-select"><input type="checkbox" checked={patchPaths.includes(p.path)}
            disabled={!!busy} onChange={e=>setPatchPaths(toggle(patchPaths,p.path,e.target.checked))}/>{p.path}</label>
          {p.body&&<pre className="document-reader">{p.body}</pre>}
          {p.sourceSpans.map((s,i)=><p className="small source-path" key={i}>근거 {s.versionId} · {'lines'in s.locator?s.locator.lines.start+'~'+s.locator.lines.end+'행':s.locator.jsonPointer}</p>)}
          {p.taskChanges.map(t=><div key={t.taskId}>
            <label className="candidate-select"><input type="checkbox" disabled={!!busy||!patchPaths.includes(p.path)}
              checked={taskKeys.includes(p.path+'::'+t.taskId)} onChange={e=>setTaskKeys(toggle(taskKeys,p.path+'::'+t.taskId,e.target.checked))}/>
              {t.taskId} · {t.expectedChecked?'체크됨':'미체크'} → {stateLabels[t.state]??t.state}
            </label>
            <p className="small source-path">원문 근거 {t.sourceSpans.map(s=>s.versionId).join(', ')}</p>
          </div>)}
        </section>)}
      </>}
      {run&&<>
        <p className="small source-path">{stateLabels[run.state]??run.state} · {run.runId}</p>
        {run.result?.files?.map(f=><p key={f.path} className="source-path">{f.path} · {stateLabels[f.state]??f.state}{f.message?' · '+f.message:''}</p>)}
        {effectiveFiles?.map(f=><div className="document-source-row" key={f.path}>
          <label className="candidate-select"><input type="checkbox" checked={selected.includes(f.path)} disabled={!!busy||!!confirm||runPending||!!unconfirmedError||(!restorable(run.state)&&run.state!=='previewReady')}
            onChange={e=>setSelected(toggle(selected,f.path,e.target.checked))}/>{f.path}</label>
          <details className="source-path"><summary>게시 전후 원문 비교</summary>
            <div className="knowledge-fields"><div><h4>게시 전</h4><pre className="document-reader">{f.oldText}</pre></div>
              <div><h4>게시 후</h4><pre className="document-reader">{f.newText}</pre></div></div>
          </details>
        </div>)}
      </>}
    </div>
    <footer className="publish-dialog-footer" role="group" aria-label="문서 업데이트 작업">
      {footerError&&<p className="alert error" role="alert">{footerError}</p>}
      {notice&&<p className="knowledge-status" role="status">{notice}</p>}
      {loading&&<p className="small" role="status">게시 정보를 불러오는 중입니다.</p>}
      {(busy||runPending)&&<p className="small" role="status">{busy||stateLabels[run!.state]+'…'}</p>}
      {loadError&&!loading&&<div className="knowledge-actions"><button className="subtle-button" onClick={loadAgain}>다시 불러오기</button></div>}
      {!job&&!run&&!initialRun&&<>
        {!loading&&!loadError&&draftRequirements.length>0&&<ul className="publish-requirements small" id={requirementsId}>
          {draftRequirements.map(reason=><li key={reason}>{reason}</li>)}
        </ul>}
        <div className="knowledge-actions">
          {!targets.length&&onSettings&&<button className="subtle-button" disabled={!!busy} onClick={()=>onSettings(sourceId)}>문서 연결 설정 열기</button>}
          {!loading&&!loadError&&(needsChecklistBasis||needsStatusBasis)&&onProgressSettings&&
            <button className="subtle-button" disabled={!!busy} onClick={onProgressSettings}>진행 기준 설정</button>}
          <span className="small">선택 원문 {inputIds.length} / 16</span>
          <button className="primary-button" disabled={!!busy||loading||!!loadError||draftRequirements.length>0}
            aria-describedby={!loading&&!loadError&&draftRequirements.length>0?requirementsId:undefined}
            onClick={request}>Codex 게시 초안 요청</button>
        </div>
      </>}
      {job&&!run&&<div className="knowledge-actions">
        <button className="subtle-button" disabled={!!busy} onClick={()=>openJob(null)}>게시 대상 선택으로</button>
        <button className="subtle-button" disabled={!!busy} onClick={()=>void act('전달 안내 복사 중…',async()=>{
          await navigator.clipboard.writeText('DevDock NAS 게시 초안 '+job.jobId+'. devdock-cli analysis get과 claim으로 고정된 대상·진행 기준을 확인한 뒤 documents read로 선택 원문을 읽고 documents publish-submit --stdin으로 근거가 있는 패치만 제출해 주세요.');
          setNotice('Codex 전달 안내를 복사했습니다.');
        })}>Codex 전달 안내 복사</button>
        {job.patches&&<><span className="small">선택 대상 {patchPaths.length}개</span>
          <button className="primary-button" disabled={!!busy||!patchPaths.length} onClick={prepare}>선택 변경 미리보기</button></>}
      </div>}
      {run&&<>
        {run.state==='restoreConflicts'&&<p role="status" className="warning-text">게시 후 변경된 파일은 보존했습니다. 파일별 결과를 확인해 주세요.</p>}
        {run.state==='filesVerified'&&<p role="status">선택한 파일의 게시 결과를 다시 읽어 확인했습니다.</p>}
        {run.state==='recoveryRequired'&&<p role="status" className="warning-text">파일마다 반영 여부를 확인한 뒤 이전본 복구를 선택해 주세요.</p>}
        {unconfirmedError?<div className="knowledge-actions">
          <button className="subtle-button" disabled={!!busy} onClick={()=>void act('게시 상태 확인 중…',()=>verifyOutcome(run,unconfirmedError))}>게시 상태 다시 확인</button>
        </div>:confirm?<>
          <p>{selected.length}개 NAS 파일을 {confirm==='apply'?'갱신합니다.':'이전본으로 복구합니다.'}</p>
          <div className="knowledge-actions">
            <button className="subtle-button" autoFocus disabled={!!busy} onClick={()=>{setConfirm(null);setError('');}}>취소</button>
            <button className="primary-button" disabled={!!busy} onClick={apply}>{confirm==='apply'?'NAS 게시 실행':'NAS 복구 실행'}</button>
          </div>
        </>:<div className="knowledge-actions">
          {initialRun?<button className="subtle-button" disabled={!!busy} onClick={onClose}>닫기</button>
            :<button className="subtle-button" disabled={!!busy||runPending} onClick={closeRun}>게시 기록으로</button>}
          {(run.state==='previewReady'||restorable(run.state))&&<span className="small">선택 파일 {selected.length}개</span>}
          {run.state==='previewReady'&&<button className="primary-button" disabled={!!busy||!selected.length} onClick={()=>setConfirm('apply')}>선택 파일 게시</button>}
          {restorable(run.state)&&<button className="subtle-button" disabled={!!busy||!selected.length} onClick={()=>setConfirm('restore')}>선택 파일 복구</button>}
        </div>}
      </>}
    </footer>
  </Dialog>;
}
