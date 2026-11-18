// 과거 수치의 기준 버전과 범위를 검증하고 일별 차트 입력으로 변환한다.
import type {DailyProgressPoint} from '../contracts';
import type {ProjectHistory} from './contracts';
export function rangeDays(from:string,through:string):number|null{
 if(!/^\d{4}-\d{2}-\d{2}$/.test(from)||!/^\d{4}-\d{2}-\d{2}$/.test(through))return null;
 const a=new Date(from+'T00:00:00Z'),b=new Date(through+'T00:00:00Z');
 if(!Number.isFinite(+a)||!Number.isFinite(+b)||a.toISOString().slice(0,10)!==from||b.toISOString().slice(0,10)!==through)return null;
 const count=(+b-+a)/86400000+1;return Number.isInteger(count)&&count>=1&&count<=366?count:null;
}
export function historyChartPoints(history:ProjectHistory,version:number,name:string):DailyProgressPoint[]{
 return history.points.filter(p=>p.basisVersion===version&&p.counts&&(p.coverage==='complete'||p.coverage==='legacyUnknown')).map(p=>({projectId:history.projectId,date:p.date,at:p.capturedAt,total:p.counts!.total,verified:p.counts!.verified,reportedComplete:p.counts!.reportedComplete,trackingBasis:name,basisVersion:p.basisVersion,origin:p.origin,coverage:p.coverage,sourceSpans:p.sourceSpans}));
}
