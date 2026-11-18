// 상태의 정직한 표시와 편집·근태 확인 흐름을 검사합니다.
import {StrictMode} from 'react';
import {describe,it,expect,vi} from 'vitest';
import {act,fireEvent,render,screen,waitFor,within} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import App from '../../src/App';
import type {Api,History,Snapshot,Registry} from '../../src/contracts';

// jsdom에는 Canvas 렌더러가 없어 실제 표와 화면 데이터 흐름만 검사합니다.
vi.mock('echarts/core',()=>({
 use:()=>{},
 init:()=>({setOption:()=>{},resize:()=>{},dispose:()=>{}}),
}));

function fixture() {
 const settings={schemaVersion:1,revision:1,attendanceEnabled:true,startTime:'08:10',endTime:'08:30',checkoutTime:'17:30',smartoneScript:'',autostartEnabled:false,retentionDays:7};
 const snapshot:Snapshot={now:'2026-11-17T08:15:00+09:00',registryRevision:1,daemons:[{id:'A',name:'A daemon',transportState:'unknown',activityState:'unknown',latencyMs:null,checkedAt:null,lastActivityAt:null,reason:'접근 권한을 확인해 주세요.',plan:[]}],projects:[{id:'p1',name:'DevDock',root:'D:/work/DevDock',total:5,verified:2,reportedComplete:1,inProgress:1,blocked:1,updatedAt:null,currentTask:'트레이 동작 구현',tasks:[]}],attendance:{date:'2026-11-17',state:'unknown',checkInTime:null,checkOutTime:null,lastCode:null,message:'서버 상태를 확인하지 않았습니다.',policy:settings,runs:[]},events:[],settings};
 let registry:Registry={schemaVersion:1,revision:1,projects:[{id:'p1',name:'DevDock',root:'D:/work/DevDock'}],monitors:[{id:'m1',name:'빌드 결과',kind:'structuredStatus',source:'status.json',projectId:'p1',enabled:true}],tasks:[],evidence:[]};
 const listeners=new Map<string,(value:unknown)=>void>();
 const api:Api={snapshot:vi.fn(async()=>snapshot),registry:vi.fn(async()=>registry),history:vi.fn(async()=>({monitorId:'A',points:[],progress:[]})),mutate:vi.fn(async(operation:any,revision)=>{if(revision!==registry.revision)throw new Error('revision conflict');if(operation.action==='remove'){if(operation.entity==='monitor')registry={...registry,revision:registry.revision+1,monitors:registry.monitors.map(m=>m.id===operation.id?{...m,archived:true}:m)};else {registry={...registry,revision:registry.revision+1,projects:registry.projects.map(p=>p.id===operation.id?{...p,archived:true}:p)};snapshot.projects=snapshot.projects.map(p=>p.id===operation.id?{...p,archived:true}:p);}}else if(operation.action==='upsert'){if(operation.entity==='monitor')registry={...registry,revision:registry.revision+1,monitors:registry.monitors.map(m=>m.id===operation.value.id?operation.value:m)};else {registry={...registry,revision:registry.revision+1,projects:registry.projects.map(p=>p.id===operation.value.id?operation.value:p)};snapshot.projects=snapshot.projects.map(p=>p.id===operation.value.id?{...p,...operation.value}:p);}}return registry;}),saveSettings:vi.fn(async()=>({ok:true})),attendance:vi.fn(async()=>({ok:true})),answerCheckout:vi.fn(async()=>({expired:false})),setAutostart:vi.fn(async()=>({ok:true})),refresh:vi.fn(async()=>({ok:true})),subscribe:vi.fn(async(name,callback)=>{listeners.set(name,callback);return()=>listeners.delete(name);})};
 return {snapshot,api,listeners,changeRegistry:()=>{registry={...registry,revision:registry.revision+1};}};
}
describe('DevDock dashboard',()=>{
 it('does not label an unobserved daemon as working or healthy',async()=>{
  const {api}=fixture();render(<App api={api}/>);
  expect(await screen.findByText('접근 권한을 확인해 주세요.')).toBeInTheDocument();
  expect(screen.getAllByText('확인 불가').length).toBeGreaterThan(0);
  expect(screen.queryByText('응답 정상')).not.toBeInTheDocument();
 });
 it('separates reported completion from verified completion',async()=>{
  const {api}=fixture();render(<App api={api}/>);
  await screen.findByText('트레이 동작 구현');
  expect(screen.getByText('검증 완료 2')).toBeInTheDocument();
  expect(screen.getByText('완료 보고 1')).toBeInTheDocument();
  expect(screen.getByText('전체 5개')).toBeInTheDocument();
 });
 it('removes a monitor without removing its project',async()=>{
  const {api}=fixture();const user=userEvent.setup();render(<App api={api}/>);
  await screen.findByText('트레이 동작 구현');
  await user.click(screen.getByRole('button',{name:'모니터링'}));
  expect(await screen.findByText('빌드 결과')).toBeInTheDocument();
  await user.click(screen.getByRole('button',{name:'빌드 결과 제거'}));
  await user.click(screen.getByRole('button',{name:'항목 제거'}));
  await waitFor(()=>expect(screen.queryByText('빌드 결과')).not.toBeInTheDocument());
  expect(api.mutate).toHaveBeenCalledWith({entity:'monitor',action:'remove',id:'m1'},1);
  expect(api.attendance).not.toHaveBeenCalled();
 });
 it('defaults the checkout prompt to declining and never records on Enter',async()=>{
  const {api,listeners}=fixture();const user=userEvent.setup();render(<App api={api}/>);
  await screen.findByText('트레이 동작 구현');
  await waitFor(()=>expect(listeners.has('checkout-prompt')).toBe(true));
  listeners.get('checkout-prompt')!({date:'2026-11-17'});
  const decline=await screen.findByRole('button',{name:'오늘은 안 함'});
  await waitFor(()=>expect(decline).toHaveFocus());
  await user.keyboard('{Enter}');
  await waitFor(()=>expect(api.answerCheckout).toHaveBeenCalledWith(false,'2026-11-17'));
  expect(api.attendance).not.toHaveBeenCalled();
 });
 it('queries only the selected overview project for seven or thirty days',async()=>{
  const {api,snapshot}=fixture();
  snapshot.projects.push({...snapshot.projects[0],id:'p2',name:'보안 점검과 운영 준비 현황'});
  const user=userEvent.setup();render(<App api={api}/>);
  const panel=await screen.findByRole('region',{name:'프로젝트 일별 진행현황'});
  expect(screen.queryByText('런타임 관측 이력')).not.toBeInTheDocument();
  expect(screen.queryByRole('button',{name:'응답시간'})).not.toBeInTheDocument();
  const project=within(panel).getByRole('combobox',{name:'진행현황 프로젝트'});
  expect(project).toHaveValue('p1');
  await waitFor(()=>expect(api.history).toHaveBeenCalledExactlyOnceWith('p1',168));
  await user.selectOptions(project,'p2');
  await waitFor(()=>expect(api.history).toHaveBeenLastCalledWith('p2',168));
  await user.selectOptions(within(panel).getByRole('combobox',{name:'진행현황 기간'}),'30');
  await waitFor(()=>expect(api.history).toHaveBeenLastCalledWith('p2',720));
  expect(vi.mocked(api.history).mock.calls).toEqual([['p1',168],['p2',168],['p2',720]]);
 });

 it('distinguishes an overview history failure from an empty history and retries it',async()=>{
  const {api}=fixture();vi.mocked(api.history).mockRejectedValueOnce(new Error('기록 조회 실패'));
  const user=userEvent.setup();render(<App api={api}/>);
  const panel=within(await screen.findByRole('region',{name:'프로젝트 일별 진행현황'}));
  expect(await panel.findByRole('alert')).toHaveTextContent('기록 조회 실패');
  expect(panel.queryByText(/기록이 아직 없습니다|아직 기록/)).not.toBeInTheDocument();
  await user.click(panel.getByRole('button',{name:'다시 조회'}));
  await waitFor(()=>expect(panel.queryByRole('alert')).not.toBeInTheDocument());
  expect(await panel.findByText(/기록이 아직 없습니다|아직 기록/)).toBeInTheDocument();
  expect(api.history).toHaveBeenCalledTimes(2);
 });

 it('replaces daily data when an update arrives during a pending history read',async()=>{
  const {api,snapshot,listeners}=fixture();
  snapshot.now=new Date(2026,10,17,0,15).toISOString();
  Object.assign(snapshot.projects[0],{trackingBasis:'결제 회귀 검증 체크리스트',sourceCapturedAt:'2026-11-16T07:00:00Z'});
  const point={projectId:'p1',date:'2026-11-17',at:snapshot.now,total:13,verified:4,reportedComplete:3,trackingBasis:'회귀 검증 사례',sourceCapturedAt:'2026-11-16T07:00:00Z'};
  const history=(total:number):History=>({monitorId:'p1',points:[],progress:[],dailyProgress:[{...point,total}]});
  let resolveOld!:(response:History)=>void;
  vi.mocked(api.history).mockImplementationOnce(()=>new Promise(resolve=>{resolveOld=resolve;})).mockResolvedValueOnce(history(13));
  const user=userEvent.setup();render(<App api={api}/>);
  const panel=within(await screen.findByRole('region',{name:'프로젝트 일별 진행현황'}));
  await waitFor(()=>expect(api.history).toHaveBeenCalledTimes(1));
  await act(async()=>{listeners.get('dashboard-updated')!(null);});
  await act(async()=>{resolveOld(history(999));});
  await user.click(await panel.findByText(/일별 데이터 표/));
  const table=within(panel.getByRole('table'));
  expect(table.getByRole('cell',{name:'13'})).toBeInTheDocument();
  expect(panel.getByText('결제 회귀 검증 체크리스트')).toBeInTheDocument();
  expect(panel.getByText('현재 집계 기준')).toBeInTheDocument();
  expect(panel.getByText('현재 자료 기준')).toBeInTheDocument();
  expect(table.queryByRole('cell',{name:'999'})).not.toBeInTheDocument();
 });

 it('keeps a date-only source basis as the original calendar date',async()=>{
  const {api,snapshot}=fixture();
  snapshot.projects[0].sourceCapturedAt='2026-02-08';
  render(<App api={api}/>);
  const panel=within(await screen.findByRole('region',{name:'프로젝트 일별 진행현황'}));
  expect(panel.getByText('현재 자료 기준')).toBeInTheDocument();
  expect(panel.getByText('2026-02-08')).toHaveAttribute('datetime','2026-02-08');
 });

 it('shows the first snapshot and registry when polling and update events overlap a slow read',async()=>{
  const {api,snapshot,listeners}=fixture();
  const registry=await api.registry();
  vi.mocked(api.registry).mockClear();
  let resolveSnapshot!:(value:Snapshot)=>void;
  let resolveRegistry!:(value:Registry)=>void;
  const pendingSnapshot=new Promise<Snapshot>(resolve=>{resolveSnapshot=resolve;});
  const pendingRegistry=new Promise<Registry>(resolve=>{resolveRegistry=resolve;});
  vi.mocked(api.snapshot).mockReturnValueOnce(pendingSnapshot).mockReturnValue(new Promise(()=>{}));
  vi.mocked(api.registry).mockReturnValueOnce(pendingRegistry).mockReturnValue(new Promise(()=>{}));
  vi.useFakeTimers();
  try{
   await act(async()=>{render(<App api={api}/>);});
   await act(async()=>{
    await vi.advanceTimersByTimeAsync(5000);
    listeners.get('dashboard-updated')!(null);
    await vi.advanceTimersByTimeAsync(1000);
   });
   expect(api.snapshot).toHaveBeenCalledTimes(1);
   expect(api.registry).toHaveBeenCalledTimes(1);
   await act(async()=>{resolveSnapshot(snapshot);resolveRegistry(registry);});
   expect(screen.getByText('트레이 동작 구현')).toBeInTheDocument();
   fireEvent.click(screen.getByRole('button',{name:'모니터링'}));
   expect(screen.getByText('빌드 결과')).toBeInTheDocument();
  }finally{
   vi.useRealTimers();
  }
 });

 it('shows a daily history response that takes longer than the five-second poll',async()=>{
  const {api,snapshot}=fixture();
  snapshot.now=new Date(2026,10,17,0,15).toISOString();
  const point={projectId:'p1',date:'2026-11-17',at:snapshot.now,total:42,verified:4,reportedComplete:3};
  let resolveHistory!:(value:History)=>void;
  const pendingHistory=new Promise<History>(resolve=>{resolveHistory=resolve;});
  vi.mocked(api.history).mockReturnValueOnce(pendingHistory).mockReturnValue(new Promise(()=>{}));
  vi.useFakeTimers();
  try{
   await act(async()=>{render(<App api={api}/>);});
   await act(async()=>{await vi.advanceTimersByTimeAsync(5000);});
   expect(api.history).toHaveBeenCalledTimes(1);
   await act(async()=>{
    await vi.advanceTimersByTimeAsync(1000);
    resolveHistory({monitorId:'p1',points:[],progress:[],dailyProgress:[point]});
   });
   const panel=within(screen.getByRole('region',{name:'프로젝트 일별 진행현황'}));
   expect(panel.getByText('일별 데이터 표')).toBeInTheDocument();
   fireEvent.click(panel.getByText('일별 데이터 표'));
   const table=panel.getByText('DevDock 일별 진행현황').closest('table')!;
   expect(table).toHaveTextContent('42');
  }finally{
   vi.useRealTimers();
  }
 });

 it('ignores daily results from a previous project and a previous period',async()=>{
  const {api,snapshot}=fixture();
  snapshot.now=new Date(2026,10,17,0,15).toISOString();
  snapshot.projects.push({...snapshot.projects[0],id:'p2',name:'다른 프로젝트'});
  const history=(id:string,total:number):History=>({monitorId:id,points:[],progress:[],dailyProgress:[{projectId:id,date:'2026-11-17',at:snapshot.now,total,verified:0,reportedComplete:0}]});
  let resolveProject!:(value:History)=>void;
  let resolvePeriod!:(value:History)=>void;
  vi.mocked(api.history)
   .mockImplementationOnce(()=>new Promise(resolve=>{resolveProject=resolve;}))
   .mockImplementationOnce(()=>new Promise(resolve=>{resolvePeriod=resolve;}))
   .mockResolvedValue(history('p2',42));
  const user=userEvent.setup();render(<App api={api}/>);
  const panel=within(await screen.findByRole('region',{name:'프로젝트 일별 진행현황'}));
  await user.selectOptions(panel.getByRole('combobox',{name:'진행현황 프로젝트'}),'p2');
  await user.selectOptions(panel.getByRole('combobox',{name:'진행현황 기간'}),'30');
  await user.click(await panel.findByText('일별 데이터 표'));
  const table=within(panel.getByRole('table'));
  expect(table.getByRole('cell',{name:'42'})).toBeInTheDocument();
  await act(async()=>{resolvePeriod(history('p2',98));resolveProject(history('p1',99));});
  expect(table.getByRole('cell',{name:'42'})).toBeInTheDocument();
  expect(table.queryByRole('cell',{name:/98|99/})).not.toBeInTheDocument();
 });

 it('reselects an active project when the chosen project is removed',async()=>{
  const {api,snapshot,listeners}=fixture();
  snapshot.projects.push({...snapshot.projects[0],id:'p2',name:'다른 프로젝트'});
  const user=userEvent.setup();render(<App api={api}/>);
  const panel=within(await screen.findByRole('region',{name:'프로젝트 일별 진행현황'}));
  const project=panel.getByRole('combobox',{name:'진행현황 프로젝트'});
  await user.selectOptions(project,'p2');
  snapshot.projects[1]={...snapshot.projects[1],archived:true};
  await act(async()=>{listeners.get('dashboard-updated')!(null);});
  expect(project).toHaveValue('p1');
  await waitFor(()=>expect(api.history).toHaveBeenLastCalledWith('p1',168));
  snapshot.projects=[];
  await act(async()=>{listeners.get('dashboard-updated')!(null);});
  expect(panel.queryByRole('combobox',{name:'진행현황 프로젝트'})).not.toBeInTheDocument();
  expect(panel.getByText(/연결된 프로젝트가 없습니다/)).toBeInTheDocument();
 });

 it('labels an unobserved enabled monitor separately from monitored health',async()=>{
  const {api,snapshot}=fixture();
  snapshot.monitorStatuses={m1:{state:'unknown',value:null,unit:null,reason:'상태 파일을 읽지 못했습니다.',checkedAt:snapshot.now}};
  const user=userEvent.setup();render(<App api={api}/>);
  await screen.findByText('트레이 동작 구현');
  await user.click(screen.getByRole('button',{name:'모니터링'}));
  expect(screen.getByText('사용 중')).toBeInTheDocument();
  expect(screen.getByText('확인 불가')).toBeInTheDocument();
  expect(screen.queryByText(/정상/)).not.toBeInTheDocument();
  expect(screen.getByText(/상태 코드: unknown/)).not.toBeVisible();
  expect(screen.getByText('상태 파일을 읽지 못했습니다.')).toBeInTheDocument();
 });

 it('describes the actual automatic attendance times and updates the help during editing',async()=>{
  const {api,snapshot}=fixture();
  snapshot.settings.startTime='08:15';
  const user=userEvent.setup();render(<App api={api}/>);
  expect(await screen.findByText(/08:15.*자동 출근/)).toBeInTheDocument();
  await user.click(screen.getAllByRole('button',{name:'설정'})[0]);
  expect(screen.getByText(/08:15 이상 08:30 미만/)).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('출근 시작'),{target:{value:'08:20'}});
  expect(screen.getByText(/08:20 이상 08:30 미만/)).toBeInTheDocument();
  expect(api.saveSettings).not.toHaveBeenCalled();
 });

 it('traps modal focus and restores the opener after Escape or closing',async()=>{
  const {api}=fixture();const user=userEvent.setup();render(<StrictMode><App api={api}/></StrictMode>);
  await screen.findByText('트레이 동작 구현');
  const opener=screen.getByRole('button',{name:'프로젝트 연결'});
  await user.click(opener);
  const dialog=screen.getByRole('dialog');
  const close=within(dialog).getByRole('button',{name:'닫기'});
  const submit=within(dialog).getByRole('button',{name:'연결'});
  expect(within(dialog).getByLabelText('이름')).toHaveFocus();
  close.focus();await user.tab({shift:true});
  expect(submit).toHaveFocus();
  await user.tab();
  expect(close).toHaveFocus();
  await user.keyboard('{Escape}');
  expect(opener).toHaveFocus();
  await user.click(opener);
  await user.click(within(screen.getByRole('dialog')).getByRole('button',{name:'닫기'}));
  expect(opener).toHaveFocus();
 });
 it('displays selected project progress history with denominator changes',async()=>{
  const {api}=fixture();api.history=async(id)=>({monitorId:id,points:[],progress:id==='p1'?[ {at:'2026-11-17T00:00:00Z',projectId:'p1',total:5,verified:2,reportedComplete:1},{at:'2026-11-17T01:00:00Z',projectId:'p1',total:6,verified:3,reportedComplete:2},{at:'2026-11-17T01:00:00Z',projectId:'other',total:99,verified:99,reportedComplete:0}]:[]});const user=userEvent.setup();render(<App api={api}/>);await screen.findByText('트레이 동작 구현');await user.click(screen.getByRole('button',{name:/^프로젝트 1/}));await user.click(screen.getByRole('button',{name:/DevDock/}));expect(await screen.findByText('전체 작업 6')).toBeInTheDocument();expect(screen.getByText(/전체 작업 수 변경/)).toBeInTheDocument();expect(screen.getByRole('img',{name:'프로젝트 진행 추이 차트'})).toBeInTheDocument();
 });
 it('shows verification mode clearly',async()=>{const {api,snapshot}=fixture();snapshot.verificationMode=true;render(<App api={api}/>);expect(await screen.findByText(/검증 모드/)).toBeInTheDocument();});

 it('edits and removes a project while preserving the original',async()=>{
 const {api}=fixture();const user=userEvent.setup();render(<App api={api}/>);await screen.findByText('트레이 동작 구현');await user.click(screen.getByRole('button',{name:/^프로젝트 1/}));await user.click(screen.getByRole('button',{name:/DevDock/}));await user.click(screen.getByRole('button',{name:'프로젝트 수정'}));const name=screen.getByLabelText('이름');await user.clear(name);await user.type(name,'변경 프로젝트');await user.click(screen.getByRole('button',{name:'저장'}));expect(await screen.findByText('변경 프로젝트',{selector:'h2'})).toBeInTheDocument();expect(api.mutate).toHaveBeenCalledWith({entity:'project',action:'upsert',value:{id:'p1',name:'변경 프로젝트',root:'D:/work/DevDock',checklistPath:''}},1);await user.click(screen.getByRole('button',{name:'프로젝트 제거'}));expect(screen.getByText(/원본 파일과 프로젝트는 보존/)).toBeInTheDocument();await user.click(screen.getByRole('button',{name:'목록에서 제거'}));await waitFor(()=>expect(screen.queryByText('변경 프로젝트')).not.toBeInTheDocument());expect(api.mutate).toHaveBeenCalledWith({entity:'project',action:'remove',id:'p1'},2);
 });
 it('edits an existing monitor',async()=>{const {api}=fixture();const user=userEvent.setup();render(<App api={api}/>);await screen.findByText('트레이 동작 구현');await user.click(screen.getByRole('button',{name:'모니터링'}));await user.click(screen.getByRole('button',{name:'빌드 결과 수정'}));const name=screen.getByLabelText('이름');await user.clear(name);await user.type(name,'테스트 결과');await user.click(screen.getByRole('button',{name:'저장'}));expect(await screen.findByText('테스트 결과')).toBeInTheDocument();expect(api.mutate).toHaveBeenCalledWith({entity:'monitor',action:'upsert',value:{id:'m1',name:'테스트 결과',kind:'structuredStatus',source:'status.json',projectId:'p1',enabled:true}},1);});
 it('refreshes and closes the selected project with snapshots',async()=>{const {api,snapshot,listeners}=fixture();const user=userEvent.setup();render(<App api={api}/>);await screen.findByText('트레이 동작 구현');await user.click(screen.getByRole('button',{name:/^프로젝트 1/}));await user.click(screen.getByRole('button',{name:/DevDock/}));snapshot.projects=[{...snapshot.projects[0],verified:4}];listeners.get('dashboard-updated')!(null);expect(await screen.findByText('검증 완료 4',{selector:'span.good-text'})).toBeInTheDocument();snapshot.projects=[];listeners.get('dashboard-updated')!(null);await waitFor(()=>expect(screen.queryByRole('button',{name:'상세 닫기'})).not.toBeInTheDocument());});

 it('shows stale daemon and last known state separately',async()=>{const {api,snapshot}=fixture();Object.assign(snapshot.daemons[0],{freshness:'stale',transportState:'unknown',activityState:'unknown',lastKnown:{transportState:'healthy',activityState:'working'}});render(<App api={api}/>);expect(await screen.findByText(/오래된 관측/)).toBeInTheDocument();expect(screen.getByText(/마지막 관측.*응답 정상.*요청 처리 중/)).toBeInTheDocument();expect(screen.getByText(/화면 갱신/)).toBeInTheDocument();});
 it('shows the latest execution result separately from unconfirmed server state',async()=>{
  const {api,snapshot}=fixture();
  const run={attemptId:'a',requestedDate:snapshot.attendance.date,action:'checkIn',state:'uncertain',code:'UNCERTAIN',recordDate:null,time:null,message:'확인 필요',result:{code:'UNCERTAIN',message:'확인 필요'}};
  Object.assign(snapshot.attendance,{serverState:'unknown',lastRunsByAction:{checkIn:run,checkOut:null},runs:[run]});
  const user=userEvent.setup();render(<App api={api}/>);
  expect(await screen.findByText(/서버 상태.*확인 불가/)).toBeInTheDocument();
  expect(screen.getByText('출근 결과 확인 필요')).toBeVisible();
  expect(screen.getByText(/UNCERTAIN/)).not.toBeVisible();
  await user.click(screen.getByText('근태 실행 이력'));
  expect(screen.getByText(/UNCERTAIN/)).toBeVisible();
 });
 it('displays task verification evidence and source errors',async()=>{const {api,snapshot}=fixture();Object.assign(snapshot.projects[0],{sourceErrors:['체크리스트 읽기 실패'],total:1,verified:1,reportedComplete:0,tasks:[{id:'t',projectId:'p1',title:'테스트 작업',state:'reportedComplete',revision:'1',verified:true,verificationEvidence:[{id:'e',kind:'test',success:true,subjectRevision:'1',reference:'test.log',verifiedAt:'2026-11-17T00:00:00Z'}]}]});const user=userEvent.setup();render(<App api={api}/>);await screen.findByText('트레이 동작 구현');await user.click(screen.getByRole('button',{name:/^프로젝트 1/}));await user.click(screen.getByRole('button',{name:/DevDock/}));expect(screen.getByText('테스트 작업')).toBeInTheDocument();expect(screen.getByText(/test.log/)).toBeInTheDocument();expect(screen.getByText(/체크리스트 읽기 실패/)).toBeInTheDocument();expect(screen.getAllByText('완료 보고 0').length).toBeGreaterThan(0);});
 it('shows monitor observation and restricts daemon source',async()=>{const {api,snapshot}=fixture();Object.assign(snapshot,{monitorStatuses:{m1:{state:'failed',value:7,unit:'건',reason:'수집 실패',checkedAt:'2026-11-17T00:00:00Z'}}});const user=userEvent.setup();render(<App api={api}/>);await screen.findByText('트레이 동작 구현');await user.click(screen.getByRole('button',{name:'모니터링'}));expect(screen.getByText(/수집 실패/)).toBeInTheDocument();await user.click(screen.getByRole('button',{name:'항목 추가'}));await user.selectOptions(screen.getByLabelText('종류'),'daemon');expect(screen.getByLabelText('데몬 대상')).toBeInTheDocument();});
 it('keeps the checkout prompt date when the snapshot date changes',async()=>{const {api,snapshot,listeners}=fixture();const user=userEvent.setup();render(<App api={api}/>);await screen.findByText('트레이 동작 구현');listeners.get('checkout-prompt')!({date:'2026-11-17'});expect(await screen.findByText(/2026-11-17.*퇴근/)).toBeInTheDocument();snapshot.attendance.date='2026-11-18';listeners.get('dashboard-updated')!(null);await user.click(screen.getByRole('button',{name:'오늘은 안 함'}));await waitFor(()=>expect(api.answerCheckout).toHaveBeenCalledWith(false,'2026-11-17'));});

 it('retains edit input and base revision after registry changes',async()=>{const {api,listeners,changeRegistry}=fixture();const user=userEvent.setup();render(<App api={api}/>);await screen.findByText('트레이 동작 구현');await user.click(screen.getByRole('button',{name:/^프로젝트 1/}));await user.click(screen.getByRole('button',{name:/DevDock/}));await user.click(screen.getByRole('button',{name:'프로젝트 수정'}));const name=screen.getByLabelText('이름');await user.clear(name);await user.type(name,'작성 중');changeRegistry();listeners.get('dashboard-updated')!(null);await waitFor(()=>expect(api.registry).toHaveBeenCalledTimes(2));await user.click(screen.getByRole('button',{name:'저장'}));expect(await screen.findByRole('alert')).toHaveTextContent(/충돌|conflict/);expect(screen.getByLabelText('이름')).toHaveValue('작성 중');await act(async()=>{listeners.get('dashboard-updated')!(null);});expect(screen.getByRole('alert')).toHaveTextContent(/conflict/);expect(screen.getByLabelText('이름')).toHaveValue('작성 중');});
 it('refreshes history after five seconds while retaining an open form and save error',async()=>{
  const {api}=fixture();
  vi.mocked(api.mutate).mockRejectedValueOnce(new Error('등록할 수 없습니다.'));
  vi.useFakeTimers();
  try{
   await act(async()=>{render(<App api={api}/>);});
   expect(api.history).toHaveBeenCalledExactlyOnceWith('p1',168);
   fireEvent.click(screen.getByRole('button',{name:'프로젝트 연결'}));
   fireEvent.change(screen.getByLabelText('이름'),{target:{value:'작성 중인 프로젝트'}});
   fireEvent.change(screen.getByLabelText('프로젝트 폴더'),{target:{value:'C:/projects/review'}});
   await act(async()=>{fireEvent.click(within(screen.getByRole('dialog')).getByRole('button',{name:'연결'}));});
   expect(screen.getByRole('alert')).toHaveTextContent('등록할 수 없습니다.');
   await act(async()=>{await vi.advanceTimersByTimeAsync(5000);});
   expect(api.history).toHaveBeenCalledTimes(2);
   expect(screen.getByRole('alert')).toHaveTextContent('등록할 수 없습니다.');
   expect(within(screen.getByRole('dialog')).getByLabelText('이름')).toHaveValue('작성 중인 프로젝트');
  }finally{
   vi.useRealTimers();
  }
 });
 it('saves settings once with the original revision and preserves conflicting edits',async()=>{const {api,snapshot,listeners}=fixture();Object.assign(snapshot.settings,{revision:1});api.saveSettings=vi.fn(async s=>{if(s.revision!==snapshot.settings.revision)throw new Error('revision conflict');return {...s,revision:s.revision+1};});const user=userEvent.setup();render(<App api={api}/>);await screen.findByText('트레이 동작 구현');await user.click(screen.getAllByRole('button',{name:'설정'})[0]);await user.selectOptions(screen.getByLabelText('원시 관측 보존 기간'),'14');snapshot.settings={...snapshot.settings,revision:2,attendanceEnabled:false};listeners.get('dashboard-updated')!(null);await waitFor(()=>expect(api.snapshot).toHaveBeenCalledTimes(2));await user.click(screen.getByRole('button',{name:'설정 저장'}));expect(await screen.findByRole('alert')).toHaveTextContent(/충돌|conflict/);expect(screen.getByLabelText('원시 관측 보존 기간')).toHaveValue('14');expect(api.setAutostart).not.toHaveBeenCalled();await act(async()=>{listeners.get('dashboard-updated')!(null);});expect(screen.getByRole('alert')).toHaveTextContent(/conflict/);expect(screen.getByLabelText('원시 관측 보존 기간')).toHaveValue('14');await user.click(screen.getByRole('button',{name:'최신 설정 불러오기'}));expect(screen.queryByRole('alert')).not.toBeInTheDocument();expect(screen.getByLabelText('원시 관측 보존 기간')).toHaveValue('7');expect(screen.getByLabelText(/자동 출근·퇴근 알림/)).not.toBeChecked();await user.click(screen.getByRole('button',{name:'설정 저장'}));await waitFor(()=>expect(api.saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({revision:2,attendanceEnabled:false})));});

 it('informs the user when a dated checkout prompt expires',async()=>{const {api,listeners}=fixture();api.answerCheckout=vi.fn(async()=>({expired:true,date:'2026-11-17',answer:false}));const user=userEvent.setup();render(<App api={api}/>);await screen.findByText('트레이 동작 구현');listeners.get('checkout-prompt')!({date:'2026-11-17'});await user.click(await screen.findByRole('button',{name:'실행'}));expect(await screen.findByText(/만료되어 실행되지 않았습니다/)).toBeInTheDocument();});
 it('submits settings only once and keeps the revision',async()=>{const {api}=fixture();const user=userEvent.setup();render(<App api={api}/>);await screen.findByText('트레이 동작 구현');await user.click(screen.getAllByRole('button',{name:'설정'})[0]);await user.click(screen.getByLabelText(/Windows 로그인 시 시작/));await user.click(screen.getByRole('button',{name:'설정 저장'}));await waitFor(()=>expect(api.saveSettings).toHaveBeenCalledTimes(1));expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({revision:1,autostartEnabled:true}));expect(api.setAutostart).not.toHaveBeenCalled();});
 it('keeps a remove confirmation revision when registry changes',async()=>{const {api,listeners,changeRegistry}=fixture();const user=userEvent.setup();render(<App api={api}/>);await screen.findByText('트레이 동작 구현');await user.click(screen.getByRole('button',{name:'모니터링'}));await user.click(screen.getByRole('button',{name:'빌드 결과 제거'}));changeRegistry();listeners.get('dashboard-updated')!(null);await waitFor(()=>expect(api.registry).toHaveBeenCalledTimes(2));await user.click(screen.getByRole('button',{name:'항목 제거'}));expect(await screen.findByRole('alert')).toHaveTextContent(/conflict/);expect(screen.getByRole('dialog')).toBeInTheDocument();});

 it('signals readiness only after every event listener is registered',async()=>{
  const {api,listeners}=fixture();let release!:()=>void;const gate=new Promise<void>(resolve=>{release=resolve;});const subscribe=api.subscribe;api.subscribe=async(name,callback)=>{if(name==='checkout-prompt')await gate;return subscribe(name,callback);};Object.assign(api,{ready:async()=>{listeners.get('checkout-prompt')?.({date:'2026-11-17'});}});render(<App api={api}/>);await screen.findByText('트레이 동작 구현');expect(screen.queryByRole('dialog')).not.toBeInTheDocument();release();expect(await screen.findByText(/2026-11-17.*퇴근/)).toBeInTheDocument();
 });

 it('closes the resolved checkout prompt and shows attendance execution failure',async()=>{const {api,listeners}=fixture();api.answerCheckout=async()=>({expired:false,attendanceError:'근태 실행 실패'});const user=userEvent.setup();render(<App api={api}/>);await screen.findByText('트레이 동작 구현');listeners.get('checkout-prompt')!({date:'2026-11-17'});await user.click(await screen.findByRole('button',{name:'실행'}));expect(await screen.findByRole('alert')).toHaveTextContent('근태 실행 실패');expect(screen.queryByRole('dialog')).not.toBeInTheDocument();});
 it('keeps the checkout prompt open when resolving the answer fails',async()=>{const {api,listeners}=fixture();api.answerCheckout=async()=>{throw new Error('원장 저장 실패');};const user=userEvent.setup();render(<App api={api}/>);await screen.findByText('트레이 동작 구현');listeners.get('checkout-prompt')!({date:'2026-11-17'});await user.click(await screen.findByRole('button',{name:'실행'}));expect(await screen.findByRole('alert')).toHaveTextContent('원장 저장 실패');expect(screen.getByRole('dialog')).toBeInTheDocument();});

});
