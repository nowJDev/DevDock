// 기준별 과거 기록을 수동으로 조회하고 차트·표에서 같은 날짜의 근거를 연다.
import {useEffect,useMemo,useRef,useState} from 'react';
import type {ProjectProgress} from '../contracts';
import DailyProgressChart from '../components/charts/DailyProgressChart';
import Dialog from '../components/Dialog';
import type {KnowledgeApi} from './api';
import type {HistoryPoint,Json,ProjectHistory,ProjectProgressView} from './contracts';
import {historyChartPoints,rangeDays} from './historyChart';
const kstToday=()=>new Intl.DateTimeFormat('en-CA',{timeZone:'Asia/Seoul'}).format(new Date());
function rangeFor(days:number,end:string){const d=new Date(end+'T00:00:00Z');d.setUTCDate(d.getUTCDate()-days+1);return {from:d.toISOString().slice(0,10),through:end};}
const originName=(value:string)=>value==='observed'?'실제 수집':value==='correction'?'명시적 보정':'문서 복원';
interface Read {key:string;history:ProjectHistory|null;error:string;loading:boolean}
export default function KnowledgeHistory({api,project,progress,refreshVersion,onRefresh,today}:{api:KnowledgeApi;project:ProjectProgress;progress:ProjectProgressView;refreshVersion:number;onRefresh:()=>void;today?:string}){
 const initialBasis=progress.basis?.kind==='legacy'?progress.bases.find(b=>b.kind!=='legacy')??progress.basis:progress.basis;
 const initialDays=progress.mode==='legacy'&&initialBasis&&initialBasis.kind!=='legacy'?30:7;
 const [basis,setBasis]=useState(initialBasis?.basisId??'legacy'),[version,setVersion]=useState(initialBasis?.version??0),[period,setPeriod]=useState(String(initialDays)),[range,setRange]=useState(()=>rangeFor(initialDays,today??kstToday())),[draftRange,setDraftRange]=useState(()=>rangeFor(initialDays,today??kstToday())),[manual,setManual]=useState(0),[result,setResult]=useState<Read|null>(null),[error,setError]=useState(''),[day,setDay]=useState<{date:string;point?:HistoryPoint;opener:HTMLElement|null}|null>(null),[sourceText,setSourceText]=useState(''),[sourceError,setSourceError]=useState('');
 const refresh=useRef<(()=>void)|null>(null);
 const key=JSON.stringify([project.id,basis,range.from,range.through]);
 useEffect(()=>{
  let active=true,inFlight=false,pending=false;
  const read=()=>{if(inFlight){pending=true;return;}inFlight=true;setResult(old=>old?.key===key?{...old,loading:true,error:''}:{key,history:null,error:'',loading:true});void api.history(project.id,basis,range.from,range.through).then(history=>{if(active)setResult({key,history,error:'',loading:false});},e=>{if(active)setResult(old=>({key,history:old?.key===key?old.history:null,error:String(e),loading:false}));}).finally(()=>{inFlight=false;if(active&&pending){pending=false;read();}});};
  refresh.current=read;return()=>{active=false;refresh.current=null;};
 },[api,key]);
 useEffect(()=>{refresh.current?.();},[key,refreshVersion,manual]);
 const matching=result?.key===key?result:null,history=matching?.history;
 const selected=progress.bases.find(b=>b.basisId===basis),name=basis==='legacy'?'이전 집계 (기준 미기록)':selected?.name??'선택한 진행 기준';
 const points=useMemo(()=>history?historyChartPoints(history,version,name):[],[history,version,name]);
 const versions=useMemo(()=>[...new Set([version,...(history?.points.map(p=>p.basisVersion)??[]),...(selected?[selected.version]:[])])].sort((a,b)=>b-a),[history,selected,version]);
 const days=rangeDays(range.from,range.through)??0;
 function selectDate(date:string,override?:HistoryPoint){setSourceText('');setSourceError('');setDay({date,point:override??history?.points.find(p=>p.date===date&&p.basisVersion===version),opener:document.activeElement instanceof HTMLElement?document.activeElement:null});}
 function refreshNow(){if(period!=='custom'){const next=rangeFor(Number(period),today??kstToday());setRange(next);setDraftRange(next);}onRefresh();}
 const work=(history?.workEntries??[]).filter(v=>v&&typeof v==='object'&&!Array.isArray(v)&&(v.workDate===day?.date||v.effectiveDate===day?.date)) as Record<string,Json>[];
 return <div className="knowledge-history">
  <div className="history-filters">
   <label>진행 기준<select aria-label="차트 진행 기준" value={basis} onChange={e=>{const next=e.target.value;setBasis(next);setVersion(progress.bases.find(b=>b.basisId===next)?.version??0);}}><option value="legacy">이전 집계 (기준 미기록)</option>{progress.bases.map(b=><option key={b.basisId} value={b.basisId}>{b.name}</option>)}</select></label>
   <label>기준 버전<select aria-label="차트 기준 버전" value={version} onChange={e=>setVersion(Number(e.target.value))}>{versions.map(v=><option key={v} value={v}>{v===0?'기준 미기록':'v'+v}</option>)}</select></label>
   <label>기간<select aria-label="기준별 차트 기간" value={period} onChange={e=>{setPeriod(e.target.value);if(e.target.value!=='custom'){const next=rangeFor(Number(e.target.value),today??kstToday());setRange(next);setDraftRange(next);}}}><option value="7">최근 7일</option><option value="30">최근 30일</option><option value="custom">사용자 기간</option></select></label>
   <button type="button" className="subtle-button" aria-label="기준별 차트 새로고침" onClick={refreshNow}>새로고침</button>
  </div>
  {period==='custom'&&<form className="history-filters" onSubmit={e=>{e.preventDefault();if(!rangeDays(draftRange.from,draftRange.through)){setError('과거 조회 기간은 올바른 날짜로 최대 366일까지 선택해 주세요.');return;}setError('');setRange(draftRange);setManual(n=>n+1);}}><label>시작일<input aria-label="과거 조회 시작일" type="date" value={draftRange.from} onChange={e=>setDraftRange({...draftRange,from:e.target.value})}/></label><label>종료일<input aria-label="과거 조회 종료일" type="date" value={draftRange.through} onChange={e=>setDraftRange({...draftRange,through:e.target.value})}/></label><button type="submit" className="subtle-button">기간 조회</button><span className="small">한 번에 최대 366일</span></form>}
  <p className="small">{name} · {version===0?'집계 기준 미기록':'선택 버전 v'+version}{selected&&selected.version!==version?' · 현재 기준은 v'+selected.version:''} · 새로고침 시 갱신</p>
  {error&&<p className="alert error" role="alert">{error}</p>}{matching?.error&&<p className="alert error" role="alert">{matching.error}</p>}
  {matching?.loading&&<p className="small" role="status">기록을 조회하는 중입니다. 기존 차트를 유지합니다.</p>}
  {history&&days>0?<DailyProgressChart points={points} days={days} today={range.through} projectName={project.name} provenance onSelectDate={selectDate}/>:!matching?.error&&<p className="small" role="status">진행 이력을 불러오는 중입니다.</p>}
  {(history?.comparisons?.length??0)>(history?.points.length??0)&&<details className="source-path"><summary>원래 관측과 다른 기록 비교</summary>{history?.comparisons?.filter(p=>p.basisVersion===version).map((p,i)=><p key={i}><button type="button" className="text-button" onClick={()=>selectDate(p.date,p)}>{p.date} · {originName(p.origin)} · {p.counts?`${p.counts.verified+p.counts.reportedComplete}/${p.counts.total}`:'범위 불완전'}</button></p>)}</details>}
  {day&&<Dialog title={day.date+' 업무와 근거'} wide returnFocus={day.opener} onClose={()=>setDay(null)}><p>{day.point?`${originName(day.point.origin)} · 기준 ${day.point.basisVersion===0?'미기록':'v'+day.point.basisVersion}`:'해당 날짜에 수치 기록이 없습니다.'}</p>{day.point?.counts&&<p>전체 {day.point.counts.total} · 검증 {day.point.counts.verified} · 미검증 완료 보고 {day.point.counts.reportedComplete}</p>}{work.map((w,i)=><p key={i}>{String((w.proposedChanges as Record<string,Json>)?.text??'업무 기록')}</p>)}{!work.length&&<p className="small">해당 날짜에 승인한 업무 본문은 없습니다.</p>}{day.point?.sourceSpans.map((span,i)=><div className="source-path" key={i}><p>버전 {span.versionId} · SHA-256 {span.sha256}</p><button type="button" className="subtle-button" onClick={()=>{setSourceError('');void api.read(span.versionId,'lines' in span.locator?span.locator.lines.start:1,200).then(v=>setSourceText(v.text),e=>setSourceError(String(e)));}}>원문 근거 보기</button></div>)}{!day.point?.sourceSpans.length&&<p className="small">원문 구간 근거는 기록되지 않았습니다.</p>}{sourceError&&<p className="alert error" role="alert">{sourceError}</p>}{sourceText&&<><p className="small">참조한 불변 캐시 원문의 구간입니다.</p><pre className="document-reader" tabIndex={0}>{sourceText}</pre></>}</Dialog>}
 </div>;
}
