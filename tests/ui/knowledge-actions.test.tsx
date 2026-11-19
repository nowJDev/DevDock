// 프로젝트 행의 일일보고와 NAS 버튼이 상세 열기와 분리되는지 확인한다.
import {render,screen} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {expect,it,vi} from 'vitest';
import App from '../../src/App';
import {dashboardFixture,knowledgeFixture} from './knowledge-fixture';

it('opens the row report without selecting project detail and provides a NAS settings link',async()=>{
  const execute=vi.fn(async(command:string)=>command==='list_report_handover_files'?{folders:[]}:{workHeading:'샘플 프로젝트',templateMode:'default'});
  const {api}=knowledgeFixture({execute:execute as unknown as ReturnType<typeof knowledgeFixture>['api']['execute']});
  const user=userEvent.setup();
  render(<App api={dashboardFixture()} knowledge={api}/>);
  await user.click(await screen.findByRole('button',{name:'샘플 프로젝트 일일보고'}));
  expect(screen.getByRole('dialog',{name:'일일보고 · 샘플 프로젝트'})).toBeInTheDocument();
  expect(screen.queryByRole('button',{name:'상세 닫기'})).toBeNull();
  await user.keyboard('{Escape}');
  await user.click(screen.getByRole('button',{name:'샘플 프로젝트 NAS 문서 열기'}));
  expect(await screen.findByText('이 프로젝트에 연결한 NAS 문서 폴더가 없습니다.')).toBeInTheDocument();
  await user.click(screen.getByRole('button',{name:'문서 연결 설정 열기'}));
  expect(await screen.findByRole('button',{name:'문서 폴더 추가'})).toBeInTheDocument();
  expect(await screen.findByLabelText('저장 위치')).toHaveValue('nas');
  expect(execute).not.toHaveBeenCalledWith('apply_document_publish',expect.anything());
});

it('opens a folder editor already linked to the project from an empty report',async()=>{
  const execute=vi.fn(async(command:string)=>command==='list_report_handover_files'?{folders:[]}:{workHeading:'샘플 프로젝트'});
  const {api}=knowledgeFixture({execute:execute as unknown as ReturnType<typeof knowledgeFixture>['api']['execute']});
  const user=userEvent.setup();
  render(<App api={dashboardFixture()} knowledge={api}/>);
  await user.click(await screen.findByRole('button',{name:'샘플 프로젝트 일일보고'}));
  expect(await screen.findByText('이 프로젝트에 연결된 문서 폴더가 없습니다.')).toBeVisible();
  await user.click(screen.getByRole('button',{name:'문서 연결 설정'}));
  expect(await screen.findByLabelText('연결 범위')).toHaveDisplayValue('샘플 프로젝트');
  expect(screen.getByLabelText('저장 위치')).toHaveValue('local');
  expect(screen.queryByRole('dialog',{name:'일일보고 · 샘플 프로젝트'})).toBeNull();
});

it('opens the selected NAS folder editor when publish targets are missing',async()=>{
  const folder='C:/nas/샘플';
  const f=knowledgeFixture();
  f.setConfig({revision:1,nasIntervalSec:30,localIntervalSec:5,sources:[{sourceId:'nas',scope:'project',projectId:'p',location:'nas',root:folder,roles:['dailyLog'],enabled:true}]});
  const user=userEvent.setup();
  render(<App api={dashboardFixture()} knowledge={f.api}/>);
  await user.click(await screen.findByRole('button',{name:'샘플 프로젝트 문서 업데이트'}));
  await user.click(await screen.findByRole('button',{name:'문서 연결 설정 열기'}));
  expect(await screen.findByLabelText('문서 폴더 경로')).toHaveValue(folder);
  expect(screen.getByLabelText('문서 폴더 경로')).toHaveFocus();
});
