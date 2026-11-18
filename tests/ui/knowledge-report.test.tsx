// 보고서는 명시한 원문으로만 요청하고 수정하면 이전 검사 상태를 지운다.
import {render,screen} from '@testing-library/react';
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
