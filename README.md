# flex-dir-spike

[flex-dir](https://github.com/solitasroh/flex-dir) Rust 이전을 위한 **1주 GUI 스택 실험 저장소**입니다.

- 승인: 2026-11-25 (Soojang, "착수해")
- 후보: **A. Tauri 2 (wry)** · **B. Slint** (Linux는 Qt 백엔드)
- 같은 `fd-core` · `fd-viewmodel` 최소판, 같은 10만 항목 fixture, 같은 계측 지점(`ColdStart` · `FirstItem` · `Frame:details` · `Frame:icons`, C#판 perf.log 와 같은 5칸)으로 두 후보를 비교합니다.
- 판정 순서: Linux(매일 사용) → Windows 11 → macOS(주말)

## 구조 (D0·D1)

| 경로 | 내용 |
|---|---|
| `crates/fd-core` | 경로 문법 두 벌(Windows·POSIX), 이름 정책(대소문자·NFC/NFD), 자연 정렬, 표시 형식, perf 줄(5칸) |
| `crates/fd-viewmodel` | `PaneVm::update(msg) -> Vec<Effect>` — async·I/O 없음. 선택·type-ahead(IME 조합 미리 찾기 포함)·이름 바꾸기·놓기·기하(Layout) |
| `crates/fd-portable` | 열거(배치)·복사/이동·연결 프로그램 열기·프로세스 트리 메모리 |
| `crates/fd-host` | **두 시험 앱 공용 앱 계층** — 탭·칸, 효과 실행, 자동 새로고침, `perf.log`·`events.log`. 스택 차이를 View 에만 남긴다 |
| `apps/tauri-shell` | 시험 앱 **A** (Tauri 2). Rust 다리 + 가상화 목록 JS. Linux 앱→OS 끌기는 `drag` 크레이트(GTK3) |
| `apps/slint-shell` | 시험 앱 **B** (Slint 1.19). Linux 시험본은 Qt 백엔드 (OS 끌어다 놓기가 Qt 백엔드에만 있다) |
| `docs/shell-contract.md` | 두 시험 앱이 똑같이 지킬 동작 (브라이언 확인표 부록 A + 엘레나 A-2) |
| `assets/` | 글꼴(원본 그대로)·아이콘 — 두 앱이 같은 파일을 쓴다 |
| `qa/` | 브라이언 `fd-check.sh`·`judge.py` 사본 (해시 고정). CI 가 이걸로 두 앱을 끝까지 돌린다 |

## CI (`.github/workflows/ci.yml`)

- `core` — 세 OS 에서 공용 크레이트 시험·clippy
- `tauri-linux` / `slint-linux` — Ubuntu 22.04 에서 AppImage (glibc 2.35 기준 — 그보다 새 배포판에서 돈다)
- `e2e-linux` — 두 AppImage 를 브라이언 `fd-check.sh prepare → perf A → perf B` 로 가상 화면(Xvfb)에서 돌린다. 숫자는 추세용, 판정은 대표님 기기
- `shells-other-os` — Windows·macOS 에서 두 시험 앱 빌드 (D4·주말 준비)

## 규칙

- **이 저장소의 코드는 버립니다.** 제품 코드가 아닙니다. 남기는 것은 숫자, 녹화, 체크리스트, 결정 문서(ADR 초안)입니다.
- 원본 `solitasroh/flex-dir`은 수정하지 않습니다.
- 결정 문서가 나오면 이 저장소는 보관(archive) 처리합니다.

## 일정

| 날 | 날짜 | OS | 내용 |
|---|---|---|---|
| D0·D1 | 11/25(수) | 공통·Linux | 최소 core/viewmodel, fixture, 3-OS CI, 시험 앱 2개, Linux 설치본 |
| D2 | 11/26(목) | Linux | 관문 L1 Wayland 끌어다 놓기 · L2 한글 입력 · L3 10만 항목 성능 (대표님 기기 30분) |
| D3 | 11/27(금) | Linux | 클립보드 · 우클릭 메뉴 · 트레이 · 창 크롬 · 업데이트 (대표님 기기 30분) |
| 주말 | 11/28–29 | macOS | 결정을 뒤집을 문제만 확인 (대표님 맥 30분) |
| D4 | 11/30(월) | Windows 11 | C#판 10만 기준선 · 셸 메뉴 · 클립보드 · 끌어다 놓기 · IME (대표님 PC 30분 × 1~2) |
| D5 | 12/1(화) 오전 | 공통 | 채점 → 결정 문서 |
