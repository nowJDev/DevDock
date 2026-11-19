// 프로젝트 문서로 Codex에 붙여넣을 일일보고 요청문을 만든다.
import {useEffect,useId,useMemo,useRef,useState,type FormEvent} from 'react';
import {Copy} from 'lucide-react';
import Dialog from '../components/Dialog';
import type {Project} from '../contracts';
import type {KnowledgeApi} from './api';
import type {DocumentSource,DocumentVersion,ReportProfile} from './contracts';
import './dailyReport.css';

type HandoverFolder = {
  sourceId:string;
  root:string;
  files:{name:string;date:string|null}[];
  error?:string;
};
type ReportFile = {path:string;name:string;root:string;date:string;handover:boolean};
type ReportDocuments = {files:ReportFile[];hasSources:boolean;warnings:string[];retrySources:string[]};

const unknownDate='날짜 미상';
const sourceFailures:Record<string,string>={partial:'일부만 읽음',offline:'오프라인',denied:'접근 권한 없음',timeout:'연결 시간 초과',invalidRoot:'폴더 경로 확인 필요',error:'연결 확인 실패'};
const errorMessage=(error:unknown)=>String(error).replace(/^Error: /,'');
const toSlashes=(path:string)=>path.replace(/\\/g,'/');
const fullPath=(root:string,relative:string)=>toSlashes(root).replace(/\/+$/,'')+'/'+toSlashes(relative);
const localDate=new Intl.DateTimeFormat('sv-SE',{timeZone:'Asia/Seoul',year:'numeric',month:'2-digit',day:'2-digit'});

function documentDate(document:DocumentVersion) {
  if(document.authoredDate)return document.authoredDate;
  const name=toSlashes(document.path).split('/').at(-1)??'';
  for(const match of name.matchAll(/(?<!\d)(\d{4})[-_.]?(\d{2})[-_.]?(\d{2})(?!\d)/g)){
    const date=[match[1],match[2],match[3]].join('-');
    const time=new Date(date+'T00:00:00Z');
    if(!Number.isNaN(time.getTime())&&time.toISOString().startsWith(date))return date;
  }
  const captured=new Date(document.capturedAt);
  return Number.isNaN(captured.getTime())?unknownDate:localDate.format(captured);
}

function reportFiles(documents:DocumentVersion[],sources:DocumentSource[],folders:HandoverFolder[]) {
  const activeSources=new Map(sources.map(s=>[s.sourceId,s]));
  const liveFolders=new Set(folders.map(folder=>folder.sourceId));
  const files=new Map<string,ReportFile>();
  const add=(file:ReportFile)=>{
    const key=/^(?:[a-z]:|\/\/)/i.test(file.path)?file.path.toLowerCase():file.path;
    if(!files.has(key))files.set(key,file);
  };
  for(const folder of folders){
    for(const file of folder.files){
      add({path:fullPath(folder.root,file.name),name:file.name,root:folder.root,date:file.date??unknownDate,handover:true});
    }
  }
  for(const doc of documents){
    const source=activeSources.get(doc.sourceId);
    if(!source)continue;
    const name=toSlashes(doc.path);
    if(liveFolders.has(doc.sourceId)&&!name.includes('/'))continue;
    add({path:fullPath(source.root,name),name,root:source.root,date:documentDate(doc),
      handover:liveFolders.has(doc.sourceId)||doc.kind==='handover'});
  }
  return [...files.values()].sort((a,b)=>Number(b.handover)-Number(a.handover)||
    b.date.localeCompare(a.date,'en',{numeric:true})||a.name.localeCompare(b.name,'ko'));
}

function byDate(files:ReportFile[],dates:string[]){
  const grouped=new Map<string,ReportFile[]>();
  for(const file of files){
    const group=grouped.get(file.date)??[];
    group.push(file);
    grouped.set(file.date,group);
  }
  return dates.flatMap(date=>{
    const files=grouped.get(date);
    return files?[{date,files}]:[];
  });
}

function buildPrompt(files:ReportFile[],heading:string){
  return files.length&&heading.trim()?[
    '$daily-work-report',
    '파일 위치: '+[...new Set(files.map(file=>file.root))].join(', '),
    '업무명: '+heading.trim(),
    ...files.map(file=>'파일명: '+file.path),
  ].join('\n'):'';
}

export default function DailyReport({api,project,returnFocus,onClose,onSettings}:{
  api:KnowledgeApi;project:Pick<Project,'id'|'name'>;returnFocus:HTMLElement|null;onClose:()=>void;onSettings?:()=>void;
}){
  const listId=useId();
  const preview=useRef<HTMLTextAreaElement>(null);
  const dateInput=useRef<HTMLSelectElement>(null);
  const selectionToggle=useRef<HTMLButtonElement>(null);
  const headingEdited=useRef(false);
  const [documents,setDocuments]=useState<ReportDocuments|null>(null);
  const [heading,setHeading]=useState(project.name),[dateFilter,setDateFilter]=useState('');
  const [selected,setSelected]=useState<Set<string>>(()=>new Set());
  const [selectionOpen,setSelectionOpen]=useState(false);
  const [generatedPrompt,setGeneratedPrompt]=useState(''),[copiedPrompt,setCopiedPrompt]=useState('');
  const [promptChanged,setPromptChanged]=useState(false),[copyError,setCopyError]=useState('');
  const [loading,setLoading]=useState(true),[loadError,setLoadError]=useState('');
  const [reloadSources,setReloadSources]=useState<string[]>([]);

  useEffect(()=>{
    let active=true;
    async function loadDocuments(){
      const documents:DocumentVersion[]=[];
      for(let page=0;active;page++){
        const result=await api.documents(project.id,page);
        documents.push(...result.documents);
        if(result.documents.length<result.pageSize)break;
      }
      return documents;
    }
    async function load(){
      const warnings:string[]=[];
      if(reloadSources.length){
        try{await api.scan(reloadSources,true);}
        catch(error){warnings.push('폴더를 다시 확인하지 못했습니다. '+errorMessage(error));}
      }
      if(!active)return;
      const [profile,config,indexed,handover,states]=await Promise.allSettled([
        api.execute<ReportProfile>('get_report_profile',{projectId:project.id}),
        api.config(),
        loadDocuments(),
        api.execute<{folders:HandoverFolder[]}>('list_report_handover_files',{projectId:project.id}),
        api.sourceStates(),
      ]);
      if(!active)return;
      if(config.status==='rejected')throw config.reason;
      if(profile.status==='fulfilled'){
        if(!headingEdited.current)setHeading(profile.value.workHeading.trim()||project.name);
      }else warnings.push('일일보고 업무명을 불러오지 못했습니다. '+errorMessage(profile.reason));
      const sources=config.value.sources.filter(source=>source.enabled&&!source.archived&&
        (source.scope==='global'||source.projectId===project.id));
      const sourceIds=new Set(sources.map(source=>source.sourceId));
      const folders=handover.status==='fulfilled'?handover.value.folders.filter(folder=>sourceIds.has(folder.sourceId)):[];
      if(handover.status==='rejected')warnings.push('핸드오버 목록을 읽지 못했습니다. '+errorMessage(handover.reason));
      if(indexed.status==='rejected')warnings.push('프로젝트 문서 목록을 읽지 못했습니다. '+errorMessage(indexed.reason));

      const liveFolders=new Set(folders.map(folder=>folder.sourceId));
      const unavailable=new Set<string>();
      for(const folder of folders){
        if(!folder.error)continue;
        unavailable.add(folder.sourceId);
        warnings.push(folder.root+' · '+folder.error);
      }
      const retrySources:string[]=[];
      if(states.status==='fulfilled'){
        for(const state of states.value.sources){
          const source=sources.find(candidate=>candidate.sourceId===state.sourceId);
          const failure=sourceFailures[state.state];
          if(!source||!failure||liveFolders.has(state.sourceId))continue;
          if(state.state!=='partial')unavailable.add(source.sourceId);
          warnings.push([source.root,failure,...(state.errors??[]).filter(Boolean)].join(' · '));
          retrySources.push(source.sourceId);
        }
      }else warnings.push('문서 폴더 연결 상태를 확인하지 못했습니다. '+errorMessage(states.reason));
      const files=reportFiles(
        indexed.status==='fulfilled'?indexed.value:[],
        sources.filter(source=>!unavailable.has(source.sourceId)),
        folders.filter(folder=>!folder.error),
      );
      const paths=new Set(files.map(file=>file.path));
      setDocuments({files,hasSources:sources.length>0,warnings,retrySources});
      setSelected(previous=>new Set([...previous].filter(path=>paths.has(path))));
    }
    void load().catch(error=>{if(active)setLoadError(errorMessage(error));})
      .finally(()=>{if(active)setLoading(false);});
    return()=>{active=false;};
  },[api,project.id,project.name,reloadSources]);

  const files=documents?.files;
  const dates=useMemo(()=>{
    const unique=[...new Set(files?.map(file=>file.date))];
    return unique.sort((a,b)=>a===unknownDate?1:b===unknownDate?-1:b.localeCompare(a));
  },[files]);
  const visibleFiles=useMemo(()=>files?.filter(file=>!dateFilter||file.date===dateFilter)??[],[files,dateFilter]);
  const groups=useMemo(()=>[
    {title:'핸드오버',dates:byDate(visibleFiles.filter(file=>file.handover),dates)},
    {title:'프로젝트 문서',dates:byDate(visibleFiles.filter(file=>!file.handover),dates)},
  ],[visibleFiles,dates]);
  const chosen=useMemo(()=>files?.filter(file=>selected.has(file.path))??[],[files,selected]);
  const selectedByDate=useMemo(()=>byDate(chosen,dates),[chosen,dates]);
  const visibleSelected=visibleFiles.filter(file=>selected.has(file.path)).length;
  const allVisibleSelected=visibleFiles.length>0&&visibleSelected===visibleFiles.length;
  const requestText=useMemo(()=>buildPrompt(chosen,heading),[chosen,heading]);
  const prompt=generatedPrompt===requestText?generatedPrompt:'';
  const warnings=documents?.warnings??[];

  function toggleVisibleSelection(){
    updateSelection(visibleFiles.map(file=>file.path),!allVisibleSelected);
  }

  function updateSelection(paths:string[],checked:boolean){
    setSelected(previous=>{
      const next=new Set(previous);
      for(const path of paths){
        if(checked)next.add(path);
        else next.delete(path);
      }
      return next;
    });
    invalidatePrompt();
  }

  function invalidatePrompt(){
    if(generatedPrompt)setPromptChanged(true);
    setGeneratedPrompt('');
    setCopiedPrompt('');
    setCopyError('');
  }

  function clearSelection(){
    setSelected(new Set());
    setSelectionOpen(false);
    invalidatePrompt();
    dateInput.current?.focus();
  }

  function removeSelection(path:string){
    updateSelection([path],false);
    if(chosen.length===1){
      setSelectionOpen(false);
      dateInput.current?.focus();
    }else selectionToggle.current?.focus();
  }

  function retry(){
    setLoadError('');
    setLoading(true);
    setReloadSources([...(documents?.retrySources??[])]);
  }

  function createPrompt(event:FormEvent){
    event.preventDefault();
    if(!requestText)return;
    setGeneratedPrompt(requestText);
    setPromptChanged(false);
    setCopiedPrompt('');
    setCopyError('');
  }
  useEffect(()=>{if(prompt)preview.current?.focus();},[prompt]);

  async function copy(){
    if(!prompt)return;
    setCopyError('');
    try{
      await navigator.clipboard.writeText(prompt);
    }catch{
      preview.current?.focus();
      preview.current?.select();
      if(!document.execCommand?.('copy')){
        setCopyError('자동 복사에 실패했습니다. 선택된 요청문을 Ctrl+C로 복사해 주세요.');
        return;
      }
    }
    setCopiedPrompt(prompt);
  }

  return <Dialog title={'일일보고 · '+project.name} wide className="report-dialog" returnFocus={returnFocus} onClose={onClose}>
    <form className="knowledge-content report-dialog-body" onSubmit={createPrompt}>
      <div className="knowledge-fields">
        <label className="field">업무명<input required maxLength={1024} value={heading} disabled={!files} onChange={e=>{
          headingEdited.current=true;
          setHeading(e.target.value);
          invalidatePrompt();
        }}/></label>
        <label className="field">문서 날짜<select ref={dateInput} value={dateFilter} onChange={e=>setDateFilter(e.target.value)} disabled={!dates.length&&!dateFilter}>
          <option value="">전체 날짜</option>
          {dateFilter&&!dates.includes(dateFilter)&&<option value={dateFilter}>{dateFilter}</option>}
          {dates.map(date=><option key={date} value={date}>{date}</option>)}
        </select></label>
      </div>
      {loading&&<p className="small" role="status">보고에 사용할 문서를 불러오는 중입니다.</p>}
      {(loadError||warnings.length>0)&&<div className="report-source-problems">
        <div className="alert error" role="alert">
          <p>{loadError?'문서를 불러오지 못했습니다. '+loadError:'일부 문서 자료를 확인하지 못했습니다.'}</p>
          {warnings.length>0&&<ul>{[...new Set(warnings)].map(warning=><li key={warning}>{warning}</li>)}</ul>}
        </div>
        <button className="subtle-button" type="button" onClick={retry} disabled={loading}>다시 불러오기</button>
      </div>}
      {documents&&<fieldset className="document-roles report-document-picker" aria-busy={loading}>
        <legend id={listId}>보고에 사용할 문서</legend>
        {!loading&&!loadError&&!documents.hasSources&&<div className="report-empty">
          <p className="small">이 프로젝트에 연결된 문서 폴더가 없습니다.</p>
          {onSettings&&<button className="subtle-button" type="button" onClick={onSettings}>문서 연결 설정</button>}
        </div>}
        {!loading&&documents.hasSources&&documents.files.length===0&&!warnings.length&&!loadError&&
          <p className="small">연결된 폴더에 보고에 사용할 문서가 없습니다.</p>}
        {!loading&&documents.files.length>0&&visibleFiles.length===0&&<div className="report-empty">
          <p className="small">선택한 날짜에 문서가 없습니다.</p>
          <button className="subtle-button" type="button" onClick={()=>{setDateFilter('');dateInput.current?.focus();}}>전체 날짜로 보기</button>
        </div>}
        {visibleFiles.length>0&&<>
          <div className="knowledge-actions report-list-actions">
            <button className="subtle-button" type="button" aria-controls={listId+'-files'} onClick={toggleVisibleSelection}>
              {allVisibleSelected?'현재 목록 선택 해제':'현재 목록 전체 선택'}
            </button>
          </div>
          <div id={listId+'-files'} className="report-document-list" role="region" aria-labelledby={listId} tabIndex={0}>
            {groups.filter(group=>group.dates.length>0).map(group=><section className="report-document-group" key={group.title}>
              <h3>{group.title}</h3>
              {group.dates.map(bucket=><div className="report-document-day" key={bucket.date}>
                <h4>{bucket.date}</h4>
                {bucket.files.map(file=><label className="candidate-select" key={file.path} title={file.path}>
                  <input type="checkbox" checked={selected.has(file.path)} onChange={e=>updateSelection([file.path],e.target.checked)}/>
                  <span>{file.name}</span>
                </label>)}
              </div>)}
            </section>)}
          </div>
        </>}
      </fieldset>}
      <section className="report-selection">
        <div className="knowledge-actions report-request-actions">
          <span className="small" role="status">{`선택한 파일 ${chosen.length}개 · 현재 목록 ${visibleSelected}개 · 다른 날짜 ${chosen.length-visibleSelected}개`}</span>
          {chosen.length>0&&<>
            <button ref={selectionToggle} className="subtle-button" type="button" aria-expanded={selectionOpen}
              aria-controls={listId+'-selection'} onClick={()=>setSelectionOpen(!selectionOpen)}>
              {selectionOpen?'선택한 파일 닫기':'선택한 파일 보기'}
            </button>
            <button className="text-button" type="button" onClick={clearSelection}>선택 모두 해제</button>
          </>}
          <button className="primary-button" type="submit" disabled={!requestText}>Codex 보고 요청</button>
        </div>
        {chosen.length>0&&<div className="report-selected-files" id={listId+'-selection'} role="region" aria-label="선택한 파일" hidden={!selectionOpen}>
          {selectedByDate.map(bucket=><div className="report-selected-day" key={bucket.date}>
            <h3>{bucket.date}</h3>
            <ul>{bucket.files.map(file=><li key={file.path}>
              <span title={file.path}>{file.name}</span>
              <button className="text-button" type="button" aria-label={file.name+' 선택 해제'} onClick={()=>removeSelection(file.path)}>해제</button>
            </li>)}</ul>
          </div>)}
        </div>}
        {!prompt&&(promptChanged||generatedPrompt)&&<p className="small" role="status">선택이나 업무명이 변경되었습니다. 요청문을 다시 생성해 주세요.</p>}
      </section>
      {prompt&&<section className="report-prompt">
        <label className="field">Codex 요청문<textarea ref={preview} readOnly rows={Math.min(prompt.split('\n').length,7)} value={prompt}/></label>
        <div className="knowledge-actions">
          {copiedPrompt===prompt&&<span className="small" role="status">요청문을 복사했습니다.</span>}
          <button className="subtle-button" type="button" onClick={()=>void copy()}><Copy size={15} aria-hidden="true"/>복사</button>
        </div>
        {copyError&&<p className="alert error" role="alert">{copyError}</p>}
      </section>}
    </form>
  </Dialog>;
}
