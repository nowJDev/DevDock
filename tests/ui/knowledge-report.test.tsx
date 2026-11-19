// 일일보고에서 파일을 날짜별로 고르고 Codex용 요청문을 복사하는 흐름을 검증한다.
import {render,screen,within} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {expect,it,vi} from 'vitest';
import DailyReport from '../../src/knowledge/DailyReport';
import {knowledgeFixture} from './knowledge-fixture';

function setup(handoverError?:string) {
  const root='C:\\Agent\\handovers';
  const execute=vi.fn(async(command:string)=>{
    switch(command){
      case 'get_report_profile': return {workHeading:'VTS_Unity'};
      case 'list_report_handover_files': return {folders:[{sourceId:'shared',root,...(handoverError?{error:handoverError}:{}),files:handoverError?[]:[
        {name:'HANDOVER_20261117_closeout.snapshot.json',date:'2026-11-17'},
        {name:'HANDOVER_20261116_notes.md',date:'2026-11-16'},
      ]}]};
      default: throw Error(command);
    }
  });
  const f=knowledgeFixture({execute:execute as unknown as ReturnType<typeof knowledgeFixture>['api']['execute']});
  f.setConfig({revision:1,nasIntervalSec:30,localIntervalSec:5,sources:[
    {sourceId:'shared',scope:'global',projectId:null,location:'local',root,roles:['handover'],enabled:true},
    {sourceId:'nas',scope:'project',projectId:'p',location:'nas',root:'Z:/docs',roles:['dailyLog'],enabled:true},
  ]});
  const indexed=(sourceId:string,path:string,versionId:string)=>({
    documentId:sourceId+'/'+path,versionId,sourceId,path,sha256:'hash',bytes:20,capturedAt:'2026-11-18T00:00:00Z',kind:'md',
  });
  vi.mocked(f.api.documents).mockResolvedValueOnce({page:0,pageSize:2,documents:[
    indexed('nas','기록/2026-11-18.md','v1'),indexed('shared','HANDOVER_20261116_notes.md','v2'),
  ]}).mockResolvedValueOnce({page:1,pageSize:2,documents:[indexed('nas','2026-11-17.md','v3')]});
  const user=userEvent.setup();
  render(<DailyReport api={f.api} project={{id:'p',name:'개발'}} returnFocus={null} onClose={()=>{}}/>);
  return {user,execute,api:f.api};
}

it('shows handovers first and filters both groups by document date',async()=>{
  const {user}=setup();
  const list=await screen.findByRole('region',{name:'보고에 사용할 문서'});
  const [first,second]=within(list).getAllByRole('heading',{level:3});
  expect(first).toHaveTextContent('핸드오버');
  expect(second).toHaveTextContent('프로젝트 문서');
  expect(within(list).getAllByRole('checkbox',{name:'HANDOVER_20261116_notes.md'})).toHaveLength(1);
  expect(within(list).getByRole('checkbox',{name:'기록/2026-11-18.md'})).toBeInTheDocument();
  list.focus();
  expect(list).toHaveFocus();
  await user.selectOptions(screen.getByLabelText('문서 날짜'),'2026-11-17');
  expect(within(list).queryByRole('checkbox',{name:'HANDOVER_20261116_notes.md'})).toBeNull();
  expect(within(list).queryByRole('checkbox',{name:'기록/2026-11-18.md'})).toBeNull();
  expect(within(list).getByRole('checkbox',{name:'2026-11-17.md'})).toBeInTheDocument();
  expect(within(list).getByRole('checkbox',{name:'HANDOVER_20261117_closeout.snapshot.json'})).toBeInTheDocument();
});

it('copies the exact prompt for selected files across dates without starting a job',async()=>{
  const {user,execute}=setup();
  const ask=await screen.findByRole('button',{name:'Codex 보고 요청'});
  expect(ask).toBeDisabled();
  await user.click(await screen.findByLabelText('HANDOVER_20261117_closeout.snapshot.json'));
  await user.selectOptions(screen.getByLabelText('문서 날짜'),'2026-11-16');
  await user.click(screen.getByLabelText('HANDOVER_20261116_notes.md'));
  await user.click(ask);
  const prompt=[
    '$daily-work-report',
    '파일 위치: C:\\Agent\\handovers',
    '업무명: VTS_Unity',
    '파일명: C:/Agent/handovers/HANDOVER_20261117_closeout.snapshot.json',
    '파일명: C:/Agent/handovers/HANDOVER_20261116_notes.md',
  ].join('\n');
  expect(screen.getByLabelText('Codex 요청문')).toHaveValue(prompt);
  await user.click(screen.getByRole('button',{name:'복사'}));
  expect(await navigator.clipboard.readText()).toBe(prompt);
  expect(await screen.findByText('요청문을 복사했습니다.')).toBeInTheDocument();
  expect(execute.mock.calls.map(([command])=>command)).toEqual(['get_report_profile','list_report_handover_files']);
  await user.type(screen.getByLabelText('업무명'),' VR');
  expect(screen.queryByLabelText('Codex 요청문')).toBeNull();
});


it('selects and clears only documents visible under the selected date',async()=>{
  const {user}=setup();
  const list=await screen.findByRole('region',{name:'보고에 사용할 문서'});
  await user.click(within(list).getByRole('checkbox',{name:'HANDOVER_20261116_notes.md'}));
  await user.selectOptions(screen.getByLabelText('문서 날짜'),'2026-11-17');
  await user.click(screen.getByRole('button',{name:'현재 목록 전체 선택'}));
  expect(within(list).getAllByRole('checkbox',{checked:true})).toHaveLength(2);
  expect(screen.getByText(/선택한 파일 3개 · 현재 목록 2개 · 다른 날짜 1개/)).toBeInTheDocument();
  await user.click(screen.getByRole('button',{name:'현재 목록 선택 해제'}));
  expect(within(list).queryAllByRole('checkbox',{checked:true})).toHaveLength(0);
  expect(screen.getByText(/선택한 파일 1개 · 현재 목록 0개 · 다른 날짜 1개/)).toBeInTheDocument();
  await user.selectOptions(screen.getByLabelText('문서 날짜'),'');
  expect(within(list).getByRole('checkbox',{name:'HANDOVER_20261116_notes.md'})).toBeChecked();
  await user.click(screen.getByRole('button',{name:'현재 목록 전체 선택'}));
  expect(within(list).getAllByRole('checkbox',{checked:true})).toHaveLength(4);
  await user.click(screen.getByRole('button',{name:'Codex 보고 요청'}));
  expect(screen.getByLabelText<HTMLTextAreaElement>('Codex 요청문').value).toContain('기록/2026-11-18.md');
  await user.click(screen.getByRole('button',{name:'현재 목록 선택 해제'}));
  expect(screen.queryByLabelText('Codex 요청문')).toBeNull();
  expect(screen.getByRole('button',{name:'Codex 보고 요청'})).toBeDisabled();
});

it('can review and remove off-date selections from an already generated report',async()=>{
  const {user}=setup();
  await user.click(await screen.findByRole('checkbox',{name:'기록/2026-11-18.md'}));
  await user.click(screen.getByRole('checkbox',{name:'HANDOVER_20261116_notes.md'}));
  await user.selectOptions(screen.getByLabelText('문서 날짜'),'2026-11-17');
  await user.click(screen.getByRole('checkbox',{name:'2026-11-17.md'}));
  const request=screen.getByRole('button',{name:'Codex 보고 요청'});
  await user.click(request);
  expect(screen.getByLabelText<HTMLTextAreaElement>('Codex 요청문').value)
    .toContain('파일명: Z:/docs/기록/2026-11-18.md');
  expect(screen.getByText(/선택한 파일 3개 · 현재 목록 1개 · 다른 날짜 2개/)).toBeVisible();
  await user.click(screen.getByRole('button',{name:'선택한 파일 보기'}));
  const chosen=screen.getByRole('region',{name:'선택한 파일'});
  expect(within(chosen).getByRole('heading',{name:'2026-11-18'})).toBeVisible();
  await user.click(within(chosen).getByRole('button',{name:'기록/2026-11-18.md 선택 해제'}));
  expect(screen.queryByRole('button',{name:'복사'})).toBeNull();
  await user.click(request);
  expect(screen.getByLabelText<HTMLTextAreaElement>('Codex 요청문').value)
    .not.toContain('파일명: Z:/docs/기록/2026-11-18.md');
  await user.click(screen.getByRole('button',{name:'선택 모두 해제'}));
  expect(request).toBeDisabled();
});

it('keeps working documents selectable when a connected handover folder cannot be read',async()=>{
  const {user,execute,api}=setup('접근 권한 없음');
  expect(await screen.findByText(/C:\\Agent\\handovers.*접근 권한 없음/)).toBeVisible();
  expect(screen.getByRole('checkbox',{name:'2026-11-17.md'})).toBeEnabled();
  expect(screen.queryByRole('checkbox',{name:'HANDOVER_20261116_notes.md'})).toBeNull();
  vi.mocked(api.documents).mockResolvedValue({page:0,pageSize:100,documents:[]});
  execute.mockResolvedValueOnce({workHeading:'VTS_Unity'}).mockResolvedValueOnce({folders:[{
    sourceId:'shared',root:'C:\\Agent\\handovers',files:[{name:'HANDOVER_20261119_restored.md',date:'2026-11-19'}],
  }]});
  await user.click(screen.getByRole('button',{name:'다시 불러오기'}));
  expect(await screen.findByRole('checkbox',{name:'HANDOVER_20261119_restored.md'})).toBeEnabled();
  expect(screen.queryByText(/접근 권한 없음/)).toBeNull();
});

it('can reset a filtered date that disappeared when documents were reloaded',async()=>{
  const {user,execute,api}=setup('접근 권한 없음');
  await screen.findByRole('checkbox',{name:'2026-11-17.md'});
  await user.selectOptions(screen.getByLabelText('문서 날짜'),'2026-11-17');
  vi.mocked(api.documents).mockResolvedValue({page:0,pageSize:100,documents:[]});
  execute.mockResolvedValueOnce({workHeading:'VTS_Unity'}).mockResolvedValueOnce({folders:[{
    sourceId:'shared',root:'C:\\Agent\\handovers',files:[{name:'HANDOVER_20261119_new.md',date:'2026-11-19'}],
  }]});
  await user.click(screen.getByRole('button',{name:'다시 불러오기'}));
  expect(await screen.findByText('선택한 날짜에 문서가 없습니다.')).toBeVisible();
  await user.click(screen.getByRole('button',{name:'전체 날짜로 보기'}));
  expect(screen.getByRole('checkbox',{name:'HANDOVER_20261119_new.md'})).toBeEnabled();
});

it('distinguishes an empty connected project folder from a missing folder',async()=>{
  const execute=vi.fn(async(command:string)=>command==='list_report_handover_files'?{folders:[]}:{workHeading:'VTS_Unity'});
  const f=knowledgeFixture({execute:execute as unknown as ReturnType<typeof knowledgeFixture>['api']['execute']});
  f.setConfig({revision:1,nasIntervalSec:30,localIntervalSec:5,sources:[
    {sourceId:'nas',scope:'project',projectId:'p',location:'nas',root:'Z:/docs',roles:['dailyLog'],enabled:true},
  ]});
  vi.mocked(f.api.sourceStates).mockResolvedValue({sources:[{sourceId:'nas',state:'online'}]});
  render(<DailyReport api={f.api} project={{id:'p',name:'개발'}} returnFocus={null} onClose={()=>{}}/>);
  expect(await screen.findByText('연결된 폴더에 보고에 사용할 문서가 없습니다.')).toBeVisible();
  expect(screen.queryByText('이 프로젝트에 연결된 문서 폴더가 없습니다.')).toBeNull();
  expect(screen.getByRole('button',{name:'Codex 보고 요청'})).toBeDisabled();
});

it('shows a missing indexed folder as a retryable read failure',async()=>{
  const execute=vi.fn(async(command:string)=>command==='list_report_handover_files'?{folders:[]}:{workHeading:'검증'});
  const f=knowledgeFixture({execute:execute as unknown as ReturnType<typeof knowledgeFixture>['api']['execute']});
  f.setConfig({revision:1,nasIntervalSec:30,localIntervalSec:5,sources:[
    {sourceId:'nas',scope:'project',projectId:'p',location:'nas',root:'Z:/docs',roles:['dailyLog'],enabled:true},
  ]});
  vi.mocked(f.api.sourceStates).mockResolvedValueOnce({sources:[{sourceId:'nas',state:'invalidRoot',errors:['디렉터리가 없습니다.']}]}).mockResolvedValue({sources:[{sourceId:'nas',state:'online'}]});
  render(<DailyReport api={f.api} project={{id:'p',name:'개발'}} returnFocus={null} onClose={()=>{}}/>);
  expect(await screen.findByRole('alert')).toHaveTextContent(/Z:\/docs.*디렉터리가 없습니다/);
  await userEvent.click(screen.getByRole('button',{name:'다시 불러오기'}));
  expect(f.api.scan).toHaveBeenCalledWith(['nas'],true);
  expect(await screen.findByText('연결된 폴더에 보고에 사용할 문서가 없습니다.')).toBeVisible();
});
