//! fd-host — 두 시험 앱(Tauri·Slint)이 **똑같이** 쓰는 앱 계층.
//!
//! 탭·칸 상태, ViewModel 효과 실행(열거·이름 바꾸기·복사/이동), 자동 새로고침, 기록 파일(perf.log·events.log)
//! 을 여기서 한다. 시험 앱(View)은 이 API 만 부르고, 그리는 일과 OS 끌어다 놓기 연결만 한다.
//! 스택 차이를 View 에만 남겨 두어야 두 후보 비교가 공정하다 (tech.md §4.1).
//!
//! 브라이언 `spike-d2-linux-sheet.md` 부록 A 계약:
//! - 인자: 폴더 1~2개 (왼쪽, 오른쪽 칸)
//! - `FD_SPIKE_OUT` 에 `perf.log`(5칸) · `events.log` 를 쓴다. `FD_SPIKE_PERF_AUTO=1` 일 때만 perf.log 를 쓴다
//!   (손 확인 세션의 열기 기록이 섞이면 L3 판정이 오염된다).
//! - `FD_SPIKE_CANDIDATE` = A|B → 창 제목.
mod config;
mod host;
mod logs;

pub use config::Config;
pub use host::{home_dir, parse_mode, Host, Notify, PaneSnap, Row, TabInfo, UiEvent};
pub use logs::{now_local_text, EventLog, PerfLog};

pub use fd_core::perf::Point;
pub use fd_viewmodel::{Layout, PaneMsg, SortKey, Transfer, ViewMode};

use std::sync::OnceLock;
use std::time::Instant;

static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// `main` 첫 줄에서 부른다. ColdStart 기준점.
pub fn mark_process_start() {
    let _ = PROCESS_START.set(Instant::now());
}

/// 프로세스 시작부터 지금까지 (ms).
pub fn since_process_start_ms() -> f64 {
    PROCESS_START.get().map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0)
}

/// 자동 측정 시나리오 상수 — 두 앱이 같은 값을 쓴다.
pub mod perf_plan {
    /// 큰 폴더를 다시 여는 횟수 (FirstItem 10줄).
    pub const OPEN_REPEATS: usize = 10;
    /// 보기마다 자동 스크롤 시간.
    pub const SCROLL_SECONDS: f64 = 10.0;
    /// 스크롤 속도 (논리 px/s). 자세히 보기 24px 줄 기준 초당 100줄 — 손으로 빨리 훑는 속도.
    pub const SCROLL_PX_PER_S: f64 = 2400.0;
    /// 스크롤 시작 전 안정화 대기.
    pub const SETTLE_MS: u64 = 500;
}
