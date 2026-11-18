// NAS 게시 확인을 누르기 전에 파일 쓰기 명령이 호출되지 않는지 확인한다.
import {render,screen} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {expect,it,vi} from 'vitest';
import PublishReview from '../../src/knowledge/PublishReview';
import {knowledgeFixture} from './knowledge-fixture';

it('requires an explicit application after reviewing the plan and target',async()=>{
 const path='일자별 작업기록/일지.md';
 const execute=vi.fn(async(command:string)=>{
   if(command==='get_document_publish_status')return {runId:'run',planHash:'hash',state:'previewReady',files:[{path,role:'dailyLog',oldText:'이전 원문',newText:'새 내용'}]};
   if(command==='apply_document_publish')return {runId:'run',planHash:'hash',state:'filesVerified',files:[],result:{files:[{path,state:'verified'}]}};
   if(command==='list_document_publishes')return {runs:[]};
   throw Error(command);
 });
 const api=knowledgeFixture({execute:execute as unknown as ReturnType<typeof knowledgeFixture>['api']['execute']}).api;
 const user=userEvent.setup();
 render(<PublishReview api={api} project={{id:'p',name:'프로젝트'}} sources={[]} opener={null} onClose={()=>{}} initialRun="run"/>);
 expect(await screen.findByText(path)).toBeInTheDocument();
 expect(execute).not.toHaveBeenCalledWith('apply_document_publish',expect.anything());
 await user.click(screen.getByRole('button',{name:'선택 파일 게시'}));
 expect(execute).not.toHaveBeenCalledWith('apply_document_publish',expect.anything());
 await user.click(screen.getByRole('button',{name:'NAS 게시 실행'}));
 expect(execute).toHaveBeenCalledWith('apply_document_publish',{runId:'run',planHash:'hash',paths:[path]});
 expect(await screen.findByText('선택한 파일의 게시 결과를 다시 읽어 확인했습니다.')).toBeInTheDocument();
});
