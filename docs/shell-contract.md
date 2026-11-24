# 시험 앱 계약 — A(Tauri)·B(Slint) 공통

두 시험 앱은 **같은 `fd-host`** 를 쓰고 View 만 다르다. 이 문서의 동작이 다르면 비교가 무너진다.
출처: 브라이언 `planning/spike-d2-linux-sheet.md` 부록 A, 엘레나 `planning/spike-d2-design-items.md` A-2.

## 1. 실행 계약 (fd-check.sh 가 전제)

- 인자: 폴더 1~2개. 첫째 = 왼쪽 칸, 둘째 = 오른쪽 칸. (`fd_host::Config::from_env`)
- 환경변수: `FD_SPIKE_OUT`(기록 폴더) · `FD_SPIKE_CANDIDATE`(A|B → 창 제목 `Config::window_title()`) · `FD_SPIKE_PERF_AUTO=1`
- 칸마다 탭 2개로 시작한다 (`Host::start`).
- `perf.log` 는 **자동 측정일 때만** 쓴다 (`PerfLog` 가 막는다). 손 확인 세션의 폴더 열기가 섞이면 L3 판정이 오염된다.

## 2. 자동 측정 (`FD_SPIKE_PERF_AUTO=1`) — 5분 안에 끝나고 exit 0

상수는 `fd_host::perf_plan`.

1. 왼쪽 칸 첫 탭이 인자 폴더(10만)를 다 읽을 때까지 기다린다 (`snapshot(0).loading == false`).
2. `OPEN_REPEATS`(10)회: 상위 폴더로 가서(`PaneMsg::Up`) 다 읽을 때까지 기다린 뒤, 인자 폴더로 `PaneMsg::Navigate` → 그 순간부터
   **첫 항목이 화면에 그려진 프레임까지** 의 시간을 `Point::FirstItem` 으로 남긴다. 다 읽을 때까지 기다린 뒤 다음 회.
   - `UiEvent::FirstItem(pane)` 이 오면 그다음 그려진 프레임 시각을 잰다 (웹뷰: rAF 두 번, Slint: 렌더 알림/애니메이션 틱).
   - 시작 시 첫 열기는 세지 않는다 (ColdStart 에 들어간다). 정확히 10줄.
3. 자세히 보기, 맨 위에서 `SETTLE_MS` 쉰 뒤 `SCROLL_SECONDS`(10초) 동안 `SCROLL_PX_PER_S`(2400 논리px/s) 로 아래로 스크롤.
   **매 프레임 간격(ms, 소수)** 을 모아 `host.perf_frames(Point::FrameDetails, &v)` — 한 프레임에 한 줄. 첫 간격(기준 없음)은 버린다.
   - 스크롤 위치는 "시작 후 경과 시간 × 속도" 로 정한다 (프레임이 밀려도 속도가 같게).
4. 큰 아이콘 보기로 바꿔 같은 방식 → `Point::FrameIcons`.
5. 프로세스 종료 코드 0.
- `ColdStart`: 첫 화면을 그린 직후 `host.cold_start_painted()` (두 모드 모두 부른다; perf.log 에는 자동 측정일 때만 남는다).

## 3. 화면

- 2분할 (왼쪽·오른쪽 칸). 칸마다: 탭 줄(탭 + `+`) · 도구 줄(위로 · 새로 고침 · 자세히/큰 아이콘) · 주소줄 · 목록 · 상태줄.
- 자세히 보기: 열 = 이름 · 수정한 날짜 · 크기. 머리글 클릭 = 정렬(`PaneMsg::SetSort`, 같은 열 다시 누르면 방향 전환). 줄 높이 24 (VM 상수).
- 큰 아이콘: 타일 116×140, 간격 8 (VM `Layout`). 열 수·행 수·좌표 판정은 **VM 이 한다** — View 는 뷰포트 크기만 `SetViewport` 로 민다.
- 글꼴: **Roboto → Pretendard → 시스템 기본** (`assets/fonts`, 원본 그대로). 본문 13px(논리), 굵게는 Pretendard SemiBold.
- 아이콘: `assets/icons/folder.svg` · `file.svg`.
- 포커스된 칸 표시, 선택 줄 강조, 포커스 줄 테두리, 놓기 대상 폴더 줄 강조(끌-1).

## 4. 키보드 (목록에 포커스가 있을 때)

| 키 | 동작 | events.log |
|---|---|---|
| ↑↓ / PgUp·PgDn / Home·End (Shift 로 범위) | `MoveFocus` (큰 아이콘 ↑↓ = ±열 수, ←→ = ±1) | — |
| Enter | `Activate(focus)` — 폴더면 들어가고 파일이면 연다 | `shortcut Enter activate` |
| Backspace · Alt+↑ | `Up` | `shortcut` |
| F2 | `BeginRename` | `shortcut F2 rename` |
| F5 | `Refresh` | `shortcut` |
| Ctrl+A | `SelectAll` | `shortcut Ctrl+A select-all` |
| Ctrl+L | 주소줄로 포커스, 전체 선택 | `shortcut Ctrl+L address` |
| Esc | 이름 바꾸기·주소줄 편집 취소, 목록으로 | `shortcut Esc …` |
| Tab | 다른 칸으로 | — |
| 글자 | type-ahead: 확정 글자 `host.type_ahead(pane, t, true, now)`, IME 조합 중 글자 `host.type_ahead(pane, t, false, now)` | host 가 남긴다 |

- 단축키 판정은 **물리 키(code)** 로 한다. 한글 입력 상태에서도 Ctrl+A·Ctrl+L 이 그대로 동작하고 `ㅁ`·`ㅣ` 가 어디에도 들어가면 안 된다 (한-5).
- Ctrl 조합 키 이벤트는 IME 로 넘기지 않는다.

## 5. 한글 입력 (L2) — 엘레나 design §3-5

- 이름 바꾸기 입력칸 · 주소줄: **Enter 한 번에 조합 확정 + 동작이 정확히 한 번**. `회의` 조합 중 Enter → `회의.txt` (빠짐 `회.txt`, 겹침 `회의의.txt` 금지).
  - 조합 중 Enter 가 오면 "확정 후 실행" 을 예약하고, 조합 확정 이벤트 뒤 입력칸의 **최종 값**으로 한 번만 실행한다.
  - 같은 Enter 가 목록의 Activate 로 새지 않게 한다 (확정 직후 150ms 안의 Enter 는 목록에서 무시).
- Esc: 조합 중이어도 이름 바꾸기 취소. 조합 글자(`소`, `ㅅ`)가 목록 type-ahead 나 다른 곳에 남으면 안 된다 (한-2).
- 이름 바꾸기 중 다른 곳 클릭: **취소** (한-3, 엘레나 기준). 조합 글자가 새로 포커스 받은 칸으로 새면 안 된다.
- 목록 type-ahead 는 한글 상태에서 `한글` 을 치면 `한글…` 항목으로 간다. **`ghost.txt` 로 가면 ❌** (영문 키가 그대로 샌 것).
  조합 중 글자로도 미리 움직인다 (`TypeAheadPreview`, 자모 접두 비교).

## 6. 끌어다 놓기 (L1) — 기본 복사, Shift 이동 (엘레나 design §3-4)

- OS → 앱: 폴더 줄 위에 놓으면 그 폴더로, 빈 곳·파일 위면 현재 폴더로 (`host.drop_paths(pane, host.hit(..), paths, mode, "os")`).
  Shift(또는 OS 가 알려 준 move) 면 이동. 놓기 직전 대상 폴더 줄 강조.
- 앱 → OS: 끌기 시작 순간 Shift 가 눌려 있으면 **move 만**, 아니면 **copy 만** 제안한다.
  copy 와 move 를 함께 제안하면 Nautilus 가 같은 디스크에서 기본 이동을 고른다 → 끌-3 ❌.
  끝나면 `host.drag_out_finished(pane, &paths, mode, "dropped"|"cancel"|"error:…")`.
- 앱 안 칸 → 칸: 같은 규칙 (`origin = "internal"`).
- 탭을 다른 칸 탭 줄로 끌면 `host.move_tab(from, idx, to, insert_at)` (끌-7).
- 이동 뒤 목록은 `fd-host` 의 1초 폴링 + 끌기 끝 후 재읽기로 갱신된다.

## 7. 기록

- `events.log` 는 `fd-host` 가 대부분 남긴다 (drop · drag-out · rename · type-ahead · tab-move · open). View 는 `shortcut` 만 `host.log_event("shortcut", &[키, 동작])` 로 남긴다.
- View 가 알게 된 OS 원시 정보(예: GTK 가 알려 준 끌기 action)는 `host.log_event("dnd-raw", …)` 로 남긴다 — 판정 근거.
