// 문서 화면을 실제 저장·근태와 분리해 검증할 공통 가짜 접점을 제공한다.
import {vi} from 'vitest';
import type {KnowledgeApi} from '../../src/knowledge/api';
import type {DocumentConfigView,ProjectProgressView} from '../../src/knowledge/contracts';
import type {Api,Project,Snapshot,Registry} from '../../src/contracts';
export const sampleProjects:Project[]=[{id:'p',name:'샘플 프로젝트',root:'C:/work/sample'}];
export function knowledgeFixture(overrides:Partial<KnowledgeApi>={}){
 let cfg:DocumentConfigView={revision:0,nasIntervalSec:30,localIntervalSec:5,sources:[]};
 const progress:ProjectProgressView={projectId:'p',revision:0,mode:'legacy',basis:null,bases:[],counts:{total:0,verified:0,reportedComplete:0,inProgress:0,blocked:0},reportRatio:null,verifiedRatio:null,sourceFreshness:'unlinked'};
 const api:KnowledgeApi={
  execute:async<T,>()=>({} as T),config:vi.fn(async()=>structuredClone(cfg)),
  saveConfig:vi.fn(async v=>{if(v.revision!==cfg.revision)throw Error('document_config_conflict');cfg={...v,revision:v.revision+1};return structuredClone(cfg);}),
  upsertSource:vi.fn(async(s,r)=>{if(r!==cfg.revision)throw Error('document_config_conflict');cfg={...cfg,revision:r+1,sources:[...cfg.sources.filter(v=>v.sourceId!==s.sourceId),s]};return structuredClone(cfg);}),
  archiveSource:vi.fn(async()=>cfg),sourceStates:vi.fn(async()=>({sources:[]})),scan:vi.fn(async()=>({state:'completed',sources:[]})),
  documents:vi.fn(async()=>({documents:[],page:0,pageSize:100})),read:vi.fn(async id=>({versionId:id,sha256:'hash',start:1,totalLines:1,text:'원문'})),
  progress:vi.fn(async()=>structuredClone(progress)),history:vi.fn(async(projectId,basisId,from,through)=>({projectId,basisId,from,through,points:[],workEntries:[]})),
  jobs:vi.fn(async()=>({jobs:[]})),createAnalysis:vi.fn(async()=>({jobId:'job',projectId:'p',kind:'progressAnalysis' as const,state:'pending',inputVersionIds:[],expectedProjectRevision:0,createdAt:'2026-01-01'})),
  parse:vi.fn(async()=>({jobId:'job',projectId:'p',expectedProjectRevision:0,inputVersionIds:[],candidates:[],conflicts:[]})),preview:vi.fn(async()=>({jobId:'job',projectId:'p',expectedProjectRevision:0,inputVersionIds:[],candidates:[],conflicts:[]})),...overrides,
 };
 return {api,setConfig:(v:DocumentConfigView)=>{cfg=v;},getConfig:()=>cfg};
}
export function dashboardFixture():Api {
 const settings={schemaVersion:1,revision:0,attendanceEnabled:true,startTime:'08:10',endTime:'08:30',checkoutTime:'17:30',smartoneScript:'',autostartEnabled:false,retentionDays:7};
 const registry:Registry={schemaVersion:1,revision:0,projects:sampleProjects,monitors:[],tasks:[],evidence:[]};
 const snapshot:Snapshot={now:'2026-11-18T11:00:00+09:00',registryRevision:0,daemons:[],projects:sampleProjects.map(p=>({...p,total:0,verified:0,reportedComplete:0,inProgress:0,blocked:0,updatedAt:null,currentTask:'',tasks:[]})),attendance:{date:'2026-11-18',state:'unknown',checkInTime:null,checkOutTime:null,lastCode:null,message:'확인 전',policy:{startTime:'08:10',endTime:'08:30',checkoutTime:'17:30'},runs:[]},events:[],settings};
 return {snapshot:vi.fn(async()=>snapshot),registry:vi.fn(async()=>registry),history:vi.fn(async id=>({monitorId:id,points:[],progress:[],dailyProgress:[]})),subscribe:vi.fn(async()=>()=>{}),mutate:vi.fn(async()=>({})),saveSettings:vi.fn(async()=>({})),attendance:vi.fn(async()=>({})),answerCheckin:vi.fn(async()=>({})),answerCheckout:vi.fn(async()=>({})),setAutostart:vi.fn(async()=>({})),refresh:vi.fn(async()=>({}))};
}
