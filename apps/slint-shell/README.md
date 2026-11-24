# 시험 앱 B — Slint (`fd-slint-shell`, 바이너리 `flex-dir-spike-slint`)

명세는 [`docs/shell-contract.md`](../../docs/shell-contract.md). 상태·OS 작업은 전부 `fd-host`(A 와 같은 코드)이고,
여기에는 그리기·입력·OS 끌어다 놓기 연결만 있다. Slint 1.19.1 (crates.io 최신 안정판, 2026-11-24).

## 빌드

| 용도 | 명령 | 백엔드 |
|---|---|---|
| 로컬 컴파일·단위 시험 | `cargo build -p fd-slint-shell` (기본 기능 `winit`) | winit + femtovg(OpenGL) / `SLINT_BACKEND=winit-software` |
| **목요일 시험본 (CI)** | `QMAKE=/usr/lib/qt6/bin/qmake cargo build --release -p fd-slint-shell --no-default-features --features qt` | **Qt 6** (런타임에 `BackendSelector` 로 qt 를 고른다. `SLINT_BACKEND` 를 주면 그것을 따른다) |
| AppImage (CI) | `apps/slint-shell/packaging/build-appimage.sh dist` (`SMOKE=1` 이면 offscreen 으로 자동 측정까지 돌려 봄) | 결과 `dist/flex-dir-spike-slint-x86_64.AppImage` |

- **왜 Linux 는 Qt 인가**: Slint 1.19 에서 OS 수준 끌어다 놓기(다른 앱에서 놓기 + 앱 밖으로 `QDrag`)는 Qt 백엔드에만 있다
  (`i-slint-backend-qt` `qt_window.rs` `start_drag`·`dropEvent`). winit 백엔드 1.19 에는 OS 끌어다 놓기가 없다 → 기본(winit) 빌드는 시험본이 아니다.
- 표준 위젯 스타일은 `fluent` 로 고정(`build.rs`) — qt 빌드가 Qt 네이티브 스타일로 바뀌어 화면이 달라지지 않게.
- `fontique` 를 직접 의존하는 이유: fontconfig 를 링크 대신 `dlopen("libfontconfig.so.1")` (이 서버엔 개발 패키지가 없고, AppImage 는 시스템 것을 쓴다).
- CI 예시 (ubuntu-22.04):
  ```yaml
  - run: sudo apt-get update && sudo apt-get install -y qt6-base-dev qt6-wayland libqt6svg6 librsvg2-bin
  - run: cargo build --release -p fd-slint-shell --no-default-features --features qt
    env: { QMAKE: /usr/lib/qt6/bin/qmake }
  - run: SMOKE=1 apps/slint-shell/packaging/build-appimage.sh dist
  ```
  Qt 를 못 찾으면 `i-slint-backend-qt` 가 빈 껍데기로 **경고만 내고** 빌드된다 → 스크립트가 `ldd … libQt6Widgets` 로 막는다.

## 계약 대응

| 계약 | 구현 (파일) |
|---|---|
| §1 인자·환경변수·탭 2개 | `Config::from_env("slint")`, `host.start()` 는 UI·모델 연결 뒤 (`main.rs`). 창 제목 `config().window_title()` |
| §2 자동 측정 | 순수 상태 기계 `perf.rs` (`PerfPlan`) + 연결 `app.rs` `exec`. Up → 다 읽음 → Navigate(t0) → `UiEvent::FirstItem` 뒤 **그려진 프레임** t1 → `FirstItem`=t1−t0 (정확히 10줄). 자세히 → `SETTLE_MS` → 10초 동안 오프셋 = 경과×2400 → 프레임 간격(첫 간격 버림) → `Frame:details`, 큰 아이콘 같은 방식 → `Frame:icons` → `quit_event_loop` → **exit 0**. 290초 감시(넘으면 기록 남기고 exit 1). `ColdStart` = 첫 그려진 프레임 직후 `cold_start_painted()` |
| §2 프레임 시각 | ① 렌더 알림 `set_rendering_notifier(AfterRendering)` 이 되면 그것(winit/femtovg). ② **Qt 백엔드는 렌더 알림이 없다**(`Unsupported`) → **그리기 탐침**: `app.slint` 맨 끝(가장 위)의 1px 투명 사각형 배경이 `animation-tick()` 에 묶여 있어, 그 프레임을 *그리는 도중* 바인딩이 다시 계산되며 Rust 가 시각을 잰다. 스크롤 진행은 `changed tick` (Qt 에서는 그리기 끝 `timer_event`). 어떤 방식인지 `events.log` 의 `perf-method` 에 남는다 |
| §3 화면 | `ui/pane.slint`: 탭 줄(+`+`) · 도구 줄(위로·새로 고침·자세히/큰 아이콘) · 주소줄 · 머리글(이름/수정한 날짜/크기, 누르면 `SetSort`) · 목록 · 상태줄. 줄 24, 타일 116×140 간격 8 (VM `Layout` 과 같은 식). 뷰포트 크기만 `SetViewport` 로 민다(세로 스크롤바 14px 는 목록 위에 겹치므로 뺀다). 포커스된 칸(위 2px 강조선·탭 밑줄), 선택 줄, 포커스 줄 테두리, 놓기 대상 폴더 줄 |
| §3 10만 항목 | Slint `ListView`(보이는 줄만 만든다) + 직접 만든 `Model`(`rows.rs`): 물은 위치 주변 256 항목만 `Host::rows` 로 끌어와 든다. 구조 변경(폴더·정렬·개수·보기·열 수·탭) = `reset`, 선택·포커스만 바뀌면 창을 다시 읽어 **달라진 줄만** `row_changed`. 큰 아이콘 = 합성 행 모델 |
| §3 글꼴 | `ui/app.slint` 가 `assets/fonts` 원본 3개를 `import`(바이너리에 그대로). `default-font-family: "Roboto"`, 13px. 한글 폴백: Slint 1.19 는 **Qt 백엔드도 parley+fontique 로 글자를 배치**한다(`qt_window.rs` FontCache) → `fonts.rs` 가 fontique 글꼴 모음의 Hang·Hira·Kana 폴백 맨 앞에 Pretendard 를 끼운다(그 뒤는 시스템 폴백 그대로 = 한자·이모지·아랍어). 두 백엔드에 **같은 한 가지 방법**. fontconfig 설정 파일은 쓰지 않는다(Slint 가 Qt 글꼴 폴백을 거치지 않으므로 효과가 없다). 결과는 `events.log` `fonts` |
| §4 키보드 | 순수 표 `keys.rs`. 목록 포커스 받이는 **보이지 않는 `TextInput`**(IME 가 켜진 채 한글 조합·확정이 들어오게). Ctrl+A/L 등은 `key-pressed` 에서 먼저 받아 입력칸에 글자가 안 들어간다. `shortcut` 기록 |
| §5 한글 | `ime.rs` `EnterGate`: 조합이 비어 있으면 Enter 즉시 실행, 조합 중이면 예약 → `preedit-text` 가 비는 순간(확정) **최종 값으로 한 번**, 300ms 안에 확정이 안 오면 값+조합 글자로 한 번. 실행 뒤 150ms 안의 Enter 는 목록 Activate 로 안 샌다. Esc 는 조합 중이어도 취소. 이름 바꾸기 중 다른 곳 클릭 = 입력칸 포커스 잃음 = 취소. 편집이 끝난 뒤 목록 type-ahead 는 300ms(조합 중에 끝났으면 1000ms) 동안 글자를 받지 않는다(잔여 조합 글자 차단). type-ahead: 확정 → `type_ahead(…, true)`, 조합 중 → `(…, false)` |
| §6 끌어다 놓기 | `dnd.rs` + `pane.slint` `ItemHit`. 줄·타일마다 `DragArea`: **누르는 순간(포인터 이벤트 수정키)의 Shift → move 만, 아니면 copy 만** 허용(Qt 는 수정키 없을 때 허용 목록의 첫째를 기본으로 `QDrag::exec` 에 넘긴다). 짐 = 선택 전체(같은 판이면 재사용) 또는 누른 항목. 놓기: 목록 위 `DropArea` → `host.hit(콘텐츠 좌표)` → 폴더 줄이면 강조, `drop_paths(…, "os"|"internal")`. OS 가 move 를 제안하면 이동, 그 밖은 복사. 앱 안 끌기도 Qt 에서는 OS 끌기로 돌아오므로 방금 끈 경로와 같으면 `internal`. 끝나면 `drag_out_finished(…, "dropped"|"cancel")`(앱 안 놓기는 제외). 탭: 글자 짐 `fd-spike-tab:칸:번호` → 다른 칸 탭 줄 `DropArea` → `move_tab`. 원시 정보 `dnd-raw`(drag-start allowed/shift, hover proposed, drop proposed/chosen, drag-finished action) |
| §7 기록 | `shortcut`·`dnd-raw` 외에 `backend`·`env`(QT_IM_MODULE·세션)·`fonts`·`perf-method`·`edit-end`·`ime-guard`·`exit` |

## 확인한 것 (이 서버 — 화면 없음, Qt 없음, 2코어)

- `cargo build -p fd-slint-shell` ✅ · `cargo clippy -p fd-slint-shell --all-targets -- -D warnings` ✅ · `cargo test --workspace --exclude fd-tauri-shell` ✅ (이 앱 22개 포함).
- `cargo check`/`clippy --no-default-features --features qt` ✅ — **단, 이 서버에는 Qt 가 없어 `i-slint-backend-qt` 는 빈 껍데기(no_qt)로 컴파일됐다.** 우리 Rust 코드가 qt 설정에서 컴파일된다는 뜻일 뿐, Qt C++ 부분은 CI 에서 처음 컴파일된다.
- **화면 없는 통합 시험** (`src/headless.rs`, 소프트웨어 렌더러 창 + 직접 만든 이벤트 루프 — 실제 `AppWindow`·`App`·`Host`):
  - 키보드: Ctrl+A 전체 선택 · `가방.txt` F2 → `회의` → Enter 한 번 → `회의.txt` (뒤따른 Enter 는 `shortcut-ignored`) · F2 → `취소` → Esc → 그대로 · type-ahead `g` → `ghost.txt` · Ctrl+L → End → `/한글폴더` → Enter → 이동 · Tab → 오른쪽 칸.
    *글자는 키 이벤트로 넣었다 — IME 조합 이벤트(preedit)는 공개 API 로 흉내 낼 수 없어 `EnterGate` 단위 시험으로만 확인.*
  - 자동 측정: 3,000 항목 폴더로 계약 §2 전체 → perf.log `FirstItem` 정확히 10줄, `ColdStart` 1줄, `Frame:details`·`Frame:icons` 줄, 모든 줄 5칸, exit 0, `perf-method` = 그리기 탐침(= Qt 와 같은 경로). 숫자는 디버그·소프트웨어 렌더라 판정용 아님.
- 글꼴 폴백 설치 성공(`fonts Hang=Pretendard+1,…`) — 화면으로 본 것은 아니다.
- `build-appimage.sh`·`AppRun` 은 문법 검사(`bash -n`/`sh -n`)만. linuxdeploy 로 실제로 만들어 보지 않았다.

## 확인 못 한 것 (CI·대표님 기기에서 처음)

Qt 백엔드 실제 컴파일 · 창이 실제로 뜨는지 · Wayland/X11 · Nautilus 와 끌어다 놓기 · ibus/fcitx5 실제 조합 이벤트 · AppImage 생성·실행 · 글꼴 모양(D1~D3) · 10만 항목 실제 프레임.

## 목요일 위험 (솔직하게)

1. **Qt 백엔드는 GPU 를 안 쓴다.** QWidget + QPainter 래스터로 그린다. 10만 스크롤 프레임은 CPU 와 창 크기(HiDPI 배율)에 달렸다. 분수 배율에서 글자가 흐릴 수 있다(D1).
2. **프레임 간격의 뜻이 세션마다 다를 수 있다.** Qt 위젯은 X11 에서 vsync 에 묶이지 않을 수 있어 간격이 16.7ms 보다 짧게 나올 수 있고, Wayland 는 합성기 프레임 콜백에 묶여 16.67ms 근처에 붙는다(BORDER 위험). 둘 다 실기기에서 처음 본다. 탐침 시각은 "그리는 중"이라 화면 반영(버퍼 넘김) 직전이다 — FirstItem 이 아주 약간 낙관적.
3. **입력기 플러그인.** 번들 Qt 는 빌드 기계(22.04)의 Qt **6.2** 다. ① 22.04 에는 fcitx5 Qt6 플러그인이 없다 → `AppRun` 이 `QT_IM_MODULE=fcitx/fcitx5` 를 `ibus` 로 바꾼다(fcitx5 의 ibus 호환 프런트엔드에 기대는 것 — **검증 안 됨**. fcitx5 에서 ibusfrontend 애드온이 꺼져 있으면 한글이 안 들어간다 → L2 ❌ 이지만 포장 문제다. 24.04 빌드 + `fcitx5-frontend-qt6` 로 고칠 수 있다). ② Qt 6.2 는 Wayland text-input-v3 를 모른다 → `QT_IM_MODULE` 이 비어 있는 GNOME Wayland 에서는 입력기 데몬이 보이면 `ibus` 로 붙인다. 바꾼 값과 원래 값은 `events.log` `env` 에 남는다.
4. **IME 잔여 글자 차단은 시간 기반이다.** Slint 에는 IME 상태를 비우는(reset) API 가 없다. Esc·다른 곳 클릭 뒤 입력기가 조합 글자를 1초 넘게 들고 있다가 다음 키에 확정하면 목록 type-ahead 로 갈 수 있다(한-2/한-3). ibus-hangul 은 Esc 때 먼저 확정해 이름 칸으로 넣는 것으로 알지만 실기기 확인 전이다.
5. **조합 중 Enter 를 입력기가 삼키면**(확정만 하고 Enter 를 안 넘기면) Enter 2번이 된다 — Minor 로 기록될 항목.
6. **끌기 도중 Shift 를 바꿔도 동작이 안 바뀐다.** 시작 순간 하나만 허용하므로(끌-3 을 위한 의도) 커서도 그대로다(끌-8 에서 "이동 표시로 안 바뀜" 메모가 나올 수 있다). 누른 뒤 끌기 문턱을 넘기 전에 누른 Shift 까지는 반영되게 했다(포인터 이동 이벤트의 수정키 — 실기기 미확인).
7. **OS → 앱 Shift(끌-2)는 Qt 가 넘겨주는 `proposedAction` 에 달렸다.** Wayland 에서는 합성기 협상 결과다. `dnd-raw hover/drop proposed=` 로 남으니 ❌ 가 나오면 원인을 가를 수 있다.
8. **물리 키 코드가 없다.** Slint `KeyEvent` 는 논리 키만 준다. 한국어 xkb 배열(us/kr)에서는 Ctrl+A 가 `a` 로 오므로 문제없지만, 라틴이 아닌 배열이면 단축키가 죽을 수 있다.
9. 탭 끌기 짐이 글자라서 탭을 파일 관리자에 놓으면 텍스트 조각 파일이 생길 수 있다(확인표 밖 동작).
10. 끌기 그림(drag image)은 없다 — 커서만 바뀐다. 창 장식은 Qt Wayland 의 기본 클라이언트 장식(D4 는 금요일 항목).
11. ColdStart 는 첫 프레임을 그리는 중의 시각. 앱 시작 전 AppImage 풀기(`APPIMAGE_EXTRACT_AND_RUN=1`) 시간은 프로세스 시작 전이라 들어가지 않는다(A 도 같다).

## 파일

- `src/main.rs` 진입·백엔드 선택·기록 · `src/app.rs` UI 스레드 접착(Host 알림 → 이벤트 루프, Bridge → PaneMsg)
- 순수 모듈(단위 시험): `rows.rs`(가상화 창) · `keys.rs`(키 표) · `ime.rs`(Enter·잔여 글자) · `dnd.rs`(끌기 규칙) · `perf.rs`(자동 측정 순서)
- `fonts.rs` 한글 폴백 · `headless.rs` 화면 없는 통합 시험(시험 전용)
- `ui/app.slint` 창·글꼴·탐침 · `ui/pane.slint` 한 칸 · `ui/theme.slint` 색·구조체·Bridge
- `packaging/build-appimage.sh` · `packaging/AppRun`
