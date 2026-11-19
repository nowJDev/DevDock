// NAS 게시 요청 조건과 파일 반영·복구 확인 흐름을 검증한다.
import {render,screen,within} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {expect,it,vi} from 'vitest';
import PublishReview from '../../src/knowledge/PublishReview';
import type {DocumentSource,DocumentVersion} from '../../src/knowledge/contracts';
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

const source:DocumentSource={sourceId:'nas',scope:'project',projectId:'p',location:'nas',root:'C:/docs',roles:[],enabled:true,
 publishTargets:[{role:'checklist',path:'체크리스트.md'},{role:'dailyLog',path:'일지.md'}]};
const evidence:DocumentVersion={versionId:'v1',documentId:'d1',sourceId:'local',path:'원문.md',sha256:'hash',bytes:20,capturedAt:'2026-11-18T00:00:00Z',kind:'handover'};
const file={path:'일지.md',role:'dailyLog',oldText:'이전 원문',newText:'새 내용'};
const preview={runId:'run',planHash:'hash',state:'previewReady',files:[file]};

it('explains which draft condition is missing beside the request button',async()=>{
 const onProgressSettings=vi.fn();
 const {api}=knowledgeFixture({documents:async()=>({documents:[evidence],page:0,pageSize:100})});
 const user=userEvent.setup();
 render(<PublishReview api={api} project={{id:'p',name:'프로젝트'}} sources={[source]} opener={null} onClose={()=>{}} onProgressSettings={onProgressSettings}/>);
 const request=screen.getByRole('button',{name:'Codex 게시 초안 요청'});
 const original=await screen.findByRole('checkbox',{name:'원문.md'});
 expect(request).toHaveAccessibleDescription(/원문을 1~16개/);
 await user.click(original);
 expect(request).toBeDisabled();
 expect(request).toHaveAccessibleDescription(/체크리스트.*진행 기준/);
 await user.click(screen.getByRole('button',{name:'진행 기준 설정'}));
 expect(onProgressSettings).toHaveBeenCalledOnce();
 await user.click(screen.getByRole('checkbox',{name:/진행 체크리스트/}));
 expect(request).toBeEnabled();
 await user.click(screen.getByRole('checkbox',{name:/일자별 작업기록/}));
 expect(request).toBeDisabled();
 expect(request).toHaveAccessibleDescription(/게시 대상을 1개 이상/);
});

it('shows the document settings route beside an unconfigured draft request',async()=>{
 const onSettings=vi.fn(),{api}=knowledgeFixture();
 const user=userEvent.setup();
 render(<PublishReview api={api} project={{id:'p',name:'프로젝트'}} sources={[{...source,publishTargets:[]}]} opener={null} onClose={()=>{}} onSettings={onSettings}/>);
 const footer=screen.getByRole('group',{name:'문서 업데이트 작업'});
 expect(within(footer).getByRole('button',{name:'Codex 게시 초안 요청'})).toBeDisabled();
 await user.click(await within(footer).findByRole('button',{name:'문서 연결 설정 열기'}));
 expect(onSettings).toHaveBeenCalledOnce();
});

it('keeps the open comparison and file selection when a publish cannot start',async()=>{
 const safeError='안전 모드에서는 NAS 게시와 복구를 실행할 수 없습니다.';
 const execute=vi.fn(async(command:string)=>{
   if(command==='get_document_publish_status')return preview;
   if(command==='apply_document_publish')throw Error(safeError);
   if(command==='list_document_publishes')return {runs:[]};
   throw Error(command);
 });
 const {api}=knowledgeFixture({execute:execute as unknown as ReturnType<typeof knowledgeFixture>['api']['execute']});
 const user=userEvent.setup();
 render(<PublishReview api={api} project={{id:'p',name:'프로젝트'}} sources={[]} opener={null} onClose={()=>{}} initialRun="run"/>);
 const compare=await screen.findByText('게시 전후 원문 비교');
 await user.click(compare);
 await user.click(screen.getByRole('button',{name:'선택 파일 게시'}));
 await user.click(screen.getByRole('button',{name:'NAS 게시 실행'}));
 const footer=screen.getByRole('group',{name:'문서 업데이트 작업'});
 expect(await within(footer).findByRole('alert')).toHaveTextContent(safeError);
 expect(within(footer).getByRole('button',{name:'NAS 게시 실행'})).toBeEnabled();
 expect(screen.getByRole('checkbox',{name:file.path})).toBeChecked();
 expect(compare.closest('details')).toHaveAttribute('open');
});

it('blocks a second write until an uncertain NAS response is checked',async()=>{
 const apply=vi.fn(async()=>{throw Error('게시 응답이 끊겼습니다.');});
 const status=vi.fn().mockResolvedValueOnce(preview).mockRejectedValueOnce(Error('상태 연결 실패'))
   .mockResolvedValueOnce({...preview,state:'recoveryRequired',selectedPaths:[file.path],
     result:{files:[{path:file.path,state:'uncertain'}]}});
 const {api}=knowledgeFixture({execute:async<T,>(command:string)=>{
   if(command==='get_document_publish_status')return status() as Promise<T>;
   if(command==='apply_document_publish')return apply();
   if(command==='list_document_publishes')return {runs:[]} as T;
   throw Error(command);
 }});
 const user=userEvent.setup();
 render(<PublishReview api={api} project={{id:'p',name:'프로젝트'}} sources={[]} opener={null} onClose={()=>{}} initialRun="run"/>);
 await user.click(await screen.findByRole('button',{name:'선택 파일 게시'}));
 await user.click(screen.getByRole('button',{name:'NAS 게시 실행'}));
 const footer=screen.getByRole('group',{name:'문서 업데이트 작업'});
 expect(await within(footer).findByRole('alert')).toHaveTextContent('상태 연결 실패');
 expect(within(footer).queryByRole('button',{name:'NAS 게시 실행'})).not.toBeInTheDocument();
 await user.click(within(footer).getByRole('button',{name:'게시 상태 다시 확인'}));
 expect(await within(footer).findByRole('button',{name:'선택 파일 복구'})).toBeEnabled();
 expect(screen.getByText('일지.md · 확인 필요')).toBeInTheDocument();
 expect(apply).toHaveBeenCalledOnce();
});

it('does not expand the chosen recovery files when a recovery cannot start',async()=>{
 const second={path:'안내.md',role:'index',oldText:'이전 안내',newText:'새 안내'};
 const posted={...preview,state:'filesVerified',files:[file,second],selectedPaths:[file.path,second.path]};
 const execute=vi.fn(async(command:string)=>{
   if(command==='get_document_publish_status')return posted;
   if(command==='restore_document_publish')throw Error('안전 모드에서는 NAS 게시와 복구를 실행할 수 없습니다.');
   if(command==='list_document_publishes')return {runs:[]};
   throw Error(command);
 });
 const {api}=knowledgeFixture({execute:execute as unknown as ReturnType<typeof knowledgeFixture>['api']['execute']});
 const user=userEvent.setup();
 render(<PublishReview api={api} project={{id:'p',name:'프로젝트'}} sources={[]} opener={null} onClose={()=>{}} initialRun="run"/>);
 const skipped=await screen.findByRole('checkbox',{name:second.path});
 await user.click(skipped);
 const comparisons=screen.getAllByText('게시 전후 원문 비교');
 await user.click(comparisons[1]);
 await user.click(screen.getByRole('button',{name:'선택 파일 복구'}));
 await user.click(screen.getByRole('button',{name:'NAS 복구 실행'}));
 const footer=screen.getByRole('group',{name:'문서 업데이트 작업'});
 expect(await within(footer).findByRole('alert')).toHaveTextContent('안전 모드');
 expect(skipped).not.toBeChecked();
 expect(comparisons[1].closest('details')).toHaveAttribute('open');
 expect(within(footer).getByText('선택 파일 1개')).toBeInTheDocument();
});
