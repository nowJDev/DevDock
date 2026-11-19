// 일일보고에서 파일을 날짜별로 고르고 Codex용 요청문을 복사하는 흐름을 검증한다.
import {render,screen,within} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {expect,it,vi} from 'vitest';
import DailyReport from '../../src/knowledge/DailyReport';
import {knowledgeFixture} from './knowledge-fixture';

function setup() {
  const root='C:\\Agent\\handovers';
  const execute=vi.fn(async(command:string)=>{
    switch(command){
      case 'get_report_profile': return {workHeading:'VTS_Unity'};
      case 'list_report_handover_files': return {folders:[{sourceId:'shared',root,files:[
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
  return {user,execute};
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
  await user.click(screen.getByRole('button',{name:'전체 선택'}));
  expect(within(list).getAllByRole('checkbox',{checked:true})).toHaveLength(2);
  expect(screen.getByText('선택 3개')).toBeInTheDocument();
  await user.click(screen.getByRole('button',{name:'전체 해제'}));
  expect(within(list).queryAllByRole('checkbox',{checked:true})).toHaveLength(0);
  expect(screen.getByText('선택 1개')).toBeInTheDocument();
  await user.selectOptions(screen.getByLabelText('문서 날짜'),'');
  expect(within(list).getByRole('checkbox',{name:'HANDOVER_20261116_notes.md'})).toBeChecked();
  await user.click(screen.getByRole('button',{name:'전체 선택'}));
  expect(within(list).getAllByRole('checkbox',{checked:true})).toHaveLength(4);
  await user.click(screen.getByRole('button',{name:'Codex 보고 요청'}));
  expect(screen.getByLabelText<HTMLTextAreaElement>('Codex 요청문').value).toContain('기록/2026-11-18.md');
  await user.click(screen.getByRole('button',{name:'전체 해제'}));
  expect(screen.queryByLabelText('Codex 요청문')).toBeNull();
  expect(screen.getByRole('button',{name:'Codex 보고 요청'})).toBeDisabled();
});
