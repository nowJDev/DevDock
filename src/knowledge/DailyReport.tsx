// 선택한 원문으로 Codex 보고를 요청하고 초안의 검토와 로컬 저장을 제공한다.
import {useEffect,useState,type FormEvent} from 'react';
import Dialog from '../components/Dialog';
import type {Project} from '../contracts';
import type {KnowledgeApi} from './api';
import type {DocumentSource,DocumentVersion,ReportProfile} from './contracts';

type Request = {requestId:string;jobId:string};
type Grounding = {reportLine:number;sourceIndex:number;startLine:number;endLine:number;classification:string};
type Draft = {body:string;needsMoreSources:boolean;grounding:Grounding[]};
type Result = Request & {state:string;draft:Draft|null;sources:{path:string;sha256:string}[]};
type Validation = {lineCount:number;issues:string[];machine:{state:string;result?:string}};
type SavePreview = {destination:string;exists:boolean;sha256:string|null};
type Selection = {path:string;expectedSha256?:string};

const errorMessage=(error:unknown)=>String(error).replace(/^Error: /,'');
const currentDate=()=>new Intl.DateTimeFormat('en-CA',{timeZone:'Asia/Seoul'}).format(new Date());
const reportFiles=(documents:DocumentVersion[],sources:DocumentSource[])=>{
  const unique=new Map<string,DocumentVersion>();
  for(const doc of documents){
    if(!/\.(?:md|txt)$/i.test(doc.path)||!sources.some(s=>s.sourceId===doc.sourceId&&s.enabled&&!s.archived))continue;
    const key=doc.sourceId+'/'+doc.path.toLowerCase();
    if(!unique.has(key))unique.set(key,doc);
  }
  return [...unique.values()];
};
const fullPath=(root:string,relative:string)=>root.replace(/[\\/]+$/,'')+'/'+relative;
const classifications:Record<string,string>={
  design:'설계',implementation:'구현',automatedTest:'자동 검증',build:'빌드',remotePush:'원격 반영',
  hardwareAcceptance:'실기 수락',unfinished:'미완료',other:'기타',
};

export default function DailyReport({api,project,returnFocus,onClose,initialRequest}:{
  api:KnowledgeApi;project:Pick<Project,'id'|'name'>;returnFocus:HTMLElement|null;onClose:()=>void;initialRequest?:Request;
}){
  const [profile,setProfile]=useState<ReportProfile|null>(null);
  const [sources,setSources]=useState<DocumentSource[]>([]),[docs,setDocs]=useState<DocumentVersion[]>([]);
  const [date,setDate]=useState(currentDate),[heading,setHeading]=useState('');
  const [mode,setMode]=useState<ReportProfile['templateMode']>('default'),[custom,setCustom]=useState('');
  const [selected,setSelected]=useState<string[]>([]),[extraPaths,setExtraPaths]=useState(''),[crossProject,setCrossProject]=useState(false);
  const [request,setRequest]=useState<Request|undefined>(initialRequest),[result,setResult]=useState<Result|null>(null);
  const [body,setBody]=useState(''),[checked,setChecked]=useState<Validation|null>(null);
  const [destination,setDestination]=useState(''),[savePreview,setSavePreview]=useState<SavePreview|null>(null);
  const [busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState('');
  const [page,setPage]=useState(0),[more,setMore]=useState(false);
  const [recent,setRecent]=useState<(Request&{state:string;createdAt:string})[]>([]);

  useEffect(()=>{
    let current=true;
    void Promise.all([api.execute<ReportProfile>('get_report_profile',{projectId:project.id}),api.config(),api.documents(project.id)])
      .then(([p,c,d])=>{
        if(!current)return;
        setProfile(p);setHeading(p.workHeading||project.name);setMode(p.templateMode);setCustom(p.customTemplate??'');
        setSources(c.sources);setDocs(d.documents);setMore(d.documents.length===d.pageSize);
      },e=>{if(current)setError(errorMessage(e));});
    void api.jobs(project.id).then(({jobs})=>{
      if(current)setRecent(jobs.filter(j=>j.kind==='dailyReport'&&j.requestId&&j.state!=='resultExpired').map(j=>({
        requestId:j.requestId!,jobId:j.jobId,state:j.state,createdAt:j.createdAt,
      })));
    },()=>{});
    return()=>{current=false;};
  },[api,project.id,project.name]);
  useEffect(()=>{
    if(!request)return;
    let current=true;
    let timer:ReturnType<typeof setTimeout>|undefined;
    async function load(){
      try{
        const response=await api.execute<Result>('get_report_draft',{requestId:request!.requestId});
        if(!current)return;
        setResult(response);
        if(response.draft){setBody(response.draft.body);return;}
        if(['resultExpired','failed','cancelled'].includes(response.state))return;
      }catch(e){if(current)setError(errorMessage(e));}
      if(current)timer=setTimeout(()=>void load(),5000);
    }
    void load();
    return()=>{current=false;clearTimeout(timer);};
  },[api,request]);

  const files=reportFiles(docs,sources);
  const typed=extraPaths.split(/\r?\n/).map(line=>line.trim()).filter(Boolean);
  const count=selected.length+typed.length;
  const changed=!!result?.draft&&body!==result.draft.body;
  async function run(action:()=>Promise<void>){
    setBusy(true);setError('');setNotice('');
    try{await action();}catch(e){setError(errorMessage(e));}finally{setBusy(false);}
  }
  function create(event:FormEvent){
    event.preventDefault();
    const selectedFiles:Selection[]=files.filter(d=>selected.includes(d.versionId)).map(d=>({
      path:fullPath(sources.find(s=>s.sourceId===d.sourceId)!.root,d.path),expectedSha256:d.sha256,
    }));
    selectedFiles.push(...typed.map(path=>({path})));
    void run(async()=>{
      const next=await api.execute<Request>('create_report_request',{request:{
        projectId:project.id,date,heading:heading.trim(),selectedFiles,templateMode:mode,
        customTemplate:mode==='custom'?custom:null,allowCrossProject:crossProject,
      }});
      setDestination(profile?.localSaveDirectory?fullPath(profile.localSaveDirectory,date+' 일일보고.txt'):'');
      setRecent(previous=>[{...next,state:'pending',createdAt:new Date().toISOString()},...previous]);
      setRequest(next);
    });
  }
  function reset(){
    setRequest(undefined);setResult(null);setBody('');setChecked(null);setSavePreview(null);setNotice('');setError('');
  }
  const handoff=request&&'DevDock 일일보고 요청 '+request.requestId+' (작업 '+request.jobId+'). devdock-cli reports context로 요청과 daily-work-report 규칙을 읽고, reports claim과 reports read-source로 선택 자료 전체를 읽어 주세요. 결과와 읽기 증명은 reports submit --stdin 으로 제출해 주세요.';
  return <Dialog title={'일일보고 · '+project.name} wide returnFocus={returnFocus} onClose={onClose}>
    {error&&<p className="alert error" role="alert">{error}</p>}
    {notice&&<p className="knowledge-status" role="status">{notice}</p>}
    {!request&&profile&&<form className="knowledge-content" onSubmit={create}>
      {recent.length>0&&<details className="source-path"><summary>이전 보고 요청</summary>
        {recent.map(item=><div className="analysis-job" key={item.requestId}>
          <span>{new Date(item.createdAt).toLocaleString('ko-KR')}<small>{item.requestId}</small></span>
          <button className="subtle-button" type="button" disabled={busy} onClick={()=>setRequest(item)}>요청 열기</button>
        </div>)}
      </details>}
      <div className="knowledge-fields">
        <label className="field">보고 날짜<input type="date" required value={date} onChange={e=>setDate(e.target.value)}/></label>
        <label className="field">업무 헤더<input required maxLength={1024} value={heading} onChange={e=>setHeading(e.target.value)}/></label>
      </div>
      <label className="field">보고 양식<select value={mode} onChange={e=>setMode(e.target.value as typeof mode)}>
        <option value="default">기본 · 10~12줄</option><option value="custom">사용자 지정</option>
      </select></label>
      {mode==='custom'&&<label className="field">사용자 지정 양식<textarea maxLength={12000} rows={4} value={custom} onChange={e=>setCustom(e.target.value)}/></label>}
      <fieldset className="document-roles">
        <legend>보고에 사용할 문서</legend>
        {!files.length&&<p className="small">연결한 Markdown·텍스트 문서가 없습니다.</p>}
        {files.map(d=><label className="candidate-select" key={d.versionId}>
          <input type="checkbox" checked={selected.includes(d.versionId)} onChange={e=>{
            setSelected(previous=>e.target.checked?[...previous,d.versionId]:previous.filter(id=>id!==d.versionId));
          }}/><span>{d.path}</span>
        </label>)}
      </fieldset>
      {more&&<button className="subtle-button" type="button" disabled={busy} onClick={()=>void run(async()=>{
        const next=await api.documents(project.id,page+1);
        setDocs(previous=>[...previous,...next.documents]);setPage(next.page);setMore(next.documents.length===next.pageSize);
      })}>문서 더 보기</button>}
      <label className="field">추가 원문 파일<textarea rows={2} placeholder="md 또는 txt 파일의 전체 경로, 한 줄에 하나" value={extraPaths} onChange={e=>setExtraPaths(e.target.value)}/></label>
      {typed.length>0&&<label className="candidate-select"><input type="checkbox" checked={crossProject} onChange={e=>setCrossProject(e.target.checked)}/>입력한 경로에 다른 프로젝트 자료 포함 허용</label>}
      <div className="knowledge-actions"><span className="small">선택 {count}개 / 최대 16개</span>
        <button className="primary-button" disabled={busy||count<1||count>16}>Codex 보고 요청</button>
      </div>
    </form>}
    {!request&&!profile&&!error&&<p role="status">보고 설정을 불러오는 중입니다.</p>}
    {request&&<div className="knowledge-content">
      <p className="small source-path">요청 ID <strong>{request.requestId}</strong></p>
      {!result?.draft&&<p role="status">{result?.state==='resultExpired'?'이전 초안이 만료되었습니다.':result?.state==='claimed'?'Codex가 자료를 분석 중입니다.':result?.state==='failed'?'보고 작성에 실패했습니다.':'요청 ID를 현재 Codex에 전달해 주세요.'}</p>}
      <div className="knowledge-actions">
        <button className="subtle-button" onClick={()=>void run(async()=>{await navigator.clipboard.writeText(handoff??'');setNotice('Codex 전달 안내를 복사했습니다.');})}>Codex 전달 안내 복사</button>
        <button className="subtle-button" disabled={busy} onClick={reset}>새 보고 요청</button>
      </div>
      {result?.draft&&<>
        {result.draft.needsMoreSources&&<p className="warning-text">선택한 근거가 부족합니다.</p>}
        <label className="field">보고서 본문<textarea rows={11} value={body} onChange={e=>{
          setBody(e.target.value);setChecked(null);setSavePreview(null);
        }} disabled={busy}/></label>
        <details className="source-path"><summary>Codex 초안 근거 {result.draft.grounding.length}개</summary>
          {changed&&<p>편집 전 초안의 행 번호입니다.</p>}
          {result.draft.grounding.map((g,i)=><p key={i}>
            초안 {g.reportLine}행 · {classifications[g.classification]??g.classification} · {result.sources[g.sourceIndex]?.path} {g.startLine}~{g.endLine}행
          </p>)}
        </details>
        <div className="knowledge-actions">
          <button className="subtle-button" disabled={busy||!body.trim()} onClick={()=>void run(async()=>setChecked(await api.execute('validate_report_draft',{requestId:request.requestId,body})))}>형식 검사</button>
          <button className="subtle-button" disabled={busy||!body.trim()} onClick={()=>void run(async()=>{await navigator.clipboard.writeText(body);setNotice('보고서 본문을 복사했습니다.');})}>본문 복사</button>
        </div>
        {checked&&<div role="status" className="source-path">
          <p>{checked.lineCount}줄 · {checked.machine.state==='passed'?'기계 검사 통과':checked.machine.state==='failed'?'기계 검사에서 수정 사항 발견':'선택 검사기를 사용할 수 없습니다.'} · 내용과 원문은 직접 대조해 주세요.</p>
          {checked.issues.map(issue=><p className="warning-text" key={issue}>{issue}</p>)}
          {checked.machine.state==='failed'&&checked.machine.result&&<pre className="document-reader">{checked.machine.result}</pre>}
        </div>}
        <form onSubmit={e=>{e.preventDefault();void run(async()=>{
          if(!savePreview){setSavePreview(await api.execute('preview_report_save',{requestId:request.requestId,destination:destination.trim()}));return;}
          await api.execute('save_report_draft',{requestId:request.requestId,body,destination:savePreview.destination,
            expectedSha256:savePreview.sha256,overwrite:savePreview.exists});
          setSavePreview(null);setNotice('보고서를 로컬 파일로 저장했습니다.');
        });}}>
          <label className="field">로컬 저장 파일 경로<input required value={destination} onChange={e=>{setDestination(e.target.value);setSavePreview(null);}}/></label>
          {savePreview&&<p className="source-path small">{savePreview.exists?'아래 파일을 덮어씁니다.':'아래 위치에 새 파일을 만듭니다.'} {savePreview.destination}</p>}
          <div className="knowledge-actions">{savePreview&&<button type="button" className="subtle-button" onClick={()=>setSavePreview(null)}>취소</button>}
            <button className="primary-button" disabled={busy||!destination.trim()||!body.trim()}>{!savePreview?'저장 위치 확인':savePreview.exists?'덮어쓰기 확인':'로컬 파일 저장'}</button>
          </div>
        </form>
      </>}
    </div>}
  </Dialog>;
}
