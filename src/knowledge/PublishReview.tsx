// NAS 게시 초안의 원문, 변경될 파일과 게시·복구 결과를 검토하는 화면이다.
import {useEffect,useState} from 'react';
import Dialog from '../components/Dialog';
import type {Project} from '../contracts';
import type {KnowledgeApi} from './api';
import type {DocumentSource,DocumentVersion,ProgressBasis,SourceSpan} from './contracts';

type TaskChange={taskId:string;state:string;expectedChecked:boolean;sourceSpans:SourceSpan[]};
type Patch={path:string;body?:string;taskChanges:TaskChange[];sourceSpans:SourceSpan[]};
type Job={jobId:string;state:string;inputVersionIds:string[];patches?:Patch[]};
type Change={path:string;role:string;oldText:string;newText:string};
type Run={
  runId:string;planHash:string;state:string;files:Change[];selectedPaths?:string[];
  result?:{files:{path:string;state:string;message?:string}[]};error?:string;
};
const roleLabels:Record<string,string>={checklist:'진행 체크리스트',dailyLog:'일자별 작업기록',status:'현재 상태',index:'문서 안내'};
const stateLabels:Record<string,string>={
  previewReady:'게시 미리보기',applying:'게시 중',filesVerified:'선택 파일 확인',
  recoveryRequired:'복구 확인 필요',restoreConflicts:'후속 수정 보존',restoring:'복구 중',restored:'복구 확인',failed:'실행 중단',
  verified:'새 파일 확인',restoredFile:'이전본 확인',pending:'대기',uncertain:'확인 필요',conflict:'충돌',
  reportedComplete:'완료 보고',inProgress:'진행 중',blocked:'막힘',
};
const toggle=(values:string[],value:string,on:boolean)=>on?[...values,value]:values.filter(v=>v!==value);
const message=(error:unknown)=>String(error).replace(/^Error: /,'');

export default function PublishReview({api,project,sources,defaultSourceId,opener,onClose,initialRun}:{
  api:KnowledgeApi;project:Pick<Project,'id'|'name'>;sources:DocumentSource[];defaultSourceId?:string;
  opener:HTMLElement|null;onClose:()=>void;initialRun?:string;
}){
  const [sourceId,setSourceId]=useState(defaultSourceId??sources[0]?.sourceId??'');
  const [docs,setDocs]=useState<DocumentVersion[]>([]),[bases,setBases]=useState<ProgressBasis[]>([]);
  const [basisId,setBasisId]=useState(''),[inputIds,setInputIds]=useState<string[]>([]);
  const [targetPaths,setTargetPaths]=useState<string[]>([]);
  const [jobs,setJobs]=useState<Job[]>([]),[job,setJob]=useState<Job|null>(null);
  const [runs,setRuns]=useState<{runId:string;state:string}[]>([]),[run,setRun]=useState<Run|null>(null);
  const [taskKeys,setTaskKeys]=useState<string[]>([]),[patchPaths,setPatchPaths]=useState<string[]>([]);
  const [selected,setSelected]=useState<string[]>([]),[confirm,setConfirm]=useState<'apply'|'restore'|null>(null);
  const [busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState('');
  const source=sources.find(s=>s.sourceId===sourceId);
  const targets=source?.publishTargets??[];

  useEffect(()=>{
    let active=true;
    void api.execute<{runs:typeof runs}>('list_document_publishes',{projectId:project.id}).then(value=>{
      if(active)setRuns(value.runs??[]);
    },e=>{if(active)setError(message(e));});
    if(initialRun){
      void api.execute<Run>('get_document_publish_status',{runId:initialRun}).then(value=>{
        if(active){setRun(value);setSelected(value.files.map(f=>f.path));}
      },e=>{if(active)setError(message(e));});
    } else {
      void Promise.all([api.documents(project.id),api.progress(project.id),api.jobs(project.id)]).then(([d,p,j])=>{
        if(!active)return;
        setDocs(d.documents.filter(doc=>doc.kind!=='generatedPublication'));
        setBases(p.bases);setBasisId(p.basis?.basisId??'');
        setJobs(j.jobs.filter(x=>x.kind==='documentPublishDraft'));
      },e=>{if(active)setError(message(e));});
    }
    return()=>{active=false;};
  },[api,project.id,initialRun]);
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
        if(current.patches){
          setJob(current);
          setPatchPaths(current.patches.map(p=>p.path));
          setTaskKeys(current.patches.flatMap(p=>p.taskChanges.map(t=>p.path+'::'+t.taskId)));
          return;
        }
        if(['expired','failed'].includes(current.state)){setJob(current);return;}
      }catch(e){if(active)setError(message(e));}
      if(active)timer=setTimeout(()=>void check(),5000);
    }
    void check();
    return()=>{active=false;clearTimeout(timer);};
  },[api,job,run]);
  useEffect(()=>{
    if(!run||!['applying','restoring'].includes(run.state))return;
    let active=true;
    const timer=setTimeout(()=>void api.execute<Run>('get_document_publish_status',{runId:run.runId}).then(
      value=>{if(active)setRun(value);},e=>{if(active){setError(message(e));setRun({...run});}},
    ),3000);
    return()=>{active=false;clearTimeout(timer);};
  },[api,run]);

  async function act(action:()=>Promise<void>){
    setError('');setNotice('');setBusy(true);
    try{await action();}catch(e){setError(message(e));}finally{setBusy(false);}
  }
  function reviewRun(next:Run){
    setRun(next);
    setSelected(next.state==='previewReady'?next.files.map(f=>f.path):next.selectedPaths??[]);
    setConfirm(null);
  }
  const request=()=>void act(async()=>{
    setJob(await api.execute<Job>('create_publish_draft_request',{request:{
      projectId:project.id,sourceId,inputVersionIds:inputIds,basisId:basisId||null,paths:targetPaths,
    }}));
  });
  const prepare=()=>void act(async()=>{
    if(!job)return;
    reviewRun(await api.execute('prepare_document_publish',{
      jobId:job.jobId,paths:patchPaths,
      taskKeys:taskKeys.filter(key=>patchPaths.some(path=>key.startsWith(path+'::'))),
      inputVersionIds:job.inputVersionIds,
    }));
  });
  const apply=()=>void act(async()=>{
    if(!run||!confirm)return;
    const next=await api.execute<Run>(confirm==='apply'?'apply_document_publish':'restore_document_publish',
      {runId:run.runId,planHash:run.planHash,paths:selected});
    reviewRun(next);
  });
  const effectiveFiles=run?.state==='previewReady'?run.files:run?.files.filter(f=>run.selectedPaths?.includes(f.path));
  return <Dialog wide title={'문서 업데이트 · '+project.name} returnFocus={opener} onClose={onClose}>
    <div className="knowledge-content">
      {error&&<p className="alert error" role="alert">{error}</p>}
      {notice&&<p className="knowledge-status" role="status">{notice}</p>}
      {!job&&!run&&<>
        {!sources.length?<p className="small">설정에서 이 프로젝트의 NAS 문서 폴더와 게시 대상을 등록해 주세요.</p>:<>
          <label className="field">게시할 NAS 폴더<select value={sourceId} onChange={e=>setSourceId(e.target.value)}>
            {sources.map(s=><option key={s.sourceId} value={s.sourceId}>{s.root}</option>)}
          </select></label>
          <fieldset className="document-roles publish-choices"><legend>게시 대상</legend>
            {targets.map(t=><label key={t.path} className="candidate-select"><input type="checkbox" checked={targetPaths.includes(t.path)}
              onChange={e=>setTargetPaths(toggle(targetPaths,t.path,e.target.checked))}/><span>{roleLabels[t.role]} · {t.path}</span></label>)}
            {!targets.length&&<p className="small">설정에서 이 NAS 폴더의 게시 문서를 지정해 주세요.</p>}
          </fieldset>
          <label className="field">체크리스트 진행 기준<select value={basisId} onChange={e=>setBasisId(e.target.value)}>
            <option value="">기준 선택</option>{bases.map(b=><option key={b.basisId} value={b.basisId}>{b.name} · 버전 {b.version}</option>)}
          </select></label>
          <fieldset className="document-roles publish-choices"><legend>Codex에 전달할 원문</legend>
            {docs.map(d=><label key={d.versionId} className="candidate-select"><input type="checkbox" checked={inputIds.includes(d.versionId)}
              onChange={e=>setInputIds(toggle(inputIds,d.versionId,e.target.checked))}/><span>{d.path}</span></label>)}
            {!docs.length&&<p className="small">프로젝트 문서 탭에서 근거 자료를 연결해 주세요.</p>}
          </fieldset>
          <div className="knowledge-actions"><span className="small">선택 원문 {inputIds.length} / 16</span>
            <button className="primary-button" disabled={busy||!targetPaths.length||!inputIds.length||inputIds.length>16||
              (!basisId&&targets.some(t=>t.role==='checklist'&&targetPaths.includes(t.path)))} onClick={request}>Codex 게시 초안 요청</button>
          </div>
        </>}
        {jobs.length>0&&<><h3>이전 초안</h3>{jobs.map(j=><div className="analysis-job" key={j.jobId}><span>{j.jobId}</span>
          <button className="subtle-button" onClick={()=>setJob({...j,patches:undefined})}>초안 열기</button></div>)}</>}
        {runs.length>0&&<><h3>게시와 복구 기록</h3>{runs.map(r=><div className="analysis-job" key={r.runId}>
          <span>{stateLabels[r.state]??r.state}<small>{r.runId}</small></span>
          <button className="subtle-button" disabled={busy} onClick={()=>void act(async()=>reviewRun(await api.execute('get_document_publish_status',{runId:r.runId})))}>게시 기록 열기</button>
        </div>)}</>}
      </>}
      {job&&!run&&<>
        <p className="small source-path">Codex 요청 ID {job.jobId}</p>
        {!job.patches&&<p role="status">{['expired','failed'].includes(job.state)?'초안 요청을 다시 만들어 주세요.':'현재 Codex에 요청 ID를 전달해 주세요.'}</p>}
        <div className="knowledge-actions"><button className="subtle-button" onClick={()=>setJob(null)}>게시 대상 선택으로</button>
          <button className="subtle-button" onClick={()=>void act(async()=>{
            await navigator.clipboard.writeText('DevDock NAS 게시 초안 '+job.jobId+'. devdock-cli analysis get과 claim으로 고정된 대상·진행 기준을 확인한 뒤 documents read로 선택 원문을 읽고 documents publish-submit --stdin으로 근거가 있는 패치만 제출해 주세요.');
            setNotice('Codex 전달 안내를 복사했습니다.');
          })}>Codex 전달 안내 복사</button>
        </div>
        {job.patches?.map(p=><section className="document-source-row" key={p.path}>
          <label className="candidate-select"><input type="checkbox" checked={patchPaths.includes(p.path)}
            onChange={e=>setPatchPaths(toggle(patchPaths,p.path,e.target.checked))}/>{p.path}</label>
          {p.body&&<pre className="document-reader">{p.body}</pre>}
          {p.sourceSpans.map((s,i)=><p className="small source-path" key={i}>근거 {s.versionId} · {'lines'in s.locator?s.locator.lines.start+'~'+s.locator.lines.end+'행':s.locator.jsonPointer}</p>)}
          {p.taskChanges.map(t=><div key={t.taskId}>
            <label className="candidate-select"><input type="checkbox" disabled={!patchPaths.includes(p.path)}
              checked={taskKeys.includes(p.path+'::'+t.taskId)} onChange={e=>setTaskKeys(toggle(taskKeys,p.path+'::'+t.taskId,e.target.checked))}/>
              {t.taskId} · {t.expectedChecked?'체크됨':'미체크'} → {stateLabels[t.state]??t.state}
            </label>
            <p className="small source-path">원문 근거 {t.sourceSpans.map(s=>s.versionId).join(', ')}</p>
          </div>)}
        </section>)}
        {job.patches&&<div className="knowledge-actions"><button className="primary-button" disabled={busy||!patchPaths.length} onClick={prepare}>선택 변경 미리보기</button></div>}
      </>}
      {run&&<>
        <p className="small source-path">{stateLabels[run.state]??run.state} · {run.runId}</p>
        {run.error&&<p className="warning-text">{run.error}</p>}
        {run.state==='restoreConflicts'&&<p role="status" className="warning-text">게시 후 변경된 파일은 보존했습니다. 파일별 결과를 확인해 주세요.</p>}
        {run.state==='filesVerified'&&<p role="status">선택한 파일의 게시 결과를 다시 읽어 확인했습니다.</p>}
        {run.state==='recoveryRequired'&&<p role="status" className="warning-text">파일마다 반영 여부를 확인한 뒤 이전본 복구를 선택해 주세요.</p>}
        {run.result?.files.map(f=><p key={f.path} className="source-path">{f.path} · {stateLabels[f.state]??f.state}{f.message?' · '+f.message:''}</p>)}
        {effectiveFiles?.map(f=><div className="document-source-row" key={f.path}>
          <label className="candidate-select"><input type="checkbox" checked={selected.includes(f.path)} disabled={busy||!!confirm}
            onChange={e=>setSelected(toggle(selected,f.path,e.target.checked))}/>{f.path}</label>
          <details className="source-path"><summary>게시 전후 원문 비교</summary>
            <div className="knowledge-fields"><div><h4>게시 전</h4><pre className="document-reader">{f.oldText}</pre></div>
              <div><h4>게시 후</h4><pre className="document-reader">{f.newText}</pre></div></div>
          </details>
        </div>)}
        <div className="knowledge-actions">
          {run.state==='previewReady'&&!confirm&&<button className="primary-button" disabled={busy||!selected.length} onClick={()=>setConfirm('apply')}>선택 파일 게시</button>}
          {['filesVerified','recoveryRequired','restoreConflicts'].includes(run.state)&&!confirm&&
            <button className="subtle-button" disabled={busy||!selected.length} onClick={()=>setConfirm('restore')}>선택 파일 복구</button>}
          {!confirm&&<button className="subtle-button" disabled={busy} onClick={()=>setRun(null)}>게시 기록으로</button>}
        </div>
        {confirm&&<section className="document-source-editor">
          <p>{selected.length}개 NAS 파일을 {confirm==='apply'?'갱신합니다.':'이전본으로 복구합니다.'}</p>
          <div className="knowledge-actions"><button className="subtle-button" disabled={busy} onClick={()=>setConfirm(null)}>취소</button>
            <button className="primary-button" disabled={busy} onClick={apply}>{confirm==='apply'?'NAS 게시 실행':'NAS 복구 실행'}</button>
          </div>
        </section>}
      </>}
    </div>
  </Dialog>;
}
