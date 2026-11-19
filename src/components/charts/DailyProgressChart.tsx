// 일별 진행 기록의 차트와 접근 가능한 표를 렌더링합니다.
import {useEffect, useId, useMemo, useRef, useState} from 'react';
import type {EChartsType} from 'echarts/core';
import type {DailyProgressPoint} from '../../contracts';
import {dailyProgressRows, type DailyProgressRow} from './dailyProgress';
import {progressMetrics as metrics} from './progressMetrics';

interface DailyProgressChartProps {
  points: DailyProgressPoint[];
  days: number;
  today: string;
  projectName: string;
  provenance?: boolean;
  onSelectDate?: (date:string) => void;
}

interface MountedChart {
  instance: EChartsType;
  observer?: ResizeObserver;
}


const chartOptions = {
  animation: false,
  backgroundColor: '#141E2C',
  color: metrics.map(metric => metric.color),
  grid: {top: 44, right: 24, bottom: 34, left: 48, outerBoundsMode: 'same'},
  legend: {
    top: 8,
    // 계열 범례에는 선만 표시하고 점 모양은 기록 출처에 사용합니다.
    itemStyle: {opacity: 0},
    textStyle: {color: '#D4DFEE', fontSize: 12},
  },
  tooltip: {
    trigger: 'axis',
    confine: true,
    transitionDuration: 0,
    backgroundColor: '#233044',
    borderColor: '#40516A',
    textStyle: {color: '#E8EEF7'},
  },
  xAxis: {
    type: 'category',
    axisLine: {lineStyle: {color: '#40516A'}},
    axisLabel: {
      color: '#A7B5C9',
      hideOverlap: true,
      showMinLabel: true,
      showMaxLabel: true,
      formatter: (value: string) => value.slice(5).replace('-', '/'),
    },
  },
  yAxis: {
    type: 'value',
    min: 0,
    minInterval: 1,
    axisLabel: {color: '#A7B5C9'},
    splitLine: {lineStyle: {color: '#2B384A'}},
  },
};

const collectionTime = new Intl.DateTimeFormat('ko-KR', {
  dateStyle: 'short',
  timeStyle: 'medium',
  hourCycle: 'h23',
});

function disposeChart(chart: MountedChart | null) {
  chart?.observer?.disconnect();
  chart?.instance.dispose();
}

function SourceTime({value}: {value?: string | null}) {
  const source = value?.trim();
  if (!source) return '미기록';
  const time = Date.parse(source);
  if (!Number.isFinite(time)) return source;
  const display = /^\d{4}-\d{2}-\d{2}$/.test(source) ? source : collectionTime.format(time);
  return <time dateTime={source}>{display}</time>;
}

export default function DailyProgressChart({points, days, today, projectName,provenance=false,onSelectDate}: DailyProgressChartProps) {
  const rows = useMemo(() => dailyProgressRows(points, days, today), [points, days, today]);
  const ref = useRef<HTMLDivElement>(null);
  const mountedChart = useRef<MountedChart | null>(null);
  const selection=useRef(onSelectDate);selection.current=onSelectDate;
  const visibleRows=useRef(rows);visibleRows.current=rows;
  const [failedRows, setFailedRows] = useState<DailyProgressRow[] | null>(null);
  const basisNoteId = useId();
  const hasData = rows?.some(row => row.point !== null) ?? false;
  const chartFailed = rows !== null && failedRows === rows;
  const knownBases = new Set(rows?.map(row => row.point?.trackingBasis?.trim()).filter(Boolean));
  const basisChanged = knownBases.size > 1;

  useEffect(() => () => {
    disposeChart(mountedChart.current);
    mountedChart.current = null;
  }, []);

  useEffect(() => {
    const element = ref.current;
    if (!element || !rows || !hasData) return;

    const options = {
      xAxis: {data: rows.map(row => row.date)},
      series: metrics.map(metric => ({
        id: metric.key,
        data: rows.map(({point}) => {
          const value = point?.[metric.key];
          if (!point || value == null) return null;
          if (!provenance) return value;
          return {value, symbol: point.origin==='reconstructed'?'diamond':point.origin==='correction'?'triangle':'circle', symbolSize: metric.symbolSize};
        }),
      })),
      ...(provenance?{tooltip: {renderMode: 'richText', formatter: (params: unknown) => {
        const first = Array.isArray(params) ? params[0] : params;
        const index = first && typeof first==='object' && 'dataIndex' in first ? Number(first.dataIndex) : -1;
        const row = rows[index];
        if (!row?.point) return row?.date ?? '';
        const point = row.point;
        const origin = point.origin==='observed'?'실제 수집':point.origin==='correction'?'명시적 보정':'문서 복원';
        const title = [row.date, origin+' · 기준 v'+(point.basisVersion??0)];
        return [...title,...metrics.map(metric=>metric.label+' '+(point[metric.key]??'미기록'))].join('\n');
      }}}:{}),
    };

    function showFallback() {
      disposeChart(mountedChart.current);
      mountedChart.current = null;
      setFailedRows(rows);
    }

    if (mountedChart.current) {
      try {
        // 데이터만 병합해 사용자가 고른 범례 표시 상태를 보존합니다.
        mountedChart.current.instance.setOption(options);
      } catch {
        showFallback();
      }
      return;
    }

    let cancelled = false;
    void Promise.all([
      import('echarts/core'),
      import('echarts/charts'),
      import('echarts/components'),
      import('echarts/renderers'),
    ]).then(([echarts, charts, components, renderers]) => {
      if (cancelled) return;
      echarts.use([
        charts.LineChart,
        components.GridComponent,
        components.TooltipComponent,
        components.LegendComponent,
        renderers.CanvasRenderer,
      ]);
      const instance = echarts.init(element, undefined, {renderer: 'canvas'});
      const chart: MountedChart = {instance};
      mountedChart.current = chart;
      instance.setOption({
        ...chartOptions,
        xAxis: {...chartOptions.xAxis, ...options.xAxis},
        tooltip: {...chartOptions.tooltip,...options.tooltip},
        series: metrics.map((metric, index) => ({
          ...options.series[index],
          name: metric.label,
          type: 'line',
          symbol: 'circle',
          symbolSize: metric.symbolSize,
          showAllSymbol: true,
          connectNulls: false,
          lineStyle: {width: 2, type: metric.lineType},
        })),
      });
      if(selection.current)instance.on('click',(event:{dataIndex?:number})=>{const row=visibleRows.current?.[event.dataIndex??-1];if(row)selection.current?.(row.date);});
      if (typeof ResizeObserver !== 'undefined') {
        chart.observer = new ResizeObserver(() => {
          if (!element.hidden && mountedChart.current === chart) instance.resize();
        });
        chart.observer.observe(element);
      }
    }).catch(() => {
      if (!cancelled) showFallback();
    });

    return () => { cancelled = true; };
  }, [rows, hasData,provenance]);

  return <>
    {basisChanged && (
      <p id={basisNoteId} className="daily-chart-note">
        선택한 기간에 집계 기준이 바뀌었습니다. 수치를 비교할 때 일별 데이터 표의 집계 기준을 확인해 주세요.
      </p>
    )}
    <div
      ref={ref}
      className="daily-chart"
      hidden={!hasData || chartFailed}
      role="img"
      aria-label="프로젝트 일별 진행현황 차트"
      aria-describedby={basisChanged ? basisNoteId : undefined}
    />
    {!rows && <p className="daily-chart-note">프로젝트가 다른 기록이 포함되어 표시할 수 없습니다.</p>}
    {rows && !hasData && <p className="daily-chart-note">선택한 기간에 수집한 기록이 아직 없습니다.</p>}
    {chartFailed && <p className="daily-chart-note" role="status">차트를 표시할 수 없습니다. 일별 데이터 표에서 확인할 수 있습니다.</p>}
    {provenance&&<p className="daily-chart-note">원형은 실제 수집, 마름모는 문서 복원, 삼각형은 명시적 보정입니다. 날짜를 선택해 업무와 근거를 확인할 수 있습니다.</p>}
    {rows && <details>
      <summary>일별 데이터 표</summary>
      <div className="daily-chart-table">
        <table className="daily-table">
          <caption>{projectName} 일별 진행현황</caption>
          <thead>
            <tr>
              <th scope="col">날짜</th>
              {metrics.map(metric => <th key={metric.key} scope="col">{metric.label}</th>)}
              <th scope="col">마지막 수집 시각</th>
              <th scope="col">집계 기준</th>
              <th scope="col">자료 기준</th>
              {provenance&&<><th scope="col">기록 출처</th><th scope="col">기준 버전</th></>}
            </tr>
          </thead>
          <tbody>
            {rows.map(({date, point}) => (
              <tr key={date}>
                <th scope="row">{onSelectDate?<button type="button" className="text-button" aria-label={date+' 근거 보기'} onClick={()=>onSelectDate(date)}>{date}</button>:date}</th>
                {point ? <>
                  {metrics.map(metric => <td key={metric.key}>{point[metric.key]??'미기록'}</td>)}
                  <td><time dateTime={point.at}>{collectionTime.format(new Date(point.at))}</time></td>
                  <td>{point.trackingBasis?.trim() ? point.trackingBasis : '미기록'}</td>
                  <td><SourceTime value={point.sourceCapturedAt}/></td>
                  {provenance&&<><td>{point.origin==='observed'?'실제 수집':point.origin==='correction'?'명시적 보정':'문서 복원'}</td><td>{point.basisVersion===0?'기준 미기록':'v'+point.basisVersion}</td></>}
                </> : <td colSpan={metrics.length + 3+(provenance?2:0)}>미수집</td>}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </details>}
  </>;
}
