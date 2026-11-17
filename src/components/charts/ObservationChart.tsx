// 실제 관측값과 공백을 구분해 시계열 차트를 표시합니다.
import {useEffect,useRef} from 'react';
import type {History} from '../../contracts';
export default function ObservationChart({series,mode}:{series:{name:string;history:History}[];mode:'latency'|'state'}){
 const ref=useRef<HTMLDivElement>(null);
 useEffect(()=>{if(!ref.current||!series.some(s=>s.history.points.length))return;let disposed=false;let chart:{dispose():void;resize():void}|undefined;let observer:ResizeObserver|undefined;
 void Promise.all([import('echarts/core'),import('echarts/charts'),import('echarts/components'),import('echarts/renderers')]).then(([e,c,p,r])=>{
  if(disposed||!ref.current)return;e.use([c.LineChart,p.GridComponent,p.TooltipComponent,p.LegendComponent,p.DataZoomComponent,r.CanvasRenderer]);
  const instance=e.init(ref.current,undefined,{renderer:'canvas'});chart=instance;
  const states:Record<string,number>={healthy:3,slow:2,disconnected:1};
  instance.setOption({animation:false,color:['#7DB3FF','#69D5BC'],grid:{top:26,right:18,bottom:45,left:46},tooltip:{trigger:'axis',confine:true,backgroundColor:'#233044',borderColor:'#40516A',textStyle:{color:'#E8EEF7'}},legend:{top:0,right:6,textStyle:{color:'#A7B5C9'}},xAxis:{type:'time',axisLabel:{color:'#A7B5C9',hideOverlap:true},axisLine:{lineStyle:{color:'#344257'}},splitLine:{show:false}},yAxis:mode==='latency'?{type:'value',name:'ms',nameTextStyle:{color:'#A7B5C9'},axisLabel:{color:'#A7B5C9'},splitLine:{lineStyle:{color:'#2B384A'}}}:{type:'value',min:0,max:3,interval:1,axisLabel:{color:'#A7B5C9',formatter:(v:number)=>['공백','실패','지연','정상'][v]},splitLine:{lineStyle:{color:'#2B384A'}}},dataZoom:[{type:'inside',filterMode:'none'}],series:series.map((s,i)=>({name:s.name,type:'line',showSymbol:false,connectNulls:false,step:mode==='state'?'end':undefined,lineStyle:{width:2,type:i?'dashed':'solid'},data:s.history.points.flatMap((p,index,points)=>{const current:[string,number|null]=[p.at,mode==='latency'?p.latencyMs:(states[p.state]??null)];const previous=points[index-1];return previous&&Date.parse(p.at)-Date.parse(previous.at)>15000?[[new Date(Date.parse(previous.at)+5000).toISOString(),null],current]:[current];})}))});
  observer=new ResizeObserver(()=>instance.resize());observer.observe(ref.current);
 }).catch(()=>{});return()=>{disposed=true;observer?.disconnect();chart?.dispose();};
 },[series,mode]);
 return <div ref={ref} className="observation-chart" role="img" aria-label={mode==='latency'?'A/B 데몬 응답시간 관측 차트':'A/B 데몬 상태 관측 차트'}/>;
}
