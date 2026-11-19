// 일별 진행 기록의 차트와 접근 가능한 표를 렌더링합니다.
import {useEffect, useId, useMemo, useRef, useState, type CSSProperties} from 'react';
import type {EChartsType} from 'echarts/core';
import type {DailyProgressPoint} from '../../contracts';
import {dailyProgressRows, type DailyProgressRow} from './dailyProgress';
import {progressMetrics as metrics} from './progressMetrics';
import './dailyProgress.css';

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

type MetricKey = (typeof metrics)[number]['key'];

const chartOptions = {
  animation: false,
  backgroundColor: '#141E2C',
  color: metrics.map(metric => metric.color),
  grid: {top: 22, right: 24, bottom: 34, left: 48, outerBoundsMode: 'same'},
  tooltip: {
    trigger: 'axis',
    renderMode: 'richText',
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

function metricValue(point: DailyProgressPoint | null, key: MetricKey) {
  return point?.[key] ?? '미기록';
}

function tooltipText(params: unknown, rows: DailyProgressRow[], provenance: boolean) {
  const first = Array.isArray(params) ? params[0] : params;
  const index = first && typeof first === 'object' && 'dataIndex' in first ? Number(first.dataIndex) : -1;
  const row = rows[index];
  if (!row) return '';
  if (!row.point) return row.date + '\n미수집';
  const title = [row.date];
  if (provenance) {
    const origin = row.point.origin === 'observed' ? '실제 수집' : row.point.origin === 'correction' ? '명시적 보정' : '문서 복원';
    title.push(origin + ' · 기준 v' + (row.point.basisVersion ?? 0));
  }
  return [...title, ...metrics.map(metric => metric.label + ' ' + metricValue(row.point, metric.key))].join('\n');
}

export default function DailyProgressChart({points, days, today, projectName,provenance=false,onSelectDate}: DailyProgressChartProps) {
  const rows = useMemo(() => dailyProgressRows(points, days, today), [points, days, today]);
  const ref = useRef<HTMLDivElement>(null);
  const mountedChart = useRef<MountedChart | null>(null);
  const selection=useRef(onSelectDate);selection.current=onSelectDate;
  const visibleRows=useRef(rows);visibleRows.current=rows;
  const [failedRows, setFailedRows] = useState<DailyProgressRow[] | null>(null);
  const [hiddenMetrics, setHiddenMetrics] = useState<ReadonlySet<MetricKey>>(() => new Set());
  const [selectedDate, setSelectedDate] = useState<string>();
  const basisNoteId = useId();
  const hasData = rows?.some(row => row.point !== null) ?? false;
  const chartFailed = rows !== null && failedRows === rows;
  const knownBases = new Set(rows?.map(row => row.point?.trackingBasis?.trim()).filter(Boolean));
  const basisChanged = knownBases.size > 1;
  const comparison = rows?.find(row => row.date === selectedDate) ??
    rows?.reduce<DailyProgressRow | undefined>((latest, row) => row.point ? row : latest, undefined);

  function toggleMetric(key: MetricKey) {
    setHiddenMetrics(previous => {
      const next = new Set(previous);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  function selectDate(date: string) {
    setSelectedDate(date);
    selection.current?.(date);
  }

  useEffect(() => () => {
    disposeChart(mountedChart.current);
    mountedChart.current = null;
  }, []);

  useEffect(() => {
    const element = ref.current;
    if (!element || !rows || !hasData || chartFailed) return;

    const options = {
      xAxis: {data: rows.map(row => row.date)},
      // 조작은 HTML 범례로 받고 ECharts의 계열 선택 상태에 반영합니다.
      legend: {show: false, selected: Object.fromEntries(metrics.map(metric => [metric.label, !hiddenMetrics.has(metric.key)]))},
      series: metrics.map(metric => ({
        id: metric.key,
        data: rows.map(({point}) => {
          const value = point?.[metric.key];
          if (!point || value == null) return null;
          if (!provenance) return value;
          return {value, symbol: point.origin==='reconstructed'?'diamond':point.origin==='correction'?'triangle':'circle', symbolSize: metric.symbolSize};
        }),
      })),
      tooltip: {formatter: (params: unknown) => tooltipText(params, rows, provenance)},
    };

    function showFallback() {
      disposeChart(mountedChart.current);
      mountedChart.current = null;
      setFailedRows(rows);
    }

    if (mountedChart.current) {
      try {
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
        legend: options.legend,
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
      instance.on('click', (event: {dataIndex?: number}) => {
        const row = visibleRows.current?.[event.dataIndex ?? -1];
        if (row) selectDate(row.date);
      });
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
  }, [rows, hasData, chartFailed, hiddenMetrics, provenance]);

  return <>
    {basisChanged && (
      <p id={basisNoteId} className="daily-chart-note">
        선택한 기간에 집계 기준이 바뀌었습니다. 수치를 비교할 때 일별 데이터 표의 집계 기준을 확인해 주세요.
      </p>
    )}
    {hasData && !chartFailed && <div className="daily-legend" role="group" aria-label="차트 계열">
      {metrics.map(metric => <button
        key={metric.key}
        type="button"
        aria-pressed={!hiddenMetrics.has(metric.key)}
        onClick={() => toggleMetric(metric.key)}
      >
        <span
          className="daily-legend-swatch"
          data-line={Array.isArray(metric.lineType) ? 'dash-dot' : metric.lineType}
          style={{'--series-color': metric.color} as CSSProperties}
          aria-hidden="true"
        />
        {metric.label}
      </button>)}
    </div>}
    <div
      ref={ref}
      className="daily-chart"
      hidden={!hasData || chartFailed}
      role="img"
      aria-label="프로젝트 일별 진행현황 차트"
      aria-describedby={basisChanged ? basisNoteId : undefined}
    />
    {(!hasData || chartFailed) && <div className="daily-chart-placeholder">
      {!rows && <p>프로젝트가 다른 기록이 포함되어 표시할 수 없습니다.</p>}
      {rows && !hasData && <p>선택한 기간에 수집한 기록이 아직 없습니다.</p>}
      {chartFailed && <p role="status">차트를 표시할 수 없습니다. 일별 데이터 표에서 확인할 수 있습니다.</p>}
    </div>}
    {comparison && <section className="daily-comparison" aria-label="선택한 날짜 진행 수치" aria-live="polite" aria-atomic="true">
      <time dateTime={comparison.date}>{comparison.date}</time>
      <dl>
        {metrics.map(metric => <div key={metric.key}>
          <dt>{metric.label}</dt>
          <dd>{metricValue(comparison.point, metric.key)}</dd>
        </div>)}
      </dl>
    </section>}
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
                <th scope="row">{(onSelectDate || point)
                  ? <button type="button" className="text-button daily-date-button" aria-label={date+(onSelectDate?' 근거 보기':' 수치 보기')} aria-current={comparison?.date===date?'date':undefined} onClick={()=>selectDate(date)}>{date}</button>
                  : date}</th>
                {point ? <>
                  {metrics.map(metric => <td key={metric.key}>{metricValue(point, metric.key)}</td>)}
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
