// 보고서는 명시한 원문으로만 요청하고 수정하면 이전 검사 상태를 지운다.
import {render,screen,within} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {expect,it,vi} from 'vitest';
import DailyReport from '../../src/knowledge/DailyReport';
import {knowledgeFixture} from './knowledge-fixture';

it('requests selected Markdown and invalidates validation after editing the draft',async()=>{
  const execute=vi.fn(async(command:string)=>{
    switch(command){
      case 'get_report_profile': return {workHeading:'1.1 개발',templateMode:'default',localSaveDirectory:null};
      case 'create_report_request': return {requestId:'report-1',jobId:'job-1'};
      case 'get_report_draft':
        return {state:'completed',draft:{body:'검토할 초안',needsMoreSources:true,grounding:[]},sources:[]};
      case 'validate_report_draft': return {lineCount:1,issues:[],machine:{state:'passed'}};
      default: throw Error(command);
    }
  });
  const f=knowledgeFixture({execute:execute as unknown as ReturnType<typeof knowledgeFixture>['api']['execute']});
  f.setConfig({revision:1,nasIntervalSec:30,localIntervalSec:5,sources:[{
    sourceId:'local',scope:'project',projectId:'p',location:'local',root:'C:/records',roles:['handover'],enabled:true,
  }]});
  vi.mocked(f.api.documents).mockResolvedValue({page:0,pageSize:100,documents:['handover.md','tasks.json'].map(path=>({
    documentId:path,versionId:path,sourceId:'local',path,sha256:'hash',bytes:20,capturedAt:'2026-01-01',kind:'handover',
  }))});
  const user=userEvent.setup();
  render(<DailyReport api={f.api} project={{id:'p',name:'개발'}} returnFocus={null} onClose={()=>{}}/>);
  await user.click(await screen.findByLabelText('handover.md'));
  expect(screen.queryByLabelText('tasks.json')).toBeNull();
  await user.click(screen.getByRole('button',{name:'Codex 보고 요청'}));
  expect(execute).toHaveBeenCalledWith('create_report_request',{request:expect.objectContaining({
    templateMode:'default',selectedFiles:[{path:'C:/records/handover.md',expectedSha256:'hash'}],
  })});
  expect(await screen.findByText('선택한 근거가 부족합니다.')).toBeInTheDocument();
  const editor=screen.getByLabelText('보고서 본문');
  await user.click(screen.getByRole('button',{name:'형식 검사'}));
  expect(await screen.findByText(/기계 검사 통과/)).toBeInTheDocument();
  await user.type(editor,' 수정');
  expect(screen.queryByText(/기계 검사 통과/)).toBeNull();
});

it('exposes a keyboard-scrollable evidence list while keeping the request action available',async()=>{
  const execute=vi.fn(async(command:string)=>{
    switch(command){
      case 'get_report_profile': return {
        projectId:'p',revision:1,workHeading:'1.1 개발',templateMode:'default',
        customTemplate:null,skillPath:'',localSaveDirectory:null,defaultNasFolderSourceId:null,
      };
      case 'create_report_request': return {requestId:'report-40',jobId:'job-40'};
      case 'get_report_draft': return {requestId:'report-40',jobId:'job-40',state:'pending',draft:null,sources:[]};
      default: throw Error(command);
    }
  });
  const f=knowledgeFixture({execute:execute as unknown as ReturnType<typeof knowledgeFixture>['api']['execute']});
  f.setConfig({revision:1,nasIntervalSec:30,localIntervalSec:5,sources:[{
    sourceId:'local',scope:'project',projectId:'p',location:'local',root:'C:/records',roles:['handover'],enabled:true,
  }]});
  const documents=Array.from({length:40},(_,index)=>{
    const path='handover-'+(index+1)+'.md';
    return {documentId:path,versionId:path,sourceId:'local',path,sha256:'hash',bytes:20,capturedAt:'2026-01-01',kind:'handover'};
  });
  vi.mocked(f.api.documents).mockResolvedValue({page:0,pageSize:100,documents});
  const user=userEvent.setup();
  render(<DailyReport api={f.api} project={{id:'p',name:'개발'}} returnFocus={null} onClose={()=>{}}/>);

  const list=await screen.findByRole('region',{name:'보고에 사용할 문서'});
  list.focus();
  expect(list).toHaveFocus();
  await user.tab();
  expect(within(list).getByRole('checkbox',{name:'handover-1.md'})).toHaveFocus();
  const submit=screen.getByRole('button',{name:'Codex 보고 요청'});
  expect(submit).toBeDisabled();
  await user.click(within(list).getByRole('checkbox',{name:'handover-40.md'}));
  expect(screen.getByText('선택 1개 / 최대 16개')).toBeInTheDocument();
  await user.click(submit);
  expect(execute).toHaveBeenCalledWith('create_report_request',{request:expect.objectContaining({
    selectedFiles:[{path:'C:/records/handover-40.md',expectedSha256:'hash'}],
  })});
});
