#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! 시험 앱 A — Tauri 2 (wry: Linux WebKitGTK · Windows WebView2 · macOS WKWebView).
//!
//! 이 파일은 **다리 역할만** 한다: JS 명령 → `fd_host::Host`, Host 알림 → JS 이벤트,
//! OS 끌어다 놓기(wry DragDropEvent · `drag` 크레이트) → Host. 상태·규칙은 전부 fd-host/fd-viewmodel 에 있다.
//! 계약: `docs/shell-contract.md`.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use fd_host::{Config, Host, PaneMsg, PaneSnap, Point, Row, SortKey, Transfer, UiEvent, ViewMode};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, DragDropEvent, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

const DRAG_ICON: &[u8] = include_bytes!("../icons/32x32.png");
/// 앱에서 시작한 끌기가 끝난 뒤 이 시간 안에 우리 창에 놓기가 오면 "앱 안 끌기" 로 본다.
const OWN_DRAG_GRACE: Duration = Duration::from_millis(1500);

struct App {
    host: Host,
    started: AtomicBool,
    dnd: Mutex<DndState>,
}

#[derive(Default)]
struct DndState {
    /// 앱에서 시작한 끌기
    own: Option<OwnDrag>,
    /// GTK 가 마지막 drag-motion 에서 알려 준 원시 값 (Linux). 놓기 모드 판정과 기록에 쓴다.
    last_raw: Option<RawAction>,
}

#[derive(Clone, Copy)]
struct OwnDrag {
    pane: usize,
    mode: Transfer,
    finished: Option<Instant>,
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct RawAction {
    suggested: u32,
    /// 받는 쪽(WebKit)이 고른 action. 판정에는 쓰지 않고 `dnd-raw` 기록에만 남긴다 (Debug 출력).
    #[allow(dead_code)]
    selected: u32,
    actions: u32,
    shift: Option<bool>,
}

impl App {
    fn dnd(&self) -> MutexGuard<'_, DndState> {
        self.dnd.lock().unwrap_or_else(|p| p.into_inner())
    }
}

// ---------------------------------------------------------------- JS 로 나가는 모양
#[derive(Serialize, Clone)]
struct FdEvent {
    kind: &'static str,
    pane: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UiConfig {
    title: String,
    candidate: String,
    stack: &'static str,
    perf_auto: bool,
    perf_target: Option<String>,
    open_repeats: usize,
    scroll_seconds: f64,
    scroll_px_per_s: f64,
    settle_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapJs {
    version: u64,
    folder: String,
    len: usize,
    loading: bool,
    status: String,
    focus: Option<usize>,
    renaming: Option<usize>,
    selected_count: usize,
    sort: &'static str,
    descending: bool,
    view: &'static str,
    columns: usize,
    row_height: f32,
    rows: usize,
    tabs: Vec<TabJs>,
    active_tab: usize,
}

#[derive(Serialize)]
struct TabJs {
    id: u64,
    title: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RowJs {
    pos: usize,
    name: String,
    is_dir: bool,
    size: String,
    modified: String,
    selected: bool,
    focused: bool,
    ext: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct OsDragJs {
    phase: &'static str,
    /// 창 안 논리 좌표 (CSS px)
    x: f64,
    y: f64,
    paths: Vec<String>,
    mode: &'static str,
    origin: &'static str,
}

impl From<PaneSnap> for SnapJs {
    fn from(s: PaneSnap) -> Self {
        SnapJs {
            version: s.version,
            folder: s.folder,
            len: s.len,
            loading: s.loading,
            status: s.status,
            focus: s.focus,
            renaming: s.renaming,
            selected_count: s.selected_count,
            sort: match s.sort {
                SortKey::Name => "name",
                SortKey::Size => "size",
                SortKey::Type => "type",
                SortKey::Modified => "modified",
            },
            descending: s.descending,
            view: match s.view {
                ViewMode::Details => "details",
                ViewMode::LargeIcons => "icons",
            },
            columns: s.columns,
            row_height: s.row_height,
            rows: s.rows,
            tabs: s.tabs.into_iter().map(|t| TabJs { id: t.id, title: t.title }).collect(),
            active_tab: s.active_tab,
        }
    }
}

impl From<Row> for RowJs {
    fn from(r: Row) -> Self {
        RowJs {
            pos: r.pos,
            name: r.name,
            is_dir: r.is_dir,
            size: r.size,
            modified: r.modified,
            selected: r.selected,
            focused: r.focused,
            ext: r.ext,
        }
    }
}

// ---------------------------------------------------------------- JS 에서 들어오는 모양
#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "camelCase")]
enum MsgJs {
    Navigate { text: String },
    Activate { pos: usize },
    Up,
    Refresh,
    Click { pos: usize, ctrl: bool, shift: bool },
    ClickEmpty,
    SelectAll,
    MoveFocus { delta: i64, shift: bool },
    BeginRename,
    CommitRename { text: String },
    CancelRename,
    SetSort { key: String },
    SetView { view: String },
    SetViewport { width: f32, height: f32 },
}

impl From<MsgJs> for PaneMsg {
    fn from(m: MsgJs) -> Self {
        match m {
            MsgJs::Navigate { text } => PaneMsg::Navigate(text),
            MsgJs::Activate { pos } => PaneMsg::Activate(pos),
            MsgJs::Up => PaneMsg::Up,
            MsgJs::Refresh => PaneMsg::Refresh,
            MsgJs::Click { pos, ctrl, shift } => PaneMsg::Click { pos, ctrl, shift },
            MsgJs::ClickEmpty => PaneMsg::ClickEmpty,
            MsgJs::SelectAll => PaneMsg::SelectAll,
            MsgJs::MoveFocus { delta, shift } => PaneMsg::MoveFocus { delta, shift },
            MsgJs::BeginRename => PaneMsg::BeginRename,
            MsgJs::CommitRename { text } => PaneMsg::CommitRename(text),
            MsgJs::CancelRename => PaneMsg::CancelRename,
            MsgJs::SetSort { key } => PaneMsg::SetSort(match key.as_str() {
                "size" => SortKey::Size,
                "type" => SortKey::Type,
                "modified" => SortKey::Modified,
                _ => SortKey::Name,
            }),
            MsgJs::SetView { view } => PaneMsg::SetView(if view == "icons" { ViewMode::LargeIcons } else { ViewMode::Details }),
            MsgJs::SetViewport { width, height } => PaneMsg::SetViewport { width, height },
        }
    }
}

fn pane_ok(pane: usize) -> Result<usize, String> {
    if pane < 2 { Ok(pane) } else { Err(format!("칸 번호가 잘못됐습니다: {pane}")) }
}

fn mode_str(m: Transfer) -> &'static str {
    match m {
        Transfer::Copy => "copy",
        Transfer::Move => "move",
    }
}

// ---------------------------------------------------------------- 명령
#[tauri::command]
fn ui_ready(app: State<'_, App>) -> UiConfig {
    // 이벤트 수신이 붙은 뒤에 첫 폴더를 연다 — 첫 알림을 잃지 않게
    if !app.started.swap(true, Ordering::SeqCst) {
        app.host.start();
    }
    let cfg = app.host.config();
    UiConfig {
        title: cfg.window_title(),
        candidate: cfg.candidate.clone(),
        stack: cfg.stack,
        perf_auto: cfg.perf_auto,
        perf_target: app.host.perf_target(),
        open_repeats: fd_host::perf_plan::OPEN_REPEATS,
        scroll_seconds: fd_host::perf_plan::SCROLL_SECONDS,
        scroll_px_per_s: fd_host::perf_plan::SCROLL_PX_PER_S,
        settle_ms: fd_host::perf_plan::SETTLE_MS,
    }
}

#[tauri::command]
fn snapshot(app: State<'_, App>, pane: usize) -> Result<SnapJs, String> {
    Ok(app.host.snapshot(pane_ok(pane)?).into())
}

#[tauri::command]
fn rows(app: State<'_, App>, pane: usize, start: usize, count: usize) -> Result<Vec<RowJs>, String> {
    // 한 번에 너무 많이 달라는 요청은 자른다 (IPC 크기 상한)
    Ok(app.host.rows(pane_ok(pane)?, start, count.min(2000)).into_iter().map(RowJs::from).collect())
}

#[tauri::command]
fn dispatch(app: State<'_, App>, pane: usize, msg: MsgJs) -> Result<(), String> {
    app.host.dispatch(pane_ok(pane)?, msg.into());
    Ok(())
}

#[tauri::command]
fn type_ahead(app: State<'_, App>, pane: usize, text: String, committed: bool) -> Result<(), String> {
    let now_ms = fd_host::since_process_start_ms() as u64;
    app.host.type_ahead(pane_ok(pane)?, &text, committed, now_ms);
    Ok(())
}

#[tauri::command]
fn hit(app: State<'_, App>, pane: usize, x: f32, y: f32) -> Result<Option<usize>, String> {
    Ok(app.host.hit(pane_ok(pane)?, x, y))
}

#[tauri::command]
fn tab_op(app: State<'_, App>, op: String, pane: usize, idx: usize, to: Option<usize>, insert_at: Option<usize>) -> Result<(), String> {
    let pane = pane_ok(pane)?;
    match op.as_str() {
        "activate" => app.host.activate_tab(pane, idx),
        "new" => app.host.new_tab(pane),
        "close" => app.host.close_tab(pane, idx),
        "move" => app.host.move_tab(pane, idx, pane_ok(to.ok_or("to 없음")?)?, insert_at),
        other => return Err(format!("모르는 탭 명령: {other}")),
    }
    Ok(())
}

#[tauri::command]
fn log_event(app: State<'_, App>, kind: String, fields: Vec<String>) {
    app.host.log_event(&kind, &fields);
}

#[tauri::command]
fn perf(app: State<'_, App>, point: String, ms: f64) -> Result<(), String> {
    let p = Point::parse(&point).ok_or_else(|| format!("모르는 계측 지점: {point}"))?;
    app.host.perf(p, ms);
    Ok(())
}

#[tauri::command]
fn perf_frames(app: State<'_, App>, point: String, samples: Vec<f64>) -> Result<(), String> {
    let p = Point::parse(&point).ok_or_else(|| format!("모르는 계측 지점: {point}"))?;
    app.host.perf_frames(p, &samples);
    Ok(())
}

#[tauri::command]
fn cold_start(app: State<'_, App>) {
    app.host.cold_start_painted();
}

#[tauri::command]
fn exit_app(handle: AppHandle, app: State<'_, App>, code: i32) {
    app.host.log_event("exit", &[code.to_string()]);
    handle.exit(code);
}

/// 놓기. JS 가 칸과 콘텐츠 좌표를 정하고, 어느 항목 위인지는 VM 기하가 정한다.
#[tauri::command]
fn drop_at(app: State<'_, App>, pane: usize, x: f32, y: f32, paths: Vec<String>, mode: String, origin: String) -> Result<(), String> {
    let pane = pane_ok(pane)?;
    let target = app.host.hit(pane, x, y);
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    app.host.drop_paths(pane, target, paths, fd_host::parse_mode(&mode), &origin);
    Ok(())
}

/// 앱 → OS 끌기 시작. 마우스 버튼이 눌린 채로 불려야 한다 (Wayland 는 눌림 상태 없이 끌기를 시작할 수 없다).
/// 시작 순간 Shift 면 move 만, 아니면 copy 만 제안한다 (contract §6 — 둘 다 제안하면 Nautilus 가 이동을 고른다).
#[tauri::command]
fn drag_out(handle: AppHandle, window: WebviewWindow, app: State<'_, App>, pane: usize, shift: bool) -> Result<usize, String> {
    let pane = pane_ok(pane)?;
    let paths = app.host.selected_paths(pane);
    if paths.is_empty() {
        return Ok(0);
    }
    let n = paths.len();
    let mode = if shift { Transfer::Move } else { Transfer::Copy };
    app.dnd().own = Some(OwnDrag { pane, mode, finished: None });
    let host = app.host.clone();
    let w = window.clone();
    window
        .run_on_main_thread(move || {
            let opts = drag::Options {
                skip_animatation_on_cancel_or_failure: true,
                mode: if shift { drag::DragMode::Move } else { drag::DragMode::Copy },
            };
            let cb_paths = paths.clone();
            let cb_host = host.clone();
            let cb_handle = handle.clone();
            let on_end = move |r: drag::DragResult, _p: drag::CursorPosition| {
                let result = match r {
                    drag::DragResult::Dropped => "dropped",
                    drag::DragResult::Cancel => "cancel",
                };
                if let Some(st) = cb_handle.try_state::<App>()
                    && let Some(own) = st.dnd().own.as_mut()
                {
                    own.finished = Some(Instant::now());
                }
                cb_host.drag_out_finished(pane, &cb_paths, mode, result);
            };
            let image = drag::Image::Raw(DRAG_ICON.to_vec());
            #[cfg(target_os = "linux")]
            let r = match w.gtk_window() {
                Ok(gw) => drag::start_drag(&gw, drag::DragItem::Files(paths.clone()), image, on_end, opts).map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            #[cfg(not(target_os = "linux"))]
            let r = drag::start_drag(&w, drag::DragItem::Files(paths.clone()), image, on_end, opts).map_err(|e| e.to_string());
            if let Err(e) = r {
                host.drag_out_finished(pane, &paths, mode, &format!("error:{e}"));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(n)
}

// ---------------------------------------------------------------- OS → 앱 놓기
/// wry 가 준 놓기 이벤트를 JS 로 넘긴다. JS 가 좌표로 칸을 고르고 `drop_at` 으로 돌려준다.
fn on_drag_drop(window: &tauri::Window, ev: &DragDropEvent) {
    let Some(app) = window.try_state::<App>() else { return };
    let scale = window.scale_factor().unwrap_or(1.0);
    let (phase, pos, paths) = match ev {
        DragDropEvent::Enter { paths, position } => ("enter", Some(*position), paths.clone()),
        DragDropEvent::Over { position } => ("over", Some(*position), Vec::new()),
        DragDropEvent::Drop { paths, position } => ("drop", Some(*position), paths.clone()),
        DragDropEvent::Leave => ("leave", None, Vec::new()),
        _ => return,
    };
    let (mode, origin) = {
        let mut st = app.dnd();
        let own_live = st.own.filter(|o| o.finished.is_none_or(|t| t.elapsed() < OWN_DRAG_GRACE));
        match own_live {
            Some(o) => {
                if phase == "drop" {
                    st.own = None;
                    app.host.log_event("dnd-raw", &["internal".into(), format!("from_pane={}", o.pane), mode_str(o.mode).into()]);
                }
                (o.mode, "internal")
            }
            None => {
                let raw = st.last_raw;
                let m = external_mode(raw);
                if phase == "drop" {
                    app.host.log_event("dnd-raw", &["os".into(), format!("{raw:?}"), mode_str(m).into()]);
                }
                (m, "os")
            }
        }
    };
    let (x, y) = pos.map_or((0.0, 0.0), |p| (p.x / scale, p.y / scale));
    let payload = OsDragJs {
        phase,
        x,
        y,
        paths: paths.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
        mode: mode_str(mode),
        origin,
    };
    let _ = window.emit("os-drag", payload);
}

/// OS 에서 온 끌기의 복사/이동 판정. 기본은 복사, Shift(또는 OS 가 이동을 제안)면 이동.
/// - GTK 원본은 Shift 를 누르면 suggested=MOVE 로 보낸다 (X11 XdndPosition, GTK3/4 공통 규칙).
/// - Wayland 는 키보드 포커스가 없는 창에 Shift 를 알려 주지 않는다. 그래서 OS 가 제안한 action 도 함께 본다.
/// - 판정 근거(원시 값)는 events.log `dnd-raw` 에 남는다. 목요일 결과를 보고 이 규칙을 고친다.
fn external_mode(raw: Option<RawAction>) -> Transfer {
    const MOVE: u32 = 1 << 2; // GDK_ACTION_MOVE
    match raw {
        Some(r) if r.shift == Some(true) || r.suggested == MOVE || r.actions == MOVE => Transfer::Move,
        _ => Transfer::Copy,
    }
}

#[cfg(target_os = "linux")]
fn hook_gtk_dnd_probe(win: &WebviewWindow, handle: AppHandle) {
    let r = win.with_webview(move |wv| {
        use gtk::prelude::*;
        let view = wv.inner();
        // wry 의 drag-motion 처리 뒤에 붙는다. false 를 돌려 WebKit 기본 처리를 막지 않는다.
        view.connect_drag_motion(move |w, ctx, _x, _y, _t| {
            let shift = gtk::gdk::Keymap::for_display(&w.display())
                .map(|k| k.modifier_state() & gtk::gdk::ModifierType::SHIFT_MASK.bits() != 0);
            let raw = RawAction {
                suggested: ctx.suggested_action().bits(),
                selected: ctx.selected_action().bits(),
                actions: ctx.actions().bits(),
                shift,
            };
            if let Some(st) = handle.try_state::<App>() {
                st.dnd().last_raw = Some(raw);
            }
            false
        });
    });
    if let Err(e) = r {
        eprintln!("[tauri-shell] GTK 끌기 탐침을 붙이지 못함: {e}");
    }
}

/// NVIDIA 독점 드라이버 + WebKitGTK DMABuf 렌더러 조합은 창이 하얗게 비는 사례가 널리 알려져 있다.
/// 사용자가 따로 정하지 않았고 NVIDIA 드라이버가 올라와 있을 때만 끈다. 무엇을 했는지 events.log 에 남긴다.
#[cfg(target_os = "linux")]
fn linux_webkit_env() -> String {
    const KEY: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
    if std::env::var_os(KEY).is_some() {
        return format!("{KEY}=user:{}", std::env::var(KEY).unwrap_or_default());
    }
    if std::path::Path::new("/proc/driver/nvidia/version").exists() {
        // SAFETY: main 첫머리, 다른 스레드가 생기기 전이다 (Rust 2024 의 set_var 안전 조건).
        unsafe { std::env::set_var(KEY, "1") };
        return format!("{KEY}=1(nvidia)");
    }
    format!("{KEY}=unset")
}

fn main() {
    fd_host::mark_process_start();
    #[cfg(target_os = "linux")]
    let webkit_env = linux_webkit_env();
    #[cfg(not(target_os = "linux"))]
    let webkit_env = String::from("-");
    let cfg = Config::from_env("tauri");
    let title = cfg.window_title();
    tauri::Builder::default()
        .setup(move |app| {
            let handle = app.handle().clone();
            let emit_handle = handle.clone();
            let notify: fd_host::Notify = std::sync::Arc::new(move |e: UiEvent| {
                let ev = match e {
                    UiEvent::Pane(p) => FdEvent { kind: "pane", pane: p },
                    UiEvent::Tabs => FdEvent { kind: "tabs", pane: 0 },
                    UiEvent::FirstItem(p) => FdEvent { kind: "first-item", pane: p },
                };
                let _ = emit_handle.emit("fd", ev);
            });
            let host = Host::new(cfg.clone(), notify);
            host.log_event("webkit-env", std::slice::from_ref(&webkit_env));
            app.manage(App { host, started: AtomicBool::new(false), dnd: Mutex::new(DndState::default()) });
            let win = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title(&title)
                .inner_size(1280.0, 820.0)
                .min_inner_size(720.0, 420.0)
                .build()?;
            #[cfg(target_os = "linux")]
            hook_gtk_dnd_probe(&win, handle.clone());
            let _ = (&win, &handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::DragDrop(ev) = event {
                on_drag_drop(window, ev);
            }
        })
        .invoke_handler(tauri::generate_handler![
            ui_ready, snapshot, rows, dispatch, type_ahead, hit, tab_op, log_event, perf, perf_frames, cold_start, exit_app,
            drop_at, drag_out
        ])
        .run(tauri::generate_context!())
        .expect("시험 앱 A 를 시작하지 못했습니다");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_drop_defaults_to_copy_and_shift_or_move_offer_moves() {
        const COPY: u32 = 1 << 1;
        const MOVE: u32 = 1 << 2;
        assert_eq!(external_mode(None), Transfer::Copy);
        let r = |suggested, actions, shift| Some(RawAction { suggested, selected: 0, actions, shift });
        assert_eq!(external_mode(r(COPY, COPY | MOVE, None)), Transfer::Copy);
        assert_eq!(external_mode(r(MOVE, COPY | MOVE, None)), Transfer::Move, "GTK 원본이 Shift 로 MOVE 제안");
        assert_eq!(external_mode(r(COPY, COPY | MOVE, Some(true))), Transfer::Move, "키맵이 Shift 를 알려 줌");
        assert_eq!(external_mode(r(0, MOVE, None)), Transfer::Move, "원본이 이동만 허용");
    }

    #[test]
    fn js_messages_deserialize() {
        let m: MsgJs = serde_json::from_str(r#"{"type":"click","pos":3,"ctrl":false,"shift":true}"#).unwrap();
        assert!(matches!(PaneMsg::from(m), PaneMsg::Click { pos: 3, ctrl: false, shift: true }));
        let m: MsgJs = serde_json::from_str(r#"{"type":"setView","view":"icons"}"#).unwrap();
        assert!(matches!(PaneMsg::from(m), PaneMsg::SetView(ViewMode::LargeIcons)));
        let m: MsgJs = serde_json::from_str(r#"{"type":"commitRename","text":"회의.txt"}"#).unwrap();
        assert!(matches!(PaneMsg::from(m), PaneMsg::CommitRename(t) if t == "회의.txt"));
    }
}
