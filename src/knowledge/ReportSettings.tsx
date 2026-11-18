// 프로젝트별 업무 헤더, 보고 양식과 기본 저장 위치를 편집한다.
import {useEffect,useState} from 'react';
import type {Project} from '../contracts';
import type {KnowledgeApi} from './api';
import type {DocumentSource,ReportProfile} from './contracts';

export default function ReportSettings({api,projects,sources}:{
  api:KnowledgeApi;projects:Project[];sources:DocumentSource[];
}){
  const [projectId,setProjectId]=useState(projects[0]?.id??'');
  const [profile,setProfile]=useState<ReportProfile|null>(null);
  const [busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState('');
  useEffect(()=>{
    if(!projectId)return;
    let active=true;
    setProfile(null);setError('');setNotice('');
    void api.execute<ReportProfile>('get_report_profile',{projectId}).then(value=>{
      if(active)setProfile(value);
    },e=>{if(active)setError(String(e));});
    return()=>{active=false;};
  },[api,projectId]);
  function change(value:Partial<ReportProfile>){
    setProfile(previous=>previous?{...previous,...value}:null);
    setNotice('');
  }
  const nas=sources.filter(s=>s.projectId===projectId&&s.location==='nas'&&s.enabled&&!s.archived);
  return <div className="document-source-editor">
    <label className="field">보고 설정 프로젝트<select value={projectId} onChange={e=>setProjectId(e.target.value)} disabled={busy}>
      {projects.map(p=><option key={p.id} value={p.id}>{p.name}</option>)}
    </select></label>
    {error&&<p className="alert error" role="alert">{error}</p>}
    {notice&&<p role="status" className="knowledge-status">{notice}</p>}
    {profile&&<form onSubmit={e=>{
      e.preventDefault();setBusy(true);setError('');setNotice('');
      void api.execute<ReportProfile>('save_report_profile',{profile,expectedRevision:profile.revision}).then(value=>{
        setProfile(value);setNotice('보고 설정을 저장했습니다.');
      },e=>setError(String(e))).finally(()=>setBusy(false));
    }}>
      <fieldset disabled={busy} className="document-roles">
        <legend>일일보고 설정</legend>
        <div className="knowledge-fields">
          <label className="field">기본 업무 헤더<input value={profile.workHeading} onChange={e=>change({workHeading:e.target.value})}/></label>
          <label className="field">기본 보고 양식<select value={profile.templateMode} onChange={e=>change({templateMode:e.target.value as ReportProfile['templateMode']})}>
            <option value="default">기본 · 10~12줄</option><option value="custom">사용자 지정</option>
          </select></label>
        </div>
        {profile.templateMode==='custom'&&<label className="field">사용자 지정 양식<textarea rows={4} maxLength={12000} value={profile.customTemplate??''} onChange={e=>change({customTemplate:e.target.value})}/></label>}
        <label className="field">설치된 daily-work-report SKILL.md 경로<input required value={profile.skillPath} onChange={e=>change({skillPath:e.target.value})}/></label>
        <label className="field">기본 로컬 저장 폴더<input value={profile.localSaveDirectory??''} onChange={e=>change({localSaveDirectory:e.target.value||null})}/></label>
        <label className="field">기본 NAS 문서 폴더<select value={profile.defaultNasFolderSourceId??''} onChange={e=>change({defaultNasFolderSourceId:e.target.value||null})}>
          <option value="">동작할 때 선택</option>{nas.map(s=><option key={s.sourceId} value={s.sourceId}>{s.root}</option>)}
        </select></label>
        <div className="knowledge-actions"><button className="primary-button">보고 설정 저장</button></div>
      </fieldset>
    </form>}
  </div>;
}
