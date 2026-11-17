// 실제 관측값과 공백을 유지하며 A/B를 색·선·개별 상태 행으로 구분합니다.
import {useEffect,useRef} from 'react';
import type {History} from '../../contracts';
export default function ObservationChart({series,mode}:{series:{name:string;history:History}[];mode:'latency'|'state'}){
 const ref=useRef<HTMLDivElement>(null);
 useEffect(()=>{if(!ref.current||!series.some(s=>s.history.points.length))return;let disposed=false;let chart:{dispose():void;resize():void}|undefined;let observer:ResizeObserver|undefined;
 void Promise.all([import('echarts/core'),import('echarts/charts'),import('echarts/components'),import('echarts/renderers')]).then(([e,c,p,r])=>{
  if(disposed||!ref.current)return;e.use([c.LineChart,p.GridComponent,p.TitleComponent,p.TooltipComponent,p.LegendComponent,p.DataZoomComponent,r.CanvasRenderer]);
  const instance=e.init(ref.current,undefined,{renderer:'canvas'});chart=instance;
  const colors=['#62B5FF','#FFB454'];const states:Record<string,number>={healthy:3,slow:2,disconnected:1};const separated=mode==='state';
  const extent=series.reduce((bounds,s)=>s.history.points.reduce((b,point)=>{const at=Date.parse(point.at);return Number.isFinite(at)?[Math.min(b[0],at),Math.max(b[1],at)]:b;},bounds),[Infinity,-Infinity]);
  const timeMin=Number.isFinite(extent[0])?extent[0]-(extent[0]===extent[1]?5000:0):undefined;
  const timeMax=Number.isFinite(extent[1])?extent[1]+(extent[0]===extent[1]?5000:0):undefined;
  const axes=separated?series:[series[0]];
  instance.setOption({animation:false,color:colors,
   title:separated?series.map((s,i)=>({text:s.name,left:12,top:i*120+4,textStyle:{color:colors[i%colors.length],fontSize:11,fontWeight:600}})):[],
   grid:separated?series.map((_,i)=>({top:34+i*120,right:18,left:50,height:76})):{top:30,right:18,bottom:35,left:46},
   tooltip:{trigger:'axis',confine:true,backgroundColor:'#233044',borderColor:'#40516A',textStyle:{color:'#E8EEF7'}},
   axisPointer:{link:[{xAxisIndex:'all'}]},
   legend:{show:!separated,top:0,right:6,textStyle:{color:'#D4DFEE'},itemGap:22},
   xAxis:axes.map((_,i)=>({type:'time',gridIndex:i,min:timeMin,max:timeMax,axisLabel:{show:!separated||i===axes.length-1,color:'#A7B5C9',hideOverlap:true},axisLine:{lineStyle:{color:'#344257'}},splitLine:{show:false}})),
   yAxis:axes.map((_,i)=>mode==='latency'?{type:'value',gridIndex:i,name:'ms',nameTextStyle:{color:'#A7B5C9'},axisLabel:{color:'#A7B5C9'},splitLine:{lineStyle:{color:'#2B384A'}}}:{type:'value',gridIndex:i,min:0,max:3,interval:1,axisLabel:{color:'#A7B5C9',fontSize:10,formatter:(v:number)=>['공백','실패','지연','정상'][v]},splitLine:{lineStyle:{color:'#2B384A'}}}),
   dataZoom:[{type:'inside',filterMode:'none',xAxisIndex:axes.map((_,i)=>i)}],
   series:series.map((s,i)=>({name:s.name,type:'line',xAxisIndex:separated?i:0,yAxisIndex:separated?i:0,symbol:i?'diamond':'circle',symbolSize:6,showSymbol:true,showAllSymbol:'auto',connectNulls:false,step:separated?'end':undefined,lineStyle:{width:3,type:i?'dashed':'solid'},itemStyle:{color:colors[i%colors.length]},emphasis:{focus:'series'},data:s.history.points.flatMap((p,index,points)=>{const current:[string,number|null]=[p.at,mode==='latency'?p.latencyMs:(states[p.state]??null)];const previous=points[index-1];return previous&&Date.parse(p.at)-Date.parse(previous.at)>15000?[[new Date(Date.parse(previous.at)+5000).toISOString(),null],current]:[current];})}))});
  observer=new ResizeObserver(()=>instance.resize());observer.observe(ref.current);
 }).catch(()=>{});return()=>{disposed=true;observer?.disconnect();chart?.dispose();};
 },[series,mode]);
 return <div ref={ref} className="observation-chart" data-mode={mode} role="img" aria-label={mode==='latency'?'A/B 데몬 응답시간 관측 차트':'A/B 데몬 상태 관측 차트'}/>;
}
