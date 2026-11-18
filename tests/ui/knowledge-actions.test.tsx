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
  expect(execute).not.toHaveBeenCalledWith('apply_document_publish',expect.anything());
});
