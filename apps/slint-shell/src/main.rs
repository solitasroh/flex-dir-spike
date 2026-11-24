//! 시험 앱 B — Slint. `docs/shell-contract.md` 가 명세다.
//!
//! - 상태·OS 작업은 전부 `fd-host` (Tauri 판 A 와 같은 코드). 여기는 그리기·입력·OS 끌어다 놓기 연결만.
//! - 백엔드: `qt` 기능으로 빌드하면 Qt (Linux 시험본 — OS 끌어다 놓기는 Qt 백엔드에만 있다).
//!   기본 기능(winit+femtovg)은 로컬 컴파일·단위 시험용.

slint::include_modules!();

mod app;
mod dnd;
mod fonts;
// 소프트웨어 렌더러가 필요하다 (winit 기능에 들어 있다). 경로·한글 파일 이름 가정이 Linux 기준이다.
#[cfg(all(test, feature = "winit", target_os = "linux"))]
mod headless;
mod ime;
mod keys;
mod perf;
mod rows;

use std::sync::Arc;

// fontique 를 직접 의존하는 이유는 Cargo.toml 참고 (fontconfig dlopen).
#[cfg(all(unix, not(target_os = "macos")))]
use fontique as _;

/// `qt` 기능이면 Qt 백엔드를 고른다. 사용자가 SLINT_BACKEND 를 주면 그것을 따른다.
fn select_backend() -> String {
    let env = std::env::var("SLINT_BACKEND").ok();
    #[cfg(feature = "qt")]
    if env.is_none() {
        return match slint::BackendSelector::new().backend_name("qt".into()).select() {
            Ok(()) => "qt".into(),
            Err(e) => {
                eprintln!("[slint-shell] Qt 백엔드 선택 실패: {e}");
                format!("qt-select-failed:{e}")
            }
        };
    }
    match env {
        Some(v) => format!("env:{v}"),
        None => "default".into(),
    }
}

fn main() {
    fd_host::mark_process_start();
    let backend = select_backend();
    let cfg = fd_host::Config::from_env("slint");
    let ui = match AppWindow::new() {
        Ok(ui) => ui,
        Err(e) => {
            eprintln!("[slint-shell] 창을 만들 수 없습니다: {e}");
            std::process::exit(3);
        }
    };
    let host = fd_host::Host::new(cfg, Arc::new(app::notify));
    let features = if cfg!(feature = "qt") { "qt" } else { "winit" };
    host.log_event("backend", &[backend, format!("features={features}")]);
    // 입력기·세션 판정 근거 (AppRun 이 QT_IM_MODULE 을 바꿨으면 원래 값도)
    let env: Vec<String> = [
        "XDG_SESSION_TYPE",
        "WAYLAND_DISPLAY",
        "DISPLAY",
        "QT_QPA_PLATFORM",
        "QT_IM_MODULE",
        "FD_SPIKE_QT_IM_MODULE_ORIG",
        "GTK_IM_MODULE",
        "XMODIFIERS",
    ]
    .iter()
    .map(|k| format!("{k}={}", std::env::var(k).unwrap_or_default()))
    .collect();
    host.log_event("env", &env);
    match fonts::install_fallbacks() {
        Ok(s) => host.log_event("fonts", &["Roboto>Pretendard>system".into(), s]),
        Err(e) => host.log_event("fonts-error", &[e]),
    }
    let app = app::App::install(host.clone(), &ui);
    // View 가 알림을 받을 준비가 된 뒤 연다 (Host::start 문서)
    host.start();
    app.refresh_pane(0);
    app.refresh_pane(1);
    ui.invoke_focus_list(0);
    let run = ui.run();
    let code = match (run, app.exit_code()) {
        (_, Some(c)) => c,
        (Ok(()), None) => 0,
        (Err(e), None) => {
            eprintln!("[slint-shell] 이벤트 루프 오류: {e}");
            4
        }
    };
    // 열거·폴링 스레드가 남아 있어도 기다리지 않는다. 기록 파일은 줄마다 바로 쓰므로 잃을 것이 없다.
    std::process::exit(code);
}
