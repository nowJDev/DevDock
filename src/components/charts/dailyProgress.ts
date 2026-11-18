// 날짜별 진행 기록을 표와 차트에서 쓸 수 있도록 정리합니다.
import type {DailyProgressPoint} from '../../contracts';

export interface DailyProgressRow {
  date: string;
  point: DailyProgressPoint | null;
}

export function dailyProgressRows(
  points: DailyProgressPoint[],
  days: number,
  today: string,
): DailyProgressRow[] | null {
  if (!Number.isInteger(days) || days < 1 || days > 366) return null;
  const projectId = points[0]?.projectId;
  if (points.some(point => point.projectId !== projectId)) return null;

  // 달력 날짜 계산에만 UTC를 사용하고, 수집 시각의 시간대로 날짜를 바꾸지 않습니다.
  const date = new Date(today + 'T00:00:00Z');
  if (!Number.isFinite(date.getTime())) return [];
  date.setUTCDate(date.getUTCDate() - days + 1);
  const start = date.toISOString().slice(0, 10);

  const latest = new Map<string, {point: DailyProgressPoint; at: number; submillisecond: string}>();
  for (const point of points) {
    if (point.date < start || point.date > today) continue;
    const at = Date.parse(point.at);
    if (!Number.isFinite(at)) continue;

    // RFC 3339 수집 시각에서 Date.parse가 보존하지 않는 밀리초 미만 자릿수를 비교합니다.
    const submillisecond = point.at.match(/\.\d{3}(\d+)/)?.[1].replace(/0+$/, '') ?? '';
    const previous = latest.get(point.date);
    if (!previous || at > previous.at || (at === previous.at && submillisecond > previous.submillisecond)) {
      latest.set(point.date, {point, at, submillisecond});
    }
  }

  return Array.from({length: days}, () => {
    const day = date.toISOString().slice(0, 10);
    date.setUTCDate(date.getUTCDate() + 1);
    return {date: day, point: latest.get(day)?.point ?? null};
  });
}
