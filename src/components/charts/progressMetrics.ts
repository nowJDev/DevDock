// 진행 화면의 항목 이름, 색상과 선 모양을 일관되게 유지합니다.
export const progressMetrics = [
  {key: 'total', label: '전체 작업', color: '#A7B5C9', symbolSize: 13, lineType: 'solid'},
  {key: 'verified', label: '검증 완료', color: '#62B5FF', symbolSize: 9, lineType: 'dashed'},
  {key: 'reportedComplete', label: '미검증 완료 보고', color: '#FFB454', symbolSize: 6, lineType: 'dotted'},
  {key: 'inProgress', label: '진행 중', color: '#5AD2A0', symbolSize: 4, lineType: [8, 3, 1, 3] as number[]},
] as const;
