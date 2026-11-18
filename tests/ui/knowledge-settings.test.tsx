// 문서 설정 입력·충돌·연결 실패가 근태 설정과 사용자 편집을 훼손하지 않는지 확인한다.
import {render,screen,waitFor} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {describe,it,expect,vi} from 'vitest';
import DocumentSettings from '../../src/knowledge/DocumentSettings';
import App from '../../src/App';
import {knowledgeFixture,dashboardFixture,sampleProjects} from './knowledge-fixture';
describe('문서 연결 설정',()=>{
 it('source_save_preserves_pending_interval_edit',async()=>{
  const f=knowledgeFixture(),user=userEvent.setup();render(<DocumentSettings api={f.api} projects={sampleProjects} refreshVersion={0}/>);const interval=await screen.findByLabelText('NAS 확인 간격 (초)');await user.clear(interval);await user.type(interval,'60');await user.click(screen.getByRole('button',{name:'문서 폴더 추가'}));await user.type(screen.getByLabelText('문서 폴더 경로'),'C:/문서');await user.click(screen.getByRole('button',{name:'문서 폴더 저장'}));await waitFor(()=>expect(screen.queryByLabelText('문서 폴더 경로')).not.toBeInTheDocument());expect(interval).toHaveValue(60);
 });
 it('settings_registers_multiple_nas_roots',async()=>{
  const f=knowledgeFixture();const user=userEvent.setup();render(<DocumentSettings api={f.api} projects={sampleProjects} refreshVersion={0}/>);
  for(const root of ['\\\\server\\문서\\첫째','\\\\server\\문서\\둘째']){
   await user.click(await screen.findByRole('button',{name:'문서 폴더 추가'}));await user.selectOptions(screen.getByLabelText('저장 위치'),'nas');await user.selectOptions(screen.getByLabelText('연결 범위'),'p');await user.type(screen.getByLabelText('문서 폴더 경로'),root);await user.click(screen.getByRole('button',{name:'문서 폴더 저장'}));await waitFor(()=>expect(screen.queryByLabelText('문서 폴더 경로')).not.toBeInTheDocument());
  }
  expect(f.api.upsertSource).toHaveBeenCalledTimes(2);expect(f.getConfig().sources.map(s=>s.root)).toEqual(['\\\\server\\문서\\첫째','\\\\server\\문서\\둘째']);
 });
 it('failed_probe_preserves_path_and_cache',async()=>{
  const f=knowledgeFixture({scan:vi.fn(async()=>{throw Error('document_worker_timeout');})});f.setConfig({revision:1,nasIntervalSec:30,localIntervalSec:5,sources:[{sourceId:'nas',scope:'project',projectId:'p',location:'nas',root:'\\\\server\\문서',roles:['dailyLog'],enabled:true}]});const user=userEvent.setup();render(<DocumentSettings api={f.api} projects={sampleProjects} refreshVersion={0}/>);await user.click(await screen.findByRole('button',{name:'연결 확인'}));expect(await screen.findByRole('alert')).toHaveTextContent('document_worker_timeout');expect(f.getConfig().sources[0].root).toBe('\\\\server\\문서');expect(screen.getByText('기존 캐시와 반영 이력은 유지됩니다.')).toBeInTheDocument();
 });
 it('background_refresh_keeps_dirty_document_form_and_conflict',async()=>{
  const f=knowledgeFixture();const user=userEvent.setup();const {rerender}=render(<DocumentSettings api={f.api} projects={sampleProjects} refreshVersion={0}/>);await user.click(await screen.findByRole('button',{name:'문서 폴더 추가'}));await user.type(screen.getByLabelText('문서 폴더 경로'),'C:/내 문서');f.setConfig({revision:1,nasIntervalSec:60,localIntervalSec:5,sources:[]});rerender(<DocumentSettings api={f.api} projects={sampleProjects} refreshVersion={1}/>);await waitFor(()=>expect(f.api.config).toHaveBeenCalledTimes(2));expect(screen.getByLabelText('문서 폴더 경로')).toHaveValue('C:/내 문서');await user.click(screen.getByRole('button',{name:'문서 폴더 저장'}));expect(await screen.findByRole('alert')).toHaveTextContent('conflict');expect(screen.getByLabelText('문서 폴더 경로')).toHaveValue('C:/내 문서');
 });
 it('document_save_does_not_save_attendance',async()=>{
  const f=knowledgeFixture(),app=dashboardFixture(),user=userEvent.setup();render(<App api={app} knowledge={f.api}/>);await user.click(await screen.findByRole('button',{name:'설정'}));await user.click(screen.getByRole('button',{name:'문서 연결 설정'}));const interval=await screen.findByLabelText('NAS 확인 간격 (초)');await user.clear(interval);await user.type(interval,'60');await user.click(screen.getByRole('button',{name:'문서 확인 간격 저장'}));await waitFor(()=>expect(f.api.saveConfig).toHaveBeenCalledTimes(1));expect(app.saveSettings).not.toHaveBeenCalled();expect(app.setAutostart).not.toHaveBeenCalled();
 });
});
