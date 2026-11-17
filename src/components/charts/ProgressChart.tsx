// 프로젝트의 실제 작업 수 이력을 공백과 함께 시각화합니다.
import {useEffect,useRef} from 'react';
import type {History} from '../../contracts';
type Point=History['progress'][number];
export default function ProgressChart({points}:{points:Point[]}){
 const ref=useRef<HTMLDivElement>(null);
 useEffect(()=>{if(!ref.current||!points.length)return;let disposed=false;let chart:{dispose():void;resize():void}|undefined;let observer:ResizeObserver|undefined;
 void Promise.all([import('echarts/core'),import('echarts/charts'),import('echarts/components'),import('echarts/renderers')]).then(([e,c,p,r])=>{
  if(disposed||!ref.current)return;e.use([c.LineChart,p.GridComponent,p.TooltipComponent,p.LegendComponent,p.DataZoomComponent,r.CanvasRenderer]);const instance=e.init(ref.current,undefined,{renderer:'canvas'});chart=instance;
  instance.setOption({animation:false,color:['#A7B5C9','#69D5BC','#F2C66D'],grid:{top:36,right:18,bottom:45,left:46},tooltip:{trigger:'axis',confine:true,backgroundColor:'#233044',borderColor:'#40516A',textStyle:{color:'#E8EEF7'}},legend:{textStyle:{color:'#A7B5C9'}},xAxis:{type:'time',axisLabel:{color:'#A7B5C9',hideOverlap:true},splitLine:{show:false}},yAxis:{type:'value',min:0,minInterval:1,axisLabel:{color:'#A7B5C9'},splitLine:{lineStyle:{color:'#2B384A'}}},dataZoom:[{type:'inside',filterMode:'none'}],series:([['전체 작업','total'],['검증 완료','verified'],['완료 보고','reportedComplete']] as const).map(([name,key])=>({name,type:'line',connectNulls:false,showSymbol:true,lineStyle:{opacity:0},data:points.map(point=>[point.at,point[key]])}))});
  observer=new ResizeObserver(()=>instance.resize());observer.observe(ref.current);
 }).catch(()=>{});return()=>{disposed=true;observer?.disconnect();chart?.dispose();};
 },[points]);return <div ref={ref} className="observation-chart" role="img" aria-label="프로젝트 진행 추이 차트"/>;
}
