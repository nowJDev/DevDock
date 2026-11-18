// 데몬·프로젝트·차트·근태를 구분한 모던 다크 대시보드를 제공합니다.
import {useCallback,useEffect,useMemo,useRef,useState,type FormEvent} from 'react';
import {Activity,ArrowDownRight,ArrowUpRight,Check,CheckCheck,ChevronRight,Clock3,Code2,Database,FolderKanban,LayoutDashboard,Loader2,MonitorDot,Plus,RefreshCw,Search,Settings2,ShieldCheck,Trash2,X} from 'lucide-react';
import type {Api,AttendanceRun,Daemon,Monitor,Project,ProjectProgress,Registry,Settings,Snapshot,History} from './contracts';
import {api as defaultApi} from './api';
import DailyProgressChart from './components/charts/DailyProgressChart';
import ProgressChart from './components/charts/ProgressChart';
import DocumentSettings from './knowledge/DocumentSettings';
import ProjectKnowledge from './knowledge/ProjectKnowledge';
import {ProjectActions,ProjectActionDialog,type ProjectAction} from './knowledge/ProjectActions';
import KnowledgeHistory from './knowledge/KnowledgeHistory';
import './styles/project-actions.css';
import Modal from './components/Dialog';
import {knowledgeApi as defaultKnowledge,type KnowledgeApi} from './knowledge/api';
type Page='overview'|'projects'|'monitors'|'activity'|'settings';
const pages:{id:Page;label:string;icon:typeof Activity}[]=[{id:'overview',label:'개요',icon:LayoutDashboard},{id:'projects',label:'프로젝트',icon:FolderKanban},{id:'monitors',label:'모니터링',icon:MonitorDot},{id:'activity',label:'활동 이력',icon:Activity},{id:'settings',label:'설정',icon:Settings2}];
const transport:Record<string,string>={healthy:'응답 정상',slow:'응답 지연',disconnected:'연결 실패',unknown:'확인 불가'};
const activity:Record<string,string>={working:'요청 처리 중',toolRunning:'도구 실행 중',approval:'승인 대기',input:'입력 대기',idle:'대기 중',failed:'실패',unknown:'현재 활동 확인 불가'};
const taskLabels:Record<string,string>={pending:'미착수',inProgress:'진행 중',blocked:'막힘',reportedComplete:'완료 보고',verified:'검증 완료',cancelled:'취소'};
function time(value:string|null|undefined){if(!value)return '아직 확인하지 않음';const d=new Date(value);return Number.isNaN(+d)?value:d.toLocaleTimeString('ko-KR',{hour:'2-digit',minute:'2-digit',second:'2-digit',hour12:false});}
function localDate(value:string){
 const d=new Date(value);
 return [d.getFullYear(),String(d.getMonth()+1).padStart(2,'0'),String(d.getDate()).padStart(2,'0')].join('-');
}
function capturedAt(value:string){
 if(/^\d{4}-\d{2}-\d{2}$/.test(value))return value;
 const d=new Date(value);
 return Number.isNaN(+d)?value:d.toLocaleString('ko-KR',{year:'numeric',month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit',hour12:false});
}
const errorMessage=(error:unknown)=>String(error).replace(/^Error: /,'');

type HistoryResult={projectId:string;hours:number}&({state:'ready';history:History}|{state:'error';message:string});
function useProjectHistory(api:Api,projectId:string|undefined,hours:number,refreshVersion:number){
 const [result,setResult]=useState<HistoryResult|null>(null);
 const [attempt,retry]=useState(0);
 const refresh=useRef<(()=>void)|null>(null);
 useEffect(()=>{
  if(!projectId){setResult(null);return;}
  let current=true;
  let inFlight=false;
  let refreshPending=false;
  const read=()=>{
   if(inFlight){refreshPending=true;return;}
   inFlight=true;
   void api.history(projectId,hours).then(
    history=>{if(current)setResult({projectId,hours,state:'ready',history});},
    error=>{if(current)setResult({projectId,hours,state:'error',message:errorMessage(error)});},
   ).finally(()=>{
    inFlight=false;
    if(current&&refreshPending){refreshPending=false;read();}
   });
  };
  refresh.current=read;
  return()=>{current=false;refresh.current=null;};
 },[api,projectId,hours,attempt]);
 useEffect(()=>{refresh.current?.();},[api,projectId,hours,attempt,refreshVersion]);
 const matchingResult=result?.projectId===projectId&&result?.hours===hours?result:null;
 return {result:matchingResult,retry:()=>{setResult(null);retry(n=>n+1);}};
}
function HistoryError({message,onRetry}:{message:string;onRetry:()=>void}){
 return <div className="history-error"><p role="alert">진행 이력을 불러오지 못했습니다. {message}</p><button className="subtle-button" onClick={onRetry}>다시 조회</button></div>;
}
function DaemonCard({daemon}:{daemon:Daemon}){
 const fresh=!daemon.freshness||daemon.freshness==='fresh';
 const currentTransport=fresh?daemon.transportState:'unknown';
 const currentActivity=fresh?daemon.activityState:'unknown';
 const good=currentTransport==='healthy';
 return <article className="daemon-card">
  <div className="daemon-top"><div className={'daemon-symbol '+(daemon.id==='B'?'teal':'')}><Code2 size={18}/></div><h3>{daemon.name||daemon.id+' daemon'}</h3><span className={'status-pill '+(good?'good':currentTransport==='slow'?'warn':'neutral')}><i/>{transport[currentTransport]??'확인 불가'}</span></div>
  <div className="daemon-main"><span className="activity-label">{activity[currentActivity]??'현재 활동 확인 불가'}</span>{fresh&&daemon.latencyMs!==null&&<span className="latency">{Math.round(daemon.latencyMs)}<small> ms</small></span>}</div>
  {daemon.freshness==='stale'&&<p className="daemon-reason">오래된 관측 · 마지막 관측: {transport[daemon.lastKnown?.transportState??'']??'확인 불가'} · {activity[daemon.lastKnown?.activityState??'']??'확인 불가'}</p>}
  {daemon.reason&&<p className="daemon-reason">{daemon.reason}</p>}
  <footer><span><Clock3 size={13}/> {time(daemon.checkedAt)}</span></footer>
 </article>;
}
function ProjectSource({project,summary=false,current=false}:{project:ProjectProgress;summary?:boolean;current?:boolean}){
 const prefix=current?'현재 ':'';
 return <span className="project-source">
  {project.trackingBasis?<span><b>{prefix}집계 기준</b> <span>{project.trackingBasis}</span></span>:<span>{prefix}집계 기준 미연결</span>}
  {project.sourceCapturedAt&&<span><b>{prefix}자료 기준</b> <time dateTime={project.sourceCapturedAt}>{capturedAt(project.sourceCapturedAt)}</time></span>}
  {project.verified===0&&<span>검증 근거 미연결</span>}
  {Boolean(project.sourceErrors?.length)&&(summary
   ?<span className="warning-text">소스 오류 {project.sourceErrors?.length}건</span>
   :<span className="warning-text">{project.sourceErrors?.map((error,i)=><span key={i}>소스 오류: {error}</span>)}</span>)}
 </span>;
}
function ProgressRows({projects,onSelect,onAction}:{projects:ProjectProgress[];onSelect:(p:ProjectProgress)=>void;onAction:(value:ProjectAction)=>void}){
 if(!projects.length)return <div className="empty-project"><FolderKanban size={30}/><h3>첫 프로젝트를 연결하세요.</h3><p>체크리스트와 검증 근거를 연결하면 진행상황을 확인할 수 있습니다.</p></div>;
 return <div className="project-rows">{projects.map(p=><article className="project-list-item" key={p.id}><button className="project-row" type="button" aria-label={p.name+' 상세 열기'} onClick={()=>onSelect(p)}>
  <span className="project-icon"><FolderKanban size={19}/></span>
  <span className="project-copy"><strong>{p.name}</strong><span>{p.currentTask||'현재 작업 정보가 없습니다.'}</span><ProjectSource project={p} summary/></span>
  <span className="project-progress"><span className="progress-track"><i style={{width:(p.total?p.verified/p.total*100:0)+'%'}}/><b style={{width:(p.total?p.reportedComplete/p.total*100:0)+'%'}}/></span><span className="progress-caption"><span>검증 완료 {p.verified}</span><span>완료 보고 {p.reportedComplete}</span><span>전체 {p.total}개</span></span></span>
  <span className="project-row-end">{p.blocked>0&&<span className="warning-count">막힘 {p.blocked}</span>}<ChevronRight size={17}/></span>
 </button><ProjectActions project={p} onAction={onAction}/></article>)}</div>;
}

const attendanceActions:Record<string,string>={checkIn:'출근',checkOut:'퇴근'};
function attendanceResult(run:AttendanceRun,action:string){
 switch(run.state){
  case 'alreadyIn':return '이미 출근';
  case 'alreadyOut':return '이미 퇴근';
  case 'recorded':return action+' 기록 완료';
  case 'uncertain':return action+' 결과 확인 필요';
  case 'failed':return action+' 실행 실패';
  case 'refused':return action+' 실행 거절';
  case 'pending':return action+' 실행 중';
  default:return action+' 결과 확인 불가';
 }
}
function AttendancePanel({attendance,settings,onSettings}:{attendance:Snapshot['attendance'];settings:Settings;onSettings:()=>void}){
 const today=attendance.date;
 return <section className="panel attendance-panel" aria-labelledby="attendance-title">
  <div className="section-heading"><div><h2 id="attendance-title">오늘의 근태</h2><p>{settings.attendanceEnabled?`${settings.startTime} 이상 ${settings.endTime} 미만 자동 출근`:'자동 출근·퇴근 알림 꺼짐'}</p></div><button className="subtle-button" onClick={onSettings}>설정 <ChevronRight size={14}/></button></div>
  <div className="attendance-times">{(['checkIn','checkOut'] as const).map(key=>{
   const action=attendanceActions[key], latest=attendance.lastRunsByAction?.[key];
   const run=latest?.requestedDate===today?latest:attendance.runs.find(r=>r.action===key&&r.requestedDate===today);
   const recorded=key==='checkIn'?attendance.checkInTime:attendance.checkOutTime;
   return <div className="attendance-time" key={key}><b>{action}</b><span>{recorded??'—'}</span><small>{run?attendanceResult(run,action):recorded?action+' 기록 완료':'기록 확인 전'}</small></div>;
  })}</div>
  <p className="attendance-message">서버 상태: 확인 불가</p>
  {attendance.runs.length>0&&<details className="attendance-log"><summary>근태 실행 이력</summary>{attendance.runs.map(run=><p key={run.attemptId}>{run.requestedDate} · {attendanceResult(run,attendanceActions[run.action]??'근태')} · {run.state} · {run.code} · {run.message} · {run.recordDate} {run.time} · {run.result?JSON.stringify(run.result):'결과 없음'}</p>)}</details>}
 </section>;
}

const monitorKinds:Record<string,string>={checklist:'체크리스트',structuredStatus:'구조화 상태',daemon:'데몬'};
const monitorStates:Record<string,{label:string;tone:string}>={
 healthy:{label:'관측 정상',tone:'good'},slow:{label:'응답 지연',tone:'warn'},disconnected:{label:'연결 실패',tone:'warn'},
 failed:{label:'관측 실패',tone:'warn'},warning:{label:'주의 필요',tone:'warn'},disabled:{label:'관측 중지',tone:'neutral'},
 unknown:{label:'확인 불가',tone:'neutral'},
};
export default function App({api=defaultApi,knowledge=defaultKnowledge}:{api?:Api;knowledge?:KnowledgeApi}){
 const [page,setPage]=useState<Page>('overview');const [snapshot,setSnapshot]=useState<Snapshot|null>(null);const [registry,setRegistry]=useState<Registry|null>(null);const [error,setError]=useState('');const [loadError,setLoadError]=useState('');const [subscriptionError,setSubscriptionError]=useState('');const [notice,setNotice]=useState('');const [busy,setBusy]=useState(false);const [filter,setFilter]=useState('');const [selected,setSelected]=useState<ProjectProgress|null>(null);const [form,setForm]=useState<'project'|'monitor'|null>(null);const [remove,setRemove]=useState<{entity:'project'|'monitor';item:Project|Monitor}|null>(null);const [editing,setEditing]=useState<Project|Monitor|null>(null);const [checkout,setCheckout]=useState<{date:string;returnFocus:HTMLElement|null}|null>(null);const [baseRevision,setBaseRevision]=useState<number|undefined>();const [monitorKind,setMonitorKind]=useState('checklist');const settingsDirty=useRef(false);const [settings,setSettings]=useState<Settings|null>(null);const declineRef=useRef<HTMLButtonElement>(null);
 const [documentsOpen,setDocumentsOpen]=useState(false);
 const [projectAction,setProjectAction]=useState<ProjectAction|null>(null);
 const [chartRefresh,setChartRefresh]=useState(0);
 const [historyMode,setHistoryMode]=useState<'auto'|'legacy'|'knowledge'>('auto');
 const [days,setDays]=useState<7|30>(7);
 const [dailyProjectId,setDailyProjectId]=useState<string>();
 const [snapshotVersion,setSnapshotVersion]=useState(0);
 const mutationTrigger=useRef<HTMLElement>(null);
 const loadGeneration=useRef(0);
 const dashboardLoad=useRef<{result:Promise<void>;next?:Promise<void>}|null>(null);
 const load=useCallback(function load(refreshAfterPending=false):Promise<void>{
  const generation=loadGeneration.current;
  const pending=dashboardLoad.current;
  if(pending){
   if(!refreshAfterPending)return pending.result;
   pending.next??=pending.result.then(()=>{
    if(generation===loadGeneration.current)return load();
   });
   return pending.next;
  }
  const result=Promise.resolve().then(async()=>{
   if(generation!==loadGeneration.current)return;
   try{
    const [nextSnapshot,nextRegistry]=await Promise.all([api.snapshot(),api.registry()]);
    if(generation!==loadGeneration.current)return;
    const active=nextSnapshot.projects.filter(p=>!p.archived);
    setSnapshot(nextSnapshot);
    setSnapshotVersion(n=>n+1);
    setDailyProjectId(id=>active.some(p=>p.id===id)?id:active[0]?.id);
    setSelected(prev=>prev?(active.find(p=>p.id===prev.id)??null):null);
    setRegistry(nextRegistry);
    setLoadError('');
    setSettings(prev=>settingsDirty.current?prev:nextSnapshot.settings);
   }catch(error){if(generation===loadGeneration.current)setLoadError(errorMessage(error));}
   finally{if(generation===loadGeneration.current)dashboardLoad.current=null;}
  });
  dashboardLoad.current={result};
  return result;
 },[api]);
 useEffect(()=>{let active=true;void load();const timer=setInterval(()=>void load(),5000);const cleanup:(()=>void)[]=[];
  const register=(name:string,callback:(value:unknown)=>void)=>api.subscribe(name,callback).then(fn=>{if(active)cleanup.push(fn);else fn();});
  void Promise.all([register('dashboard-updated',()=>{if(active)void load(true);}),register('open-settings',()=>setPage('settings')),register('checkout-prompt',value=>{
   if(value&&typeof value==='object'&&'date' in value&&typeof value.date==='string'){
    const date=value.date;
    const returnFocus=document.activeElement instanceof HTMLElement?document.activeElement:null;
    setCheckout(prev=>({date,returnFocus:prev?prev.returnFocus:returnFocus}));
   }
  })]).then(async()=>{if(active)await api.ready?.();}).catch(e=>{if(active){active=false;cleanup.splice(0).forEach(fn=>fn());setSubscriptionError('이벤트 연결에 실패했습니다: '+errorMessage(e));}});
  return()=>{active=false;loadGeneration.current++;dashboardLoad.current=null;clearInterval(timer);cleanup.splice(0).forEach(fn=>fn());};
 },[api,load]);
 useEffect(()=>{if(checkout)declineRef.current?.focus();},[checkout]);
 const advancedChart=historyMode==='knowledge'||historyMode==='auto'&&snapshot?.projects.find(p=>p.id===dailyProjectId)?.progressMode==='managed';
 const dailyHistory=useProjectHistory(api,page==='overview'&&!advancedChart?dailyProjectId:undefined,days*24,chartRefresh);
 const detailHistory=useProjectHistory(api,page==='projects'?selected?.id:undefined,24,chartRefresh);
 async function perform(action:()=>Promise<unknown>,message?:string){setBusy(true);setError('');try{await action();if(message)setNotice(message);await load(true);return true;}catch(e){setError(String(e).replace(/^Error: /,''));return false;}finally{setBusy(false);}}
 async function answer(answer:boolean){if(!checkout)return;const date=checkout.date;await perform(async()=>{const result=await api.answerCheckout(answer,date);setCheckout(null);if(result.attendanceError){setNotice('');setError('퇴근 실행 실패: '+result.attendanceError);return;}setNotice(result.expired?`${date} 퇴근 확인이 만료되어 실행되지 않았습니다.`:answer?'퇴근 실행 결과를 확인해 주세요.':`${date} 퇴근 알림을 거절했습니다.`);});}
 const projects=useMemo(()=>snapshot?.projects.filter(p=>!p.archived&&p.name.toLocaleLowerCase().includes(filter.toLocaleLowerCase()))??[],[snapshot,filter]);
 const activeProjects=snapshot?.projects.filter(p=>!p.archived)??[];const monitors=registry?.monitors.filter(m=>!m.archived)??[];
 const dailyProject=activeProjects.find(p=>p.id===dailyProjectId);
 // 일별 차트의 날짜 축도 수동 조회와 선택 변경 시점에 고정합니다.
 const chartToday=useMemo(()=>localDate(snapshot?.now??new Date().toISOString()),[dailyProjectId,days,chartRefresh]);
 const dailyData=dailyHistory.result?.state==='ready'?dailyHistory.result.history.dailyProgress:undefined;
 const dailyPoints=useMemo(()=>dailyData?.filter(p=>p.projectId===dailyProjectId)??[],[dailyData,dailyProjectId]);
 const projectHistory=detailHistory.result?.state==='ready'?detailHistory.result.history.progress:undefined;
 const title=pages.find(p=>p.id===page)?.label??'개요';
 const progressHistory=useMemo(()=>projectHistory?.filter(p=>p.projectId===selected?.id).sort((a,b)=>Date.parse(a.at)-Date.parse(b.at))??[],[projectHistory,selected?.id]);
 async function submitForm(e:FormEvent<HTMLFormElement>){e.preventDefault();const data=new FormData(e.currentTarget);const id=editing?.id??crypto.randomUUID();const project=form==='project';const value=project?{...editing,id,name:String(data.get('name')),root:String(data.get('root')),checklistPath:String(data.get('checklistPath')||'')}:{...editing,id,name:String(data.get('name')),kind:String(data.get('kind')),source:String(data.get('source')),projectId:String(data.get('projectId')||''),enabled:editing&&'enabled' in editing?editing.enabled:true};const ok=await perform(()=>api.mutate({entity:project?'project':'monitor',action:'upsert',value},baseRevision),'등록했습니다.');if(ok){setForm(null);setEditing(null);}}
 async function save(e:FormEvent){e.preventDefault();if(!settings)return;const valid=/^([01]\d|2[0-3]):[0-5]\d$/;if(!valid.test(settings.startTime)||!valid.test(settings.endTime)||!valid.test(settings.checkoutTime)||settings.startTime>=settings.endTime){setError('올바른 시간과 출근 시간대의 시작·종료 순서를 확인해 주세요.');return;}await perform(async()=>{const result=await api.saveSettings(settings);settingsDirty.current=false;return result;},'설정을 저장했습니다.');}
 return <div className="app-shell"><aside className="sidebar"><div className="brand"><div className="brand-mark"><Code2 size={23}/></div><span>DevDock</span></div><nav aria-label="주요 탐색">{pages.map(p=><button key={p.id} aria-current={page===p.id?'page':undefined} className={page===p.id?'selected':''} onClick={()=>{setPage(p.id);setSelected(null);setFilter('');}}><p.icon size={18}/><span>{p.label}</span>{p.id==='projects'&&<small>{activeProjects.length}</small>}</button>)}</nav><div className="sidebar-bottom"><div className="local-tag"><span className="local-dot"/>로컬 워크스페이스</div><div className="version">DevDock <span>v0.1.0</span></div></div></aside>
 <div className="main-shell"><header className="topbar"><div className="breadcrumb">워크스페이스 <ChevronRight size={13}/><strong>{title}</strong></div><div className="topbar-actions"><span className="sync"><i/>{snapshot?'화면 갱신 '+time(snapshot.now):'연결 확인 중'}</span><button className="icon-button" aria-label="지금 확인" disabled={busy} onClick={()=>void perform(async()=>{await api.refresh();setChartRefresh(n=>n+1);})}><RefreshCw size={16} className={busy?'spin':''}/></button><div className="profile">D</div></div></header>
 <main><div className="page-heading"><div><div className="eyebrow">{new Date(snapshot?.now??Date.now()).toLocaleDateString('ko-KR',{year:'numeric',month:'long',day:'numeric',weekday:'long'})}</div><h1>{page==='overview'?'워크스페이스 개요':title}</h1><p>{page==='overview'?'프로젝트의 흐름과 지금 필요한 확인을 한곳에서 살펴보세요.':page==='projects'?'현재 작업과 검증 근거를 구분해 진행상황을 확인합니다.':page==='monitors'?'프로젝트에 필요한 관측 항목을 연결하고 관리합니다.':page==='activity'?'실제로 관측한 작업과 상태 변경을 시간순으로 확인합니다.':'자동화 시간과 로컬 연결을 관리합니다.'}</p></div>{(page==='overview'||page==='projects'||page==='monitors')&&<button className="primary-button" onClick={e=>{mutationTrigger.current=e.currentTarget;setBaseRevision(registry?.revision);setEditing(null);setMonitorKind('checklist');setForm(page==='monitors'?'monitor':'project');}}><Plus size={16}/>{page==='monitors'?'항목 추가':'프로젝트 연결'}</button>}</div>
 {snapshot?.verificationMode&&<div className="alert notice" role="status">검증 모드입니다. 실제 근태와 자동 시작 설정을 변경하지 않습니다.</div>}
 {subscriptionError&&<div className="alert error" role="alert">{subscriptionError}</div>}
 {loadError&&<div className="alert error" role="alert"><ShieldCheck size={17}/><span>{loadError}</span><button className="icon-button" aria-label="조회 오류 닫기" onClick={()=>setLoadError('')}><X size={15}/></button></div>}
 {error&&<div className="alert error" role="alert"><ShieldCheck size={17}/><span>{error}</span><button className="icon-button" aria-label="오류 닫기" onClick={()=>setError('')}><X size={15}/></button></div>}
 {notice&&<div className="alert notice" role="status"><Check size={16}/><span>{notice}</span><button className="icon-button" aria-label="알림 닫기" onClick={()=>setNotice('')}><X size={15}/></button></div>}
 {!snapshot&&!loadError&&<div className="loading"><Loader2 className="spin"/>워크스페이스를 불러오는 중입니다.</div>}
 {page==='overview'&&snapshot&&<>
  <div className="metrics" aria-label="프로젝트 요약">
   <div><span><FolderKanban size={15}/>연결 프로젝트</span><strong>{activeProjects.length}<small>개</small></strong></div>
   <div><span><Clock3 size={15}/>완료 보고가 있는 프로젝트</span><strong>{activeProjects.filter(p=>p.reportedComplete>0).length}<small>개</small></strong></div>
   <div><span><CheckCheck size={15}/>검증 근거가 연결된 프로젝트</span><strong>{activeProjects.filter(p=>p.verified>0).length}<small>개</small></strong></div>
   <div><span><ShieldCheck size={15}/>확인 필요 프로젝트</span><strong>{activeProjects.filter(p=>p.blocked>0||p.sourceErrors?.length).length}<small>개</small></strong></div>
  </div>
  <section className="panel daily-panel" aria-labelledby="daily-title">
   <div className="section-heading">
    <div><h2 id="daily-title">프로젝트 일별 진행현황</h2><p>각 날짜의 마지막 작업 개수입니다. 새로고침할 때 갱신합니다.</p></div>
    {dailyProject&&<div className="daily-controls">
     <select aria-label="진행현황 프로젝트" value={dailyProjectId} onChange={e=>setDailyProjectId(e.target.value)}>
      {activeProjects.map(p=><option key={p.id} value={p.id}>{p.name}</option>)}
     </select>
     {!advancedChart&&<><select aria-label="진행현황 기간" value={days} onChange={e=>setDays(e.target.value==='30'?30:7)}><option value={7}>최근 7일</option><option value={30}>최근 30일</option></select>
     <button type="button" className="subtle-button" aria-label="일별 차트 새로고침" onClick={()=>setChartRefresh(n=>n+1)}><RefreshCw size={14}/>새로고침</button></>}
     <button type="button" className="subtle-button" onClick={()=>setHistoryMode(advancedChart?'legacy':'knowledge')}>{advancedChart?'이전 집계 차트':'기준·과거 이력'}</button>
    </div>}
   </div>
   {dailyProject?<div className="daily-panel-content">
    <h3 className="daily-selected">{dailyProject.name}</h3>
    <ProjectSource project={dailyProject} current/>
    {advancedChart?<KnowledgeHistory key={dailyProject.id} api={knowledge} project={dailyProject} refreshVersion={chartRefresh}/>:dailyHistory.result?.state==='error'?<HistoryError message={dailyHistory.result.message} onRetry={dailyHistory.retry}/>
     :dailyHistory.result?.state==='ready'?<DailyProgressChart key={dailyProject.id} points={dailyPoints} days={days} today={chartToday} projectName={dailyProject.name}/>
     :<p className="history-pending" role="status">일별 진행 기록을 불러오는 중입니다.</p>}
   </div>:<div className="empty-project"><FolderKanban size={24}/><h3>연결된 프로젝트가 없습니다.</h3><p>프로젝트를 연결하면 일별 진행 기록을 볼 수 있습니다.</p></div>}
  </section>
  <section className="panel project-panel"><div className="section-heading"><div><h2>프로젝트 진행상황 <span className="count">{projects.length}</span></h2><p>완료 보고와 실제 검증 근거를 구분합니다.</p></div><button className="text-button" onClick={()=>setPage('projects')}>전체 보기 <ArrowUpRight size={15}/></button></div><ProgressRows projects={projects} onAction={setProjectAction} onSelect={p=>{setSelected(p);setPage('projects');}}/></section>
  <div className="overview-operations">
   <section className="panel runtime-panel" aria-labelledby="runtime-title">
    <div className="section-heading"><div><h2 id="runtime-title">런타임 현재 상태</h2><p>5초 주기 점검</p></div></div>
    {snapshot.daemons.length?<div className="daemon-grid">{snapshot.daemons.map(d=><DaemonCard key={d.id} daemon={d}/>)}</div>:<p className="empty-copy">확인할 런타임이 없습니다.</p>}
   </section>
   <AttendancePanel attendance={snapshot.attendance} settings={snapshot.settings} onSettings={()=>setPage('settings')}/>
  </div>
  <section className="panel recent-panel"><div className="section-heading"><div><h2>최근 활동</h2><p>실제 수신한 작업 이벤트입니다.</p></div><Activity size={17}/></div>{snapshot.events.length?<div className="event-list">{snapshot.events.slice(0,5).map(event=><div className="event" key={event.id}><div className="event-dot"/><div><strong>{event.message||event.type}</strong><span>{event.daemonId} · {time(event.at)}</span></div></div>)}</div>:<div className="activity-empty"><Database size={25}/><p>아직 관측한 활동이 없습니다.</p><span>연결 상태와 작업 활동은 별도로 표시합니다.</span></div>}<button className="activity-more" onClick={()=>setPage('activity')}>활동 이력 보기 <ArrowDownRight size={15}/></button></section>
 </>}
 {page==='projects'&&snapshot&&<>
  <div className="list-toolbar"><div className="search"><Search size={16}/><input aria-label="프로젝트 검색" placeholder="프로젝트 검색" value={filter} onChange={e=>setFilter(e.target.value)}/></div><span className="small">총 {projects.length}개 프로젝트</span></div>
  <section className="panel" aria-labelledby="projects-title"><h2 className="sr-only" id="projects-title">프로젝트 목록</h2><ProgressRows projects={projects} onAction={setProjectAction} onSelect={setSelected}/></section>
  {selected&&<section className="panel detail">
   <div className="section-heading">
    <div><h2>{selected.name}</h2><p>{selected.root}</p></div>
    <div className="detail-actions">
     <button className="subtle-button" onClick={e=>{mutationTrigger.current=e.currentTarget;setBaseRevision(registry?.revision);setEditing(registry?.projects.find(p=>p.id===selected.id)??selected);setForm('project');}}>프로젝트 수정</button>
     <button className="danger-button" onClick={e=>{mutationTrigger.current=e.currentTarget;setBaseRevision(registry?.revision);setRemove({entity:'project',item:selected});}}>프로젝트 제거</button>
     <button className="icon-button" aria-label="상세 닫기" onClick={()=>setSelected(null)}><X size={17}/></button>
    </div>
   </div>
   <ProjectActions project={selected} onAction={setProjectAction}/>
   <ProjectKnowledge key={selected.id} api={knowledge} project={selected} refreshVersion={snapshotVersion}><div className="detail-summary"><span>전체 {selected.total}개</span><span className={selected.verified?'good-text':undefined}>검증 완료 {selected.verified}</span><span>완료 보고 {selected.reportedComplete}</span><span>막힘 {selected.blocked}</span></div>
   <div className="detail-source"><ProjectSource project={selected}/></div>
   {selected.tasks.length?<ul className="task-list">{selected.tasks.map(t=><li key={t.id}><span className={'task-dot '+t.state}/><span>{t.title}</span><small>{t.verified?'검증 완료':taskLabels[t.state]??t.state}{t.verificationEvidence?.map(e=><span key={e.id}> · {e.kind} · {e.reference} · {e.success?'성공':'실패'} · {e.subjectRevision} · {e.verifiedAt}</span>)}</small></li>)}</ul>:<p className="empty-copy">연결된 상세 작업 목록이 없습니다.</p>}
   <h3 className="detail-subheading">프로젝트 진행 추이</h3>
   {detailHistory.result?.state==='error'?<HistoryError message={detailHistory.result.message} onRetry={detailHistory.retry}/>
    :!detailHistory.result?<p className="empty-copy" role="status">진행 이력을 불러오는 중입니다.</p>
    :progressHistory.length?<><div className="detail-summary"><span>전체 작업 {progressHistory[progressHistory.length-1].total}</span><span>검증 완료 {progressHistory[progressHistory.length-1].verified}</span><span>완료 보고 {progressHistory[progressHistory.length-1].reportedComplete}</span></div><ProgressChart points={progressHistory}/>{progressHistory.some((p,i)=>i>0&&p.total!==progressHistory[i-1].total)&&<p className="empty-copy">전체 작업 수 변경: {progressHistory.filter((p,i)=>i>0&&p.total!==progressHistory[i-1].total).map(p=>`${time(p.at)} (${p.total}개)`).join(' · ')}</p>}<p className="empty-copy">변경이 기록된 시점만 표시합니다. 수치는 비율이 아닌 작업 개수입니다.</p></>
    :<p className="empty-copy">프로젝트 진행 이력이 아직 없습니다.</p>}
   <h3 className="detail-subheading">현재 Codex 계획</h3>{snapshot.daemons.some(d=>d.plan?.length)?snapshot.daemons.filter(d=>d.plan?.length).map(d=><div key={d.id}><span className="small">{d.name} · 프로젝트 연결 근거를 별도로 확인하세요.</span><ul className="task-list">{d.plan.map((p,i)=><li key={i}><span className={'task-dot '+p.status}/><span>{p.step}</span><small>{taskLabels[p.status]??p.status}</small></li>)}</ul></div>):<p className="empty-copy">관측된 현재 요청의 계획이 없습니다.</p>}
  </ProjectKnowledge></section>}
 </>}
 {page==='monitors'&&registry&&<section className="panel">
  <div className="section-heading"><div><h2>등록한 모니터 <span className="count">{monitors.length}</span></h2><p>항목 제거는 원본 프로젝트나 세션을 삭제하지 않습니다.</p></div></div>
  {monitors.length?<div className="monitor-list">{monitors.map(m=>{
   const observation=snapshot?.monitorStatuses?.[m.id];
   const status=monitorStates[observation?.state??'unknown']??monitorStates.unknown;
   return <div className="monitor-row" key={m.id}>
    <div className="monitor-symbol"><MonitorDot size={19}/></div>
    <div className="monitor-copy">
     <strong>{m.name}</strong><span>{monitorKinds[m.kind]??'기타 모니터'}</span>
     {observation?.reason&&<span>{observation.reason}</span>}
     <details className="monitor-details"><summary>관측 상세</summary><p>종류: {m.kind} · 원본: {m.source}</p>{observation&&<p>상태 코드: {observation.state}{observation.value!==null&&<> · 관측값: {observation.value} {observation.unit}</>} · 점검: {time(observation.checkedAt)}</p>}</details>
    </div>
    <div className="monitor-badges"><span className="status-pill neutral">{m.enabled?'사용 중':'일시 중지'}</span><span className={'status-pill '+status.tone}><i/>{status.label}</span></div>
    <div className="monitor-actions">
     <button className="subtle-button" aria-label={m.name+' 수정'} onClick={e=>{mutationTrigger.current=e.currentTarget;setBaseRevision(registry?.revision);setEditing(m);setMonitorKind(m.kind);setForm('monitor');}}>수정</button>
     <button className="icon-button" aria-label={m.name+' 제거'} onClick={e=>{mutationTrigger.current=e.currentTarget;setBaseRevision(registry?.revision);setRemove({entity:'monitor',item:m});}}><Trash2 size={17}/></button>
    </div>
   </div>;
  })}</div>:<div className="empty-project"><MonitorDot size={28}/><h3>필요한 관측 항목을 추가하세요.</h3><p>체크리스트 또는 구조화 상태 파일을 연결할 수 있습니다.</p></div>}
 </section>}
 {page==='activity'&&snapshot&&<section className="panel"><div className="section-heading"><div><h2>작업 이벤트</h2><p>내부 사고 내용이나 원문 대화는 수집하지 않습니다.</p></div></div>{snapshot.events.length?<div className="event-list full">{snapshot.events.map(event=><div className="event" key={event.id}><div className="event-dot"/><div><strong>{event.message||event.type}</strong><span>{event.daemonId} · {time(event.at)} · {event.type}</span></div></div>)}</div>:<div className="empty-project"><Activity size={28}/><h3>관측한 활동이 없습니다.</h3><p>작업 중인지 확인할 수 없는 경우 정상이나 대기로 추정하지 않습니다.</p></div>}</section>}
 {page==='settings'&&settings&&<><form className="settings-layout" onChangeCapture={()=>{settingsDirty.current=true;}} onSubmit={save}><section className="panel settings-panel"><div className="section-heading"><div><h2>자동 출퇴근</h2><p>매일 적용하며 처리 여부가 불명확하면 자동 재실행하지 않습니다.</p></div><Clock3 size={18}/></div><label className="toggle-row"><span><strong>자동 출근·퇴근 알림</strong><small>{settings.startTime||'미설정'} 이상 {settings.endTime||'미설정'} 미만 출근, {settings.checkoutTime||'미설정'} 이후 하루 한 번 퇴근 확인</small></span><input type="checkbox" checked={settings.attendanceEnabled} onChange={e=>setSettings({...settings,attendanceEnabled:e.target.checked})}/></label><div className="time-inputs"><label>출근 시작<input type="time" value={settings.startTime} onChange={e=>setSettings({...settings,startTime:e.target.value})}/></label><label>출근 종료 미만<input type="time" value={settings.endTime} onChange={e=>setSettings({...settings,endTime:e.target.value})}/></label><label>퇴근 확인 시작<input type="time" value={settings.checkoutTime} onChange={e=>setSettings({...settings,checkoutTime:e.target.value})}/></label></div><label className="field">기존 스마트원 실행 스크립트<input placeholder="smartone-attendance.ps1 경로" value={settings.smartoneScript} onChange={e=>setSettings({...settings,smartoneScript:e.target.value})}/><small>기존 계정 설정을 사용하며 DevDock에 비밀번호를 복사하지 않습니다.</small></label><div className="settings-note"><ShieldCheck size={17}/><span>실행 결과가 불명확할 때는 스마트원 화면에서 기록을 확인하세요.</span></div></section><section className="panel settings-panel"><div className="section-heading"><div><h2>시작과 데이터</h2><p>이 컴퓨터의 사용자 설정으로 저장합니다.</p></div><Database size={18}/></div><label className="toggle-row"><span><strong>Windows 로그인 시 시작</strong><small>트레이에서 시작하고 백그라운드 점검을 유지합니다.</small></span><input type="checkbox" checked={settings.autostartEnabled} onChange={e=>setSettings({...settings,autostartEnabled:e.target.checked})}/></label><label className="field">원시 관측 보존 기간<select value={settings.retentionDays} onChange={e=>setSettings({...settings,retentionDays:Number(e.target.value)})}><option value={7}>7일</option><option value={14}>14일</option><option value={30}>30일</option></select></label><div className="settings-note">기본 다크 테마 · 5초 주기 점검 · 데이터는 로컬에 저장합니다.</div></section><div className="settings-footer">{error&&snapshot&&<button type="button" className="subtle-button" onClick={()=>{settingsDirty.current=false;setSettings(snapshot.settings);setError('');}}>최신 설정 불러오기</button>}<button className="primary-button" disabled={busy} type="submit"><Check size={16}/>설정 저장</button></div></form><section className="panel document-settings-panel"><div className="section-heading"><div><h2>문서와 작업기록</h2><p>핸드오버와 NAS 문서 연결을 관리합니다.</p></div><button type="button" className="subtle-button" aria-expanded={documentsOpen} onClick={()=>setDocumentsOpen(v=>!v)}>문서 연결 설정</button></div>{documentsOpen&&<DocumentSettings api={knowledge} projects={activeProjects} refreshVersion={snapshotVersion}/>}</section></>}
 <footer className="main-footer"><span><ShieldCheck size={13}/> 관측한 사실과 검증 근거를 기준으로 표시합니다.</span><span>로컬 저장 · {Intl.DateTimeFormat().resolvedOptions().timeZone||'시스템 로컬 시간'}</span></footer></main></div>
 {form&&<Modal title={editing?(form==='project'?'프로젝트 수정':'모니터 수정'):(form==='project'?'프로젝트 연결':'모니터링 항목 추가')} returnFocus={mutationTrigger.current} onClose={()=>setForm(null)}><form onSubmit={submitForm}>{error&&<p className="alert error">{error}</p>}<label className="field">이름<input name="name" defaultValue={editing?.name??''} required autoFocus placeholder={form==='project'?'프로젝트 이름':'모니터링 항목 이름'}/></label>{form==='project'?<><label className="field">프로젝트 폴더<input name="root" defaultValue={editing&&'root' in editing?editing.root:''} required placeholder="프로젝트의 전체 경로"/></label><label className="field">체크리스트 파일<input name="checklistPath" defaultValue={editing&&'checklistPath' in editing?editing.checklistPath:''} placeholder="선택 사항 · Markdown 파일 경로"/></label></>:<><label className="field">종류<select name="kind" value={monitorKind} onChange={e=>setMonitorKind(e.target.value)}><option value="checklist">체크리스트</option><option value="structuredStatus">구조화 상태</option><option value="daemon">데몬</option></select></label><label className="field">{monitorKind==='daemon'?<>데몬 대상<select name="source" defaultValue={editing&&'source' in editing?editing.source:'A'}><option value="A">A</option><option value="B">B</option></select></>:<>소스 경로<input name="source" defaultValue={editing&&'source' in editing?editing.source:''} required placeholder="파일 경로"/></>}</label><label className="field">프로젝트<select name="projectId" required={monitorKind!=='daemon'} defaultValue={editing&&'projectId' in editing?editing.projectId??'':''}><option value="">프로젝트 선택</option>{activeProjects.map(p=><option key={p.id} value={p.id}>{p.name}</option>)}</select></label></>}<div className="modal-actions"><button type="button" className="subtle-button" onClick={()=>setForm(null)}>취소</button><button className="primary-button" disabled={busy} type="submit">{editing?'저장':'연결'}</button></div></form></Modal>}
 {remove&&<Modal title={remove.entity==='project'?'프로젝트 제거':'모니터링 항목 제거'} returnFocus={mutationTrigger.current} onClose={()=>setRemove(null)}>{error&&<p className="alert error">{error}</p>}<p className="modal-copy">{remove.item.name} 항목을 목록에서 제거합니다. 원본 파일과 프로젝트는 보존합니다.</p><div className="modal-actions"><button className="subtle-button" autoFocus onClick={()=>setRemove(null)}>취소</button><button className="danger-button" disabled={busy} onClick={()=>void perform(()=>api.mutate({entity:remove.entity,action:'remove',id:remove.item.id},baseRevision),'항목을 제거했습니다.').then(ok=>{if(ok)setRemove(null);})}>{remove.entity==='project'?'목록에서 제거':'항목 제거'}</button></div></Modal>}
 {projectAction&&<ProjectActionDialog key={projectAction.project.id+projectAction.action} api={knowledge} value={projectAction}
  onClose={()=>setProjectAction(null)}
  onSettings={()=>{setProjectAction(null);setPage('settings');setDocumentsOpen(true);}}/>}
 {checkout&&<Modal title="스마트원 퇴근을 실행할까요." returnFocus={checkout.returnFocus} onClose={()=>void answer(false)}>{error&&<p className="alert error">{error}</p>}<div className="prompt-icon"><Clock3 size={25}/></div><p className="modal-copy">{checkout.date} 퇴근을 기록할 수 있습니다. 실행을 선택하면 실제 스마트원 퇴근이 처리됩니다.</p><p className="small">현재 기록은 {snapshot?.attendance.message||'서버 상태 확인 필요'}입니다.</p><div className="modal-actions"><button ref={declineRef} className="subtle-button" onClick={()=>void answer(false)}>오늘은 안 함</button><button className="primary-button" disabled={busy} onClick={()=>void answer(true)}>실행</button></div></Modal>}

 </div>;
}
