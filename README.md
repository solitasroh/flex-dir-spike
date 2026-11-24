# flex-dir-spike

[flex-dir](https://github.com/solitasroh/flex-dir) Rust 이전을 위한 **1주 GUI 스택 실험 저장소**입니다.

- 승인: 2026-11-25 (Soojang, "착수해")
- 후보: **A. Tauri 2 (wry)** · **B. Slint** (Linux는 Qt 백엔드)
- 같은 `fd-core` · `fd-viewmodel` 최소판, 같은 10만 항목 fixture, 같은 계측 지점(`ColdStart` · `WindowShown` · `FirstItem`)으로 두 후보를 비교합니다.
- 판정 순서: Linux(매일 사용) → Windows 11 → macOS(주말)

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
