// 일별 프로젝트 진행 값과 차트의 표 대체 경로를 검사합니다.
import {render, screen, waitFor, within} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {beforeEach, describe, expect, it, vi} from 'vitest';
import DailyProgressChart from '../../src/components/charts/DailyProgressChart';
import {dailyProgressRows} from '../../src/components/charts/dailyProgress';
import type {DailyProgressPoint} from '../../src/contracts';

const chart = vi.hoisted(() => ({
  init: vi.fn(),
  setOption: vi.fn(),
  dispose: vi.fn(),
  resize: vi.fn(),
}));
vi.mock('echarts/core', () => ({init: chart.init, use: vi.fn()}));
vi.mock('echarts/charts', () => ({LineChart: vi.fn()}));
vi.mock('echarts/components', () => ({
  GridComponent: vi.fn(),
  TooltipComponent: vi.fn(),
  LegendComponent: vi.fn(),
}));
vi.mock('echarts/renderers', () => ({CanvasRenderer: vi.fn()}));

function point(date: string, values: Partial<DailyProgressPoint> = {}): DailyProgressPoint {
  return {
    projectId: 'devdock',
    date,
    at: date + 'T12:00:00+09:00',
    total: 4,
    verified: 1,
    reportedComplete: 2,
    ...values,
  };
}

const today = '2026-03-10';

beforeEach(() => {
  vi.resetAllMocks();
  chart.init.mockReturnValue(chart);
});

describe('daily progress rows', () => {
  it('builds an ascending seven-day calendar through today and keeps unobserved dates null', () => {
    const first = point('2026-03-04', {total: 0, verified: 0, reportedComplete: 0});
    const last = point(today, {at: '2026-03-10T00:15:00+09:00', total: 7});
    expect(dailyProgressRows([point('2026-03-11'), last, point('2026-03-03'), first], 7, today)).toEqual([
      {date: '2026-03-04', point: first},
      {date: '2026-03-05', point: null},
      {date: '2026-03-06', point: null},
      {date: '2026-03-07', point: null},
      {date: '2026-03-08', point: null},
      {date: '2026-03-09', point: null},
      {date: today, point: last},
    ]);
  });

  it('uses the latest absolute collection time and leaves a single sample on its supplied date', () => {
    const latest = point(today, {
      at: '2026-03-10T01:00:00-08:00',
      total: 9,
      sourceCapturedAt: '2026-03-10T00:00:00+09:00',
    });
    const earlier = point(today, {
      at: '2026-03-10T22:15:00+14:00',
      sourceCapturedAt: '2026-03-10T20:00:00+09:00',
    });
    const rows = dailyProgressRows([latest, earlier], 30, today);
    expect(rows).toHaveLength(30);
    expect(rows?.[0]).toEqual({date: '2026-02-09', point: null});
    expect(rows?.filter(row => row.point)).toEqual([{date: today, point: latest}]);
  });

  it('preserves submillisecond ordering from RFC 3339 collection times', () => {
    const earlier = point(today, {at: '2026-03-10T10:00:00.123456+09:00'});
    const latest = point(today, {at: '2026-03-10T01:00:00.123456001Z', total: 5});
    expect(dailyProgressRows([earlier, latest], 7, today)?.at(-1)?.point).toEqual(latest);
  });
});

describe('DailyProgressChart', () => {
  it('uses the same dates, gaps, and changing counts in the chart and keyboard-accessible table', async () => {
    const user = userEvent.setup();
    const first = point('2026-03-04', {total: 0, verified: 0, reportedComplete: 0});
    const latest = point(today, {
      at: '2026-03-10T01:00:00+09:00',
      total: 7,
      verified: 3,
      reportedComplete: 2,
      inProgress: 1,
    });
    const points = [point('2026-02-09'), first, point(today, {at: '2026-03-10T00:00:00+09:00'}), latest];
    const {rerender} = render(<DailyProgressChart points={points} days={7} today={today} projectName="DevDock"/>);

    expect(screen.getByRole('img', {name: '프로젝트 일별 진행현황 차트'})).toBeInTheDocument();
    await waitFor(() => expect(chart.setOption).toHaveBeenCalledWith(expect.objectContaining({
      animation: false,
      xAxis: expect.objectContaining({
        type: 'category',
        data: ['2026-03-04', '2026-03-05', '2026-03-06', '2026-03-07', '2026-03-08', '2026-03-09', today],
      }),
      series: [
        expect.objectContaining({name: '전체 작업', connectNulls: false, data: [0, null, null, null, null, null, 7]}),
        expect.objectContaining({name: '검증 완료', connectNulls: false, data: [0, null, null, null, null, null, 3]}),
        expect.objectContaining({name: '미검증 완료 보고', connectNulls: false, data: [0, null, null, null, null, null, 2]}),
        expect.objectContaining({name: '진행 중', connectNulls: false, data: [null, null, null, null, null, null, 1]}),
      ],
    })));

    await user.tab();
    expect(screen.getByText('일별 데이터 표')).toHaveFocus();
    await user.keyboard('{Enter}');
    const table = screen.getByRole('table', {name: 'DevDock 일별 진행현황'});
    expect(within(table).getAllByRole('row')).toHaveLength(8);
    expect(within(table).getAllByRole('columnheader').map(header => header.textContent)).toEqual([
      '날짜', '전체 작업', '검증 완료', '미검증 완료 보고', '진행 중', '마지막 수집 시각', '집계 기준', '자료 기준',
    ]);
    const firstRow = within(table).getByRole('row', {name: /2026-03-04/});
    expect(within(firstRow).getAllByRole('cell').map(cell => cell.textContent).slice(0, 4)).toEqual(['0', '0', '0', '미기록']);
    const missing = within(table).getByRole('row', {name: /2026-03-05/});
    expect(within(missing).getByText('미수집')).toBeInTheDocument();
    expect(within(missing).queryByText('0')).toBeNull();
    const lastRow = within(table).getByRole('row', {name: /^2026-03-10/});
    expect(within(lastRow).getAllByRole('cell').map(cell => cell.textContent).slice(0, 4)).toEqual(['7', '3', '2', '1']);
    expect(lastRow.querySelector('time')).toHaveAttribute('datetime', latest.at);

    rerender(<DailyProgressChart points={points} days={30} today={today} projectName="DevDock"/>);
    expect(within(table).getAllByRole('row')).toHaveLength(31);
    expect(within(table).getByRole('row', {name: /2026-02-09/})).toBeInTheDocument();
    await waitFor(() => expect(chart.setOption).toHaveBeenCalledTimes(2));
    expect(chart.dispose).not.toHaveBeenCalled();
  });

  it('updates an existing ECharts instance without resetting a hidden legend series', async () => {
    const echarts = await vi.importActual<typeof import('echarts')>('echarts');
    echarts.setPlatformAPI({measureText: text => ({width: text.length * 8})});
    const instances: ReturnType<typeof echarts.init>[] = [];
    chart.init.mockImplementation(() => {
      const instance = echarts.init(null, undefined, {renderer: 'svg', ssr: true, width: 640, height: 260});
      instances.push(instance);
      return instance;
    });

    const {rerender, unmount} = render(
      <DailyProgressChart points={[point(today)]} days={7} today={today} projectName="DevDock"/>,
    );
    const totalSeries = (count: number) => ({
      series: expect.arrayContaining([
        expect.objectContaining({name: '전체 작업', data: [null, null, null, null, null, null, count]}),
      ]),
    });
    await waitFor(() => expect(instances[0]?.getOption()).toMatchObject(totalSeries(4)));
    const instance = instances[0];
    instance.dispatchAction({type: 'legendToggleSelect', name: '미검증 완료 보고'});
    const hiddenLegend = {legend: [{selected: {'미검증 완료 보고': false}}]};
    expect(instance.getOption()).toMatchObject(hiddenLegend);

    rerender(<DailyProgressChart points={[point(today, {total: 9})]} days={7} today={today} projectName="DevDock"/>);
    await waitFor(() => expect(instances.at(-1)?.getOption()).toMatchObject(totalSeries(9)));
    expect(chart.init).toHaveBeenCalledTimes(1);
    expect(instance.getOption()).toMatchObject(hiddenLegend);
    expect(instance.isDisposed()).toBeFalsy();
    unmount();
    expect(instance.isDisposed()).toBe(true);
  });

  it('renders line-only metric swatches and keeps point shapes tied to the source of the record', async () => {
    const echarts = await vi.importActual<typeof import('echarts')>('echarts');
    echarts.setPlatformAPI({measureText: text => ({width: text.length * 8})});
    const instance = echarts.init(null, undefined, {renderer: 'svg', ssr: true, width: 640, height: 260});
    chart.init.mockReturnValue(instance);
    const points = [
      point('2026-03-08', {origin: 'observed', inProgress: 0}),
      point('2026-03-09', {origin: 'reconstructed', inProgress: 0}),
      point(today, {origin: 'correction', inProgress: 0}),
    ];
    render(<DailyProgressChart points={points} days={3} today={today} projectName="DevDock" provenance/>);

    await waitFor(() => expect(instance.getOption()).toMatchObject({
      series: [4, 1, 2, 0].map(value => expect.objectContaining({
        data: ['circle', 'diamond', 'triangle'].map(symbol => expect.objectContaining({value, symbol})),
      })),
    }));
    const svg = new DOMParser().parseFromString(instance.renderToSVGString(), 'image/svg+xml');
    const legendPositions = new Set([...svg.querySelectorAll('[ecmeta_ssr_type="legend"]')]
      .map(hitArea => hitArea.getAttribute('transform')));
    const visibleSwatches = [...svg.querySelectorAll('path[transform]')].filter(path =>
      legendPositions.has(path.getAttribute('transform')) &&
      !path.hasAttribute('ecmeta_ssr_type') &&
      path.getAttribute('fill-opacity') !== '0',
    );
    expect(visibleSwatches).toHaveLength(4);
    expect(visibleSwatches.every(swatch => swatch.hasAttribute('stroke'))).toBe(true);
    expect(new Set(visibleSwatches.map(swatch => swatch.getAttribute('stroke-dasharray'))).size).toBe(4);
  });

  it('shows historical collection bases and warns only when known bases differ in the selected period', async () => {
    const user = userEvent.setup();
    const samplePoints = [
      point('2026-02-09', {total: 2, trackingBasis: '기능 단위', sourceCapturedAt: '2026-02-08'}),
      point('2026-03-04', {trackingBasis: null, sourceCapturedAt: null}),
      point(today, {total: 25, trackingBasis: 'UAT 항목 단위', sourceCapturedAt: '2026-03-09T14:00:00+09:00'}),
    ];
    const {rerender} = render(
      <DailyProgressChart points={samplePoints} days={30} today={today} projectName="DevDock"/>,
    );
    const changeNote = /선택한 기간에 집계 기준이 바뀌었습니다/;
    expect(screen.getByText(changeNote)).toBeVisible();
    await user.click(screen.getByText('일별 데이터 표'));

    const table = screen.getByRole('table');
    const earlyRow = within(table).getByRole('row', {name: /^2026-02-09/});
    expect(within(earlyRow).getByRole('cell', {name: '기능 단위'})).toBeInTheDocument();
    expect(within(earlyRow).getByText('2026-02-08')).toHaveAttribute('datetime', '2026-02-08');
    const legacyRow = within(table).getByRole('row', {name: /^2026-03-04/});
    expect(within(legacyRow).getAllByRole('cell', {name: '미기록'})).toHaveLength(3);
    const lastRow = within(table).getByRole('row', {name: /^2026-03-10/});
    expect(within(lastRow).getByRole('cell', {name: 'UAT 항목 단위'})).toBeInTheDocument();
    const recordedTimes = lastRow.querySelectorAll('time');
    expect(recordedTimes[0]).toHaveAttribute('datetime', samplePoints[2].at);
    expect(recordedTimes[1]).toHaveAttribute('datetime', samplePoints[2].sourceCapturedAt);

    rerender(<DailyProgressChart points={samplePoints} days={7} today={today} projectName="DevDock"/>);
    expect(screen.queryByText(changeNote)).toBeNull();
    expect(within(table).queryByText('기능 단위')).toBeNull();
  });

  it('suppresses mixed project data when the chart has no selected project ID to resolve it', () => {
    render(<DailyProgressChart
      points={[point(today), point(today, {projectId: 'other', total: 99})]}
      days={7}
      today={today}
      projectName="DevDock"
    />);
    expect(screen.getByText('프로젝트가 다른 기록이 포함되어 표시할 수 없습니다.')).toBeInTheDocument();
    expect(screen.queryByRole('img')).toBeNull();
    expect(chart.init).not.toHaveBeenCalled();
  });

  it('shows an empty period and still exposes its unobserved dates', async () => {
    const user = userEvent.setup();
    render(<DailyProgressChart points={[]} days={7} today={today} projectName="DevDock"/>);
    expect(screen.getByText('선택한 기간에 수집한 기록이 아직 없습니다.')).toBeInTheDocument();
    expect(screen.queryByRole('img')).toBeNull();
    await user.click(screen.getByText('일별 데이터 표'));
    expect(within(screen.getByRole('table')).getAllByText('미수집')).toHaveLength(7);
    expect(chart.init).not.toHaveBeenCalled();
  });

  it('keeps the data table available when chart initialization fails', async () => {
    const user = userEvent.setup();
    chart.init.mockImplementationOnce(() => { throw new Error('chart initialization failed'); });
    render(<DailyProgressChart points={[point(today)]} days={7} today={today} projectName="DevDock"/>);
    expect(await screen.findByText('차트를 표시할 수 없습니다. 일별 데이터 표에서 확인할 수 있습니다.')).toBeInTheDocument();
    expect(screen.queryByRole('img')).toBeNull();
    await user.click(screen.getByText('일별 데이터 표'));
    const row = within(screen.getByRole('table')).getByRole('row', {name: /^2026-03-10/});
    expect(within(row).getAllByRole('cell').map(cell => cell.textContent).slice(0, 3)).toEqual(['4', '1', '2']);
  });
});
