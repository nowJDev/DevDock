# DevDock CLI 사용법

[DevDock README](README.md) · [기본 명령](#기본-명령) · [문서와 진행 관리](#문서와-진행-관리) · [일일보고](#일일보고) · [NAS 게시 초안](#nas-게시-초안)

`devdock-cli`는 DevDock 앱과 같은 로컬 데이터를 사용합니다. 성공하면 표준 출력에 JSON을 반환합니다. 실패하면 표준 오류에 JSON을 반환하고 0이 아닌 종료 코드를 내보냅니다. 앱과 CLI는 기본 `%LOCALAPPDATA%\DevDock` 또는 동일한 `DEVDOCK_DATA_DIR`을 사용하세요.

소스에서 CLI를 빌드한 뒤 저장소 루트의 PowerShell에서 아래처럼 호출할 수 있습니다. 뒤에 나오는 `devdock-cli` 예시는 `& $cli`로도 실행할 수 있습니다.

```powershell
cargo build --manifest-path src-tauri/Cargo.toml --bin devdock-cli
$cli = (Resolve-Path '.\src-tauri\target\debug\devdock-cli.exe').Path
& $cli validate
```

## 기본 명령

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

실제 근태와 자동 시작은 Windows 앱에서 관리합니다. 인증 정보·개인 계정·원문 대화는 기존 인증 환경에 보관하세요.

## 문서와 진행 관리

새 문서 명령은 `documents`, `analysis`, `progress` 그룹을 사용한다. 긴 입력은 UTF-8 JSON 파일을 `--input FILE`로 전달한다. 수정에는 조회한 설정 revision 또는 프로젝트 revision이 필요하다. 알 수 없는 속성과 오래된 revision은 거절한다.

```text
devdock-cli documents config
devdock-cli documents source-upsert --input source.json
devdock-cli documents scan --input scan.json
devdock-cli documents list --input project.json
devdock-cli documents read --input excerpt.json
devdock-cli analysis create --input request.json
devdock-cli analysis claim --input claim.json
devdock-cli analysis heartbeat --input heartbeat.json
devdock-cli analysis submit --input result.json
devdock-cli analysis parse --input job.json
devdock-cli analysis preview --input job.json
devdock-cli analysis review --input review.json
devdock-cli progress get --input project.json
devdock-cli progress save-basis --input basis.json
devdock-cli progress task --input task-change.json
devdock-cli progress activate --input cutover.json
devdock-cli progress apply --input selection.json
devdock-cli progress undo --input undo.json
devdock-cli progress history --input range.json
```

등록한 프로젝트 ID가 `sample`이면 `project.json`은 `{"projectId":"sample"}`입니다. 모든 연결 출처를 다시 검색하려면 `scan.json`에 `{"sourceIds":[],"force":true}`를 전달합니다. `excerpt.json`은 `{"versionId":"VERSION_ID","start":1,"limit":200}`이며 최대 1000행까지 읽는다. 요청·결과 계약은 [프런트엔드 타입](src/knowledge/contracts.ts)과 [명령 정의](src-tauri/src/knowledge/commands.rs)에 정의한다. `knowledge COMMAND_JSON`은 같은 명령의 공통 접점이며 `knowledge-readonly COMMAND_JSON`은 조회만 허용한다.

분석 요청 ID를 현재 Codex에 전달하고 문서 버전·해시·원문 구간을 확인한 결과를 lease token과 함께 제출합니다. 앱에서 검토 후보를 확인하고 선택한 변경을 반영합니다. 과거 가져오기는 기본 `history` 모드이고 현재 작업 변경은 별도의 `current` 모드를 요구한다.

관리 전환한 프로젝트는 DB 기준으로 집계한다. 기존 task/status 쓰기는 전용 문서·진행 명령 사용을 안내하며 거절한다. 기존 기준 미기록 관측은 `legacy`로 조회하고 명시적 보관 미리보기·반영으로 90일 이후에도 보존할 수 있다. 새 진행 기준과 자동 합산하지 않는다.

## 일일보고

일일보고 화면에서 핸드오버와 프로젝트 문서를 날짜별로 찾고, 선택한 파일의 경로와 업무명을 담은 `$daily-work-report` 요청문을 복사합니다. 복사한 요청문은 Codex에 붙여넣습니다.

기존 요청 ID를 사용하는 연동은 아래 CLI 명령으로 조회하거나 처리할 수 있습니다. 앱 메모리에 있는 저장 전 초안은 앱을 종료하면 만료됩니다.

PowerShell에서 보고 요청을 조회하는 예시입니다. 위에서 설정한 `$cli`를 사용합니다.

```powershell
'{"requestId":"REQUEST_ID"}' | & $cli reports context --stdin
```

| 명령 | 표준 입력 JSON의 필드 |
| --- | --- |
| `reports context --stdin` | `requestId` |
| `reports claim --stdin` | `jobId`, `owner` |
| `reports read-source --stdin` | `requestId`, `index`, `leaseToken` |
| `reports submit --stdin` | `requestId`, `leaseToken`, `draft`, `proofs` |

선택한 파일마다 원문 조회 결과의 `proof`를 모아 `proofs` 배열로 제출합니다. `draft` 객체는 `body`, `needsMoreSources`, `grounding`을 받습니다. `grounding`의 각 항목에는 `reportLine`, `sourceIndex`, `startLine`, `endLine`, `classification`이 필요합니다. 분류 값은 `design`, `implementation`, `automatedTest`, `build`, `remotePush`, `hardwareAcceptance`, `unfinished`, `other`입니다.

## NAS 게시 초안

1. 앱에서 **문서 업데이트 → Codex 게시 초안 요청**을 실행합니다.
2. 현재 Codex에서 `analysis get`과 `analysis claim`으로 게시 대상과 원문을 확인합니다. 두 명령에 `jobId`를 전달하며, `claim`에는 `owner`도 전달합니다.
3. `documents publish-submit --input draft.json`으로 초안을 제출합니다. `draft.json`에는 `jobId`, `leaseToken`, `result`가 필요하고, `result.patches`에는 근거가 연결된 변경을 담습니다.
4. 앱에서 변경 전후와 선택한 파일을 확인한 뒤 게시합니다. 앱의 게시 기록에서 파일별 결과를 확인할 수 있습니다.
