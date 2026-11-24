//! 화면 없는 서버에서 **진짜 UI·App·Host 연결**을 돌려 보는 통합 시험 (시험 전용).
//!
//! Slint 소프트웨어 렌더러 창(MinimalSoftwareWindow)과 직접 만든 이벤트 루프로 `AppWindow` 를 띄운다.
//! - 자동 측정: 계약 §2 순서 전체(FirstItem 10줄 → Frame:details → Frame:icons → exit 0)를 실제로 돌린다.
//!   소프트웨어 렌더러는 렌더 알림이 없으므로 **Qt 백엔드와 같은 "그리기 탐침" 경로**를 탄다.
//! - 키보드: Ctrl+A · F2 → 글자 → Enter 한 번 · F2 → Esc · type-ahead · Ctrl+L → 주소 → Enter.
//!
//! 숫자(프레임 ms)는 판정용이 아니다 (디버그 빌드·소프트웨어 렌더·2코어 서버). 순서와 기록 모양만 본다.
//! Slint 플랫폼은 프로세스당 한 번만 정할 수 있어서 두 장면을 한 시험 함수에서 차례로 돈다.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType, Rgb565Pixel};
use slint::platform::{Key, Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, PhysicalSize, PlatformError};

use crate::app::{self, App};
use crate::AppWindow;

const W: usize = 1280;
const H: usize = 800;

type Job = Box<dyn FnOnce() + Send>;

#[derive(Clone, Default)]
struct Queue {
    jobs: Arc<Mutex<VecDeque<Job>>>,
    quit: Arc<AtomicBool>,
}

struct Proxy(Queue);

impl slint::platform::EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> {
        self.0.quit.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn invoke_from_event_loop(&self, event: Job) -> Result<(), slint::EventLoopError> {
        self.0.jobs.lock().unwrap().push_back(event);
        Ok(())
    }
}

struct Headless {
    queue: Queue,
    limit: Duration,
}

thread_local! {
    static WINDOW: std::cell::RefCell<Option<Rc<MinimalSoftwareWindow>>> = const { std::cell::RefCell::new(None) };
    static QUEUE: std::cell::RefCell<Option<Queue>> = const { std::cell::RefCell::new(None) };
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        // 장면마다 새 창 (AppWindow 하나 = 창 하나)
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        window.set_size(PhysicalSize::new(W as u32, H as u32));
        WINDOW.with(|w| *w.borrow_mut() = Some(window.clone()));
        Ok(window)
    }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn slint::platform::EventLoopProxy>> {
        Some(Box::new(Proxy(self.queue.clone())))
    }
    fn run_event_loop(&self) -> Result<(), PlatformError> {
        let start = Instant::now();
        self.queue.quit.store(false, Ordering::SeqCst);
        while !self.queue.quit.load(Ordering::SeqCst) {
            pump_once();
            if start.elapsed() > self.limit {
                return Err(PlatformError::Other("headless: 시간 초과".into()));
            }
        }
        Ok(())
    }
}

/// 이벤트 루프 한 바퀴: 타이머·애니메이션 → 다른 스레드에서 온 일 → 필요하면 그리기.
fn pump_once() {
    slint::platform::update_timers_and_animations();
    let jobs: Vec<Job> = QUEUE.with(|q| q.borrow().as_ref().map(|q| q.jobs.lock().unwrap().drain(..).collect()).unwrap_or_default());
    let idle = jobs.is_empty();
    for j in jobs {
        j();
    }
    let drew = WINDOW.with(|w| {
        let w = w.borrow();
        let w = w.as_ref().unwrap();
        if w.has_active_animations() {
            w.request_redraw();
        }
        let mut buf = vec![Rgb565Pixel::default(); W * H];
        w.draw_if_needed(|r| {
            r.render(&mut buf, W);
        })
    });
    if idle && !drew {
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn pump_for(d: Duration) {
    let end = Instant::now() + d;
    while Instant::now() < end {
        pump_once();
    }
}

fn pump_until(limit: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        pump_once();
        if cond() {
            return true;
        }
    }
    false
}

fn press(ui: &AppWindow, text: impl Into<slint::SharedString>) {
    let t: slint::SharedString = text.into();
    ui.window().dispatch_event(WindowEvent::KeyPressed { text: t.clone() });
    ui.window().dispatch_event(WindowEvent::KeyReleased { text: t });
    pump_for(Duration::from_millis(30));
}

fn chord(ui: &AppWindow, modifier: Key, text: &str) {
    let m: slint::SharedString = modifier.into();
    ui.window().dispatch_event(WindowEvent::KeyPressed { text: m.clone() });
    press(ui, text);
    ui.window().dispatch_event(WindowEvent::KeyReleased { text: m });
    pump_for(Duration::from_millis(30));
}

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("TMPDIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let d = base.join(format!("fd-slint-headless-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn lines(p: &Path, needle: &str) -> Vec<String> {
    std::fs::read_to_string(p).unwrap_or_default().lines().filter(|l| l.contains(needle)).map(String::from).collect()
}

fn start_app(cfg: fd_host::Config) -> (AppWindow, Rc<App>, fd_host::Host) {
    let ui = AppWindow::new().unwrap();
    let host = fd_host::Host::new(cfg, Arc::new(app::notify));
    crate::fonts::install_fallbacks().map(|s| host.log_event("fonts", &[s])).unwrap_or_else(|e| host.log_event("fonts-error", &[e]));
    let a = App::install(host.clone(), &ui);
    host.start();
    a.refresh_pane(0);
    a.refresh_pane(1);
    ui.show().unwrap();
    ui.invoke_focus_list(0);
    (ui, a, host)
}

fn auto_perf_scene(root: &Path) {
    let big = root.join("big");
    std::fs::create_dir_all(&big).unwrap();
    for i in 0..3000 {
        std::fs::File::create(big.join(format!("img{i}.jpg"))).unwrap();
    }
    let out = root.join("out-perf");
    let cfg = fd_host::Config {
        candidate: "B".into(),
        stack: "slint",
        out_dir: out.clone(),
        perf_auto: true,
        folders: vec![big.display().to_string()],
    };
    let (ui, a, _host) = start_app(cfg);
    // 자동 측정은 App 이 스스로 돈다 → 플랫폼 이벤트 루프로 끝까지 (exit 로 quit)
    let run = ui.run();
    assert!(run.is_ok(), "이벤트 루프: {run:?}");
    assert_eq!(a.exit_code(), Some(0), "exit 0");
    let perf = out.join("perf.log");
    let events = out.join("events.log");
    assert_eq!(lines(&perf, "\tFirstItem\t").len(), 10, "FirstItem 정확히 10줄");
    assert_eq!(lines(&perf, "\tColdStart\t").len(), 1);
    let det = lines(&perf, "\tFrame:details\t").len();
    let ico = lines(&perf, "\tFrame:icons\t").len();
    eprintln!("[headless] Frame:details {det}줄, Frame:icons {ico}줄");
    assert!(det > 5 && ico > 5, "프레임 기록 ({det}, {ico})");
    assert!(std::fs::read_to_string(&perf).unwrap().lines().all(|l| l.split('\t').count() == 5), "5칸");
    assert!(lines(&events, "perf-method").iter().any(|l| l.contains("render-probe")), "렌더 알림 없는 경로 = 탐침");
    drop(ui);
}

fn keyboard_scene(root: &Path) {
    let ime = root.join("ime");
    let left = root.join("left");
    std::fs::create_dir_all(ime.join("한글폴더")).unwrap();
    std::fs::create_dir_all(&left).unwrap();
    for n in ["가방.txt", "문서함.txt", "하늘.txt", "ghost.txt", "한글.txt"] {
        std::fs::File::create(ime.join(n)).unwrap();
    }
    std::fs::File::create(ime.join("한글폴더/여기까지오면통과.txt")).unwrap();
    let out = root.join("out-keys");
    let cfg = fd_host::Config {
        candidate: "B".into(),
        stack: "slint",
        out_dir: out.clone(),
        perf_auto: false,
        folders: vec![ime.display().to_string(), left.display().to_string()],
    };
    let (ui, _a, host) = start_app(cfg);
    assert!(pump_until(Duration::from_secs(10), || { let s = host.snapshot(0); !s.loading && s.len == 6 }), "한글입력 폴더 읽기");
    let events = out.join("events.log");

    // Ctrl+A → 전체 선택, shortcut 기록
    chord(&ui, Key::Control, "a");
    assert_eq!(host.snapshot(0).selected_count, 6);
    assert_eq!(lines(&events, "\tshortcut\tCtrl+A\tselect-all").len(), 1);

    // 가방.txt 에서 F2 → 회의 → Enter 한 번 → 회의.txt
    let pos_of = |name: &str| (0..host.snapshot(0).len).find(|&p| host.item_name(0, p).as_deref() == Some(name)).unwrap();
    host.dispatch(0, fd_host::PaneMsg::Click { pos: pos_of("가방.txt"), ctrl: false, shift: false });
    pump_for(Duration::from_millis(50));
    press(&ui, Key::F2);
    assert!(host.snapshot(0).renaming.is_some(), "F2 = 이름 바꾸기");
    press(&ui, "회");
    press(&ui, "의");
    press(&ui, Key::Return);
    assert!(pump_until(Duration::from_secs(5), || ime.join("회의.txt").exists()), "회의.txt");
    assert!(!ime.join("가방.txt").exists());
    // 같은 Enter 가 목록 Activate 로 새지 않는다 (150ms)
    press(&ui, Key::Return);
    assert!(lines(&events, "shortcut-ignored\tEnter").len() <= 1);

    // 문서함.txt F2 → 취소 → Esc → 그대로
    assert!(pump_until(Duration::from_secs(5), || host.snapshot(0).len == 6 && !host.snapshot(0).loading));
    host.dispatch(0, fd_host::PaneMsg::Click { pos: pos_of("문서함.txt"), ctrl: false, shift: false });
    pump_for(Duration::from_millis(50));
    press(&ui, Key::F2);
    press(&ui, "취");
    press(&ui, "소");
    press(&ui, Key::Escape);
    pump_for(Duration::from_millis(300));
    assert!(ime.join("문서함.txt").exists() && !ime.join("취소.txt").exists());
    assert_eq!(lines(&events, "\tshortcut\tEsc\trename-cancel").len(), 1);
    assert!(host.snapshot(0).renaming.is_none());

    // type-ahead: 빈 곳 클릭 뒤 g → ghost.txt (영문 키 경로가 살아 있는지)
    pump_for(Duration::from_millis(1100)); // 편집 직후 보호 시간과 type-ahead 초기화 시간이 지나게
    host.dispatch(0, fd_host::PaneMsg::ClickEmpty);
    press(&ui, "g");
    let f = host.snapshot(0).focus.and_then(|p| host.item_name(0, p));
    assert_eq!(f.as_deref(), Some("ghost.txt"));

    // Ctrl+L → End → /한글폴더 → Enter 한 번
    chord(&ui, Key::Control, "l");
    press(&ui, Key::End);
    for ch in "/한글폴더".chars() {
        press(&ui, ch.to_string());
    }
    press(&ui, Key::Return);
    assert!(
        pump_until(Duration::from_secs(5), || host.snapshot(0).folder.ends_with("한글폴더") && !host.snapshot(0).loading),
        "주소줄 이동: {}",
        host.snapshot(0).folder
    );
    assert_eq!(host.item_name(0, 0).as_deref(), Some("여기까지오면통과.txt"));
    assert_eq!(lines(&events, "\tshortcut\tCtrl+L\taddress").len(), 1);

    // Tab → 오른쪽 칸, Backspace → 상위
    press(&ui, Key::Tab);
    assert_eq!(ui.get_active_pane(), 1);
    drop(ui);
}

#[test]
fn headless_ui_runs_contract_scenes() {
    let queue = Queue::default();
    QUEUE.with(|q| *q.borrow_mut() = Some(queue.clone()));
    slint::platform::set_platform(Box::new(Headless { queue, limit: Duration::from_secs(240) })).unwrap();
    let root = scratch("scenes");
    keyboard_scene(&root);
    auto_perf_scene(&root);
    let _ = std::fs::remove_dir_all(&root);
}
