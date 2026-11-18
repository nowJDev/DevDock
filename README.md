# DevDock

프로젝트의 진행과 Codex 런타임 상태를 함께 확인하는 Windows 대시보드다. 기본 다크 테마, 트레이 상주, 차트와 로컬 저장을 제공한다.

## 주요 동작.

- 5초마다 A/B 데몬 상태를 다시 확인하고 관측한 사실과 마지막 확인 시각을 표시한다.
- 데몬 응답과 AI 요청 상태, 프로젝트 체크리스트와 검증 증거를 구분한다.
- 프로젝트·모니터를 UI 또는 CLI로 추가·수정·제거한다. 제거는 원본 프로젝트와 세션을 삭제하지 않는다.
- 체크리스트의 [ ]는 미착수, [-]는 진행, [?]는 결정 필요, [x]는 완료 보고로 읽는다. 완료 보고만으로 검증 완료를 만들지 않는다.
- 메인 화면에서 프로젝트를 선택해 최근 7일·30일의 일별 진행현황을 확인한다. 전체 항목 수·검증 완료·미검증 완료 보고를 구분하며 집계 기준과 자료 기준일을 표시한다.
- 일별 값은 해당 로컬 날짜의 마지막 유효 수집값이다. 수집 없는 날은 공백으로 남기며 오늘은 현재까지의 마지막 값을 사용한다. 프로젝트끼리 서로 다른 단위의 진행률을 합산하지 않는다.
- 최소화와 닫기는 트레이로 이동한다. 실제 종료는 트레이 메뉴를 사용한다.
- 설정한 시간대에 스마트원 자동 출근을 시도하고 퇴근 확인 시간 이후에는 실행 여부를 하루 한 번 묻는다. 기본값은 매일 08:10 이상 08:30 미만 출근, 17:30 이후 퇴근 확인이다.

## 실행과 설정.

빌드된 DevDock 실행 파일을 실행한다. Windows 로그인 시 시작은 설정에서 관리하며 트레이에서 시작한다. 기존 스마트원 실행 스크립트 경로를 설정하면 그 프로그램의 계정과 서버 판정을 재사용한다. 비밀번호를 DevDock으로 복사하지 않는다.

근태 결과가 불명확하면 자동 재전송하지 않는다. 기존 스마트원에서 기록을 확인한다. 퇴근 알림에서 오늘은 안 함 또는 닫기를 선택하면 당일 다시 표시하지 않는다. 주말과 휴일에도 같은 시간대가 적용된다.

기본 데이터는 Windows LocalAppData 아래 DevDock에 저장된다. DEVDOCK_DATA_DIR 환경변수로 데이터 디렉터리를 바꿀 수 있으며 앱과 CLI에 같은 값을 적용해야 한다.

## 관측 범위.

기존 Codex 데몬의 제어 소켓을 공식 프록시로 조회한다. 새로운 app-server를 만들어 기존 작업을 관측한 것처럼 표시하지 않는다. 세션 시작·재개·중단·승인 응답을 관측 목적으로 호출하지 않는다.

현재 요청의 상태를 읽을 수 있는 것과 세부 작업 이벤트 구독은 별개다. 관측하지 못한 세부 활동과 계획은 확인 불가로 표시한다. 프로젝트 진행은 등록한 체크리스트나 명시적으로 게시한 구조화 상태를 근거로 한다. 모델 내부 사고나 정확한 연산 시간을 표시한다고 주장하지 않는다.

## 개발.

Node.js, Rust, Windows C++ 빌드 도구와 WebView2 환경이 필요하다. 패키지 잠금 파일을 사용한다.

```powershell
npm ci
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
npm run desktop
npm run build:desktop
```

개발 빌드와 --safe 인자는 검증 모드다. 실제 근태·퇴근 팝업·자동 시작 변경을 차단한다. 운영 일정은 배포용 릴리스 빌드에서 적용한다.

브라우저 개발 서버는 별도 .devdock-data를 사용하는 읽기 전용 미리보기다. 실제 UI 편집·Windows 기능·근태는 네이티브 앱에서 사용한다. 미리보기 코어는 다음 명령으로 준비한다.

```powershell
cargo build --manifest-path src-tauri/Cargo.toml --bin devdock-cli
npm run dev
```

## Codex용 CLI.

devdock-cli는 UI와 같은 검증·저장 코드를 사용한다. 성공은 JSON을 표준 출력으로, 실패는 JSON과 비정상 종료 코드로 반환한다. JSON 인자는 사용하는 셸의 인자 전달 규칙에 맞춰 전달한다. 따옴표 문제를 피하려면 프로세스 API의 인자 배열을 사용한다.

```text
devdock-cli snapshot
devdock-cli registry
devdock-cli settings
devdock-cli validate
devdock-cli history A 24
devdock-cli project list
devdock-cli project add PROJECT_JSON
devdock-cli project update PROJECT_JSON
devdock-cli project remove PROJECT_ID
devdock-cli monitor add MONITOR_JSON
devdock-cli monitor remove MONITOR_ID
devdock-cli task upsert TASK_JSON
devdock-cli task archive TASK_ID
devdock-cli status publish STATUS_JSON
devdock-cli evidence add EVIDENCE_JSON
devdock-cli event EVENT_JSON
devdock-cli mutate OPERATION_JSON EXPECTED_REVISION
```

Project는 id, name, root와 선택적 checklistPath를 가진다. Monitor는 id, name, kind, source, enabled와 선택적 projectId를 가진다. kind는 daemon, checklist, structuredStatus다. daemon의 source는 A 또는 B이며 파일 모니터는 projectId가 필요하다. 파일 상대 경로는 프로젝트 root를 기준으로 해석한다. 같은 파일은 중복 집계하지 않는다.

Task에는 id, projectId, title, state, revision이 필요하다. state는 pending, inProgress, blocked, reportedComplete, cancelled다. Evidence에는 id, taskId, kind, success, subjectRevision, reference가 필요하다. 성공한 증거라도 작업 버전과 일치하지 않으면 현재 검증 완료로 집계하지 않는다. Markdown 작업 버전은 내용 해시로 결정되므로 파일을 수정하면 이전 증거가 만료한다. 검증 완료와 미검증 완료 보고의 수치는 서로 겹치지 않는다.

Status는 projectId, 선택적 currentTask, tasks, metrics를 가진다. 하위 작업의 projectId는 상위와 같아야 한다. Metric은 id, value, unit, state를 가진다. 등록된 structuredStatus 파일도 같은 형식을 사용한다.

설정 저장에는 조회한 revision이 필요하다. 다른 작성자가 먼저 변경하면 충돌을 반환하며 최신 값을 다시 읽어야 한다. 자동 시작 변경은 실제 Windows 등록과 함께 앱에서 처리한다.

CLI는 실제 근태를 실행하는 명령을 제공하지 않는다. 명령과 설정에 인증 정보·개인 계정·원문 대화를 넣지 않는다.

## 일별 진행 이력.

프로젝트 수치가 바뀌지 않아도 앱이 실제로 수집한 날짜에는 마지막 값을 저장한다. 같은 날의 반복 점검은 한 일별 기록을 갱신하며 새 날짜에는 새 기록을 만든다. 소스 오류를 완료율 0으로 기록하지 않는다. 기존 변경 이력의 날짜별 마지막 기록도 표시하지만, 기록이 없는 과거 날짜는 복원하지 않는다. 일별 이력은 90일 보존한다.

차트와 같은 일별 값을 표로 열어 키보드로 확인할 수 있다. 조회 실패와 기록 없음은 구분한다. 수집 시각과 원본 자료의 분석 기준일은 다르며, 문서 분석본을 연결한 프로젝트는 해당 자료의 갱신이 필요하다.

## 저장과 검증.

개인 설정·DB·로그·프로필과 로컬 운영 문서는 소스 커밋에서 제외한다. DB 원장은 재시작·실패 후에도 보존하며, 결과 불명확 상태를 지워서 재시도하지 않는다.

자동 테스트와 실제 Windows 검증을 구분한다. 테스트는 가짜 근태 실행기를 사용하며 운영 출퇴근이나 CheckOnly를 실행하지 않는다.
