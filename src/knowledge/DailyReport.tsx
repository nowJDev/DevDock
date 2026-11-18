// 프로젝트 문서로 Codex에 붙여넣을 일일보고 요청문을 만든다.
import {useEffect,useId,useMemo,useRef,useState,type FormEvent} from 'react';
import {Copy} from 'lucide-react';
import Dialog from '../components/Dialog';
import type {Project} from '../contracts';
import type {KnowledgeApi} from './api';
import type {DocumentSource,DocumentVersion,ReportProfile} from './contracts';

type HandoverFolder = {
  sourceId:string;
  root:string;
  files:{name:string;date:string|null}[];
  error?:string;
};
type ReportFile = {path:string;name:string;root:string;date:string;handover:boolean};

const unknownDate='날짜 미상';
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
  const activeSources=new Map(sources.filter(s=>s.enabled&&!s.archived).map(s=>[s.sourceId,s]));
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

export default function DailyReport({api,project,returnFocus,onClose}:{
  api:KnowledgeApi;project:Pick<Project,'id'|'name'>;returnFocus:HTMLElement|null;onClose:()=>void;
}){
  const listId=useId();
  const preview=useRef<HTMLTextAreaElement>(null);
  const [files,setFiles]=useState<ReportFile[]|null>(null);
  const [heading,setHeading]=useState(project.name),[dateFilter,setDateFilter]=useState('');
  const [selected,setSelected]=useState<string[]>([]);
  const [prompt,setPrompt]=useState(''),[copiedPrompt,setCopiedPrompt]=useState('');
  const [error,setError]=useState(''),[copyError,setCopyError]=useState('');
  const [warnings,setWarnings]=useState<string[]>([]);

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
    void Promise.all([
      api.execute<ReportProfile>('get_report_profile',{projectId:project.id}),
      api.config(),
      loadDocuments(),
      api.execute<{folders:HandoverFolder[]}>('list_report_handover_files',{projectId:project.id}),
    ]).then(([profile,config,docs,{folders}])=>{
      if(!active)return;
      setHeading(profile.workHeading.trim()||project.name);
      setFiles(reportFiles(docs,config.sources,folders));
      setWarnings(folders.filter(folder=>folder.error).map(folder=>folder.root+' · '+folder.error));
    }).catch(e=>{if(active)setError(errorMessage(e));});
    return()=>{active=false;};
  },[api,project.id,project.name]);

  const dates=useMemo(()=>{
    const unique=[...new Set(files?.map(file=>file.date))];
    return unique.sort((a,b)=>a===unknownDate?1:b===unknownDate?-1:b.localeCompare(a));
  },[files]);
  const groups=useMemo(()=>{
    const available=files?.filter(file=>!dateFilter||file.date===dateFilter)??[];
    return [{title:'핸드오버',files:available.filter(file=>file.handover)},
      {title:'프로젝트 문서',files:available.filter(file=>!file.handover)}].map(group=>({
        title:group.title,
        dates:dates.map(date=>({date,files:group.files.filter(file=>file.date===date)}))
          .filter(bucket=>bucket.files.length>0),
      }));
  },[files,dateFilter,dates]);

  function clearPrompt(){
    setPrompt('');
    setCopiedPrompt('');
    setCopyError('');
  }
  function createPrompt(event:FormEvent){
    event.preventDefault();
    const chosen=files?.filter(file=>selected.includes(file.path))??[];
    if(!chosen.length||!heading.trim())return;
    const roots=[...new Set(chosen.map(file=>file.root))];
    setPrompt([
      '$daily-work-report',
      '파일 위치: '+roots.join(', '),
      '업무명: '+heading.trim(),
      ...chosen.map(file=>'파일명: '+file.path),
    ].join('\n'));
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
    {error&&<p className="alert error" role="alert">문서를 불러오지 못했습니다. {error}</p>}
    {!files&&!error&&<p role="status">보고에 사용할 문서를 불러오는 중입니다.</p>}
    <form className="knowledge-content report-dialog-body" onSubmit={createPrompt}>
      <div className="knowledge-fields">
        <label className="field">업무명<input required maxLength={1024} value={heading} disabled={!files} onChange={e=>{
          setHeading(e.target.value);clearPrompt();
        }}/></label>
        <label className="field">문서 날짜<select value={dateFilter} onChange={e=>setDateFilter(e.target.value)} disabled={!dates.length}>
          <option value="">전체 날짜</option>
          {dates.map(date=><option key={date} value={date}>{date}</option>)}
        </select></label>
      </div>
      {warnings.map(warning=><p className="warning-text source-path" key={warning}>{warning}</p>)}
      {files&&<fieldset className="document-roles report-document-picker">
        <legend id={listId}>보고에 사용할 문서</legend>
        {!groups.some(group=>group.dates.length>0)&&<p className="small">표시할 문서가 없습니다.</p>}
        {groups.some(group=>group.dates.length>0)&&<div className="report-document-list" role="region" aria-labelledby={listId} tabIndex={0}>
          {groups.filter(group=>group.dates.length>0).map(group=><section className="report-document-group" key={group.title}>
            <h3>{group.title}</h3>
            {group.dates.map(bucket=><div className="report-document-day" key={bucket.date}>
              <h4>{bucket.date}</h4>
              {bucket.files.map(file=><label className="candidate-select" key={file.path} title={file.path}>
                <input type="checkbox" checked={selected.includes(file.path)} onChange={e=>{
                  const checked=e.target.checked;
                  setSelected(previous=>checked?[...previous,file.path]:previous.filter(path=>path!==file.path));
                  clearPrompt();
                }}/><span>{file.name}</span>
              </label>)}
            </div>)}
          </section>)}
        </div>}
      </fieldset>}
      <div className="knowledge-actions report-request-actions">
        <span className="small">선택 {selected.length}개</span>
        <button className="primary-button" type="submit" disabled={!selected.length||!heading.trim()}>Codex 보고 요청</button>
      </div>
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
