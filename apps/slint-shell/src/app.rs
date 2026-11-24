//! UI 스레드 쪽 접착부. 상태는 전부 `fd_host::Host` 가 들고, 여기서는
//! (1) Host 알림을 Slint 이벤트 루프로 옮기고 (2) 스냅숏을 Slint 속성·모델로 밀고
//! (3) Bridge 사건을 `PaneMsg`/Host 호출로 바꾼다. 판단 규칙은 순수 모듈(keys·ime·dnd·perf·rows)에 있다.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use fd_host::{Host, PaneMsg, PaneSnap, Point, SortKey, Transfer, UiEvent, ViewMode};
use slint::{ComponentHandle, DataTransfer, ModelRc, SharedString, Timer, VecModel};

use crate::dnd;
use crate::ime::{self, EnterGate, Gate, ImeGuard};
use crate::keys::{self, KeyCtx, ListCmd, Mods};
use crate::perf::{Cmd, PerfPlan};
use crate::rows::{DetailsModel, IconsModel, PaneRows};
use crate::{AppWindow, Bridge, IconRow, ItemData, PaneState, TabData};

const RENAME: usize = 0;
const ADDRESS: usize = 1;
/// 자동 측정 감시: fd-check.sh 의 `timeout 300` 보다 먼저 스스로 끝낸다 (기록 남기고 exit 1).
const PERF_WATCHDOG: Duration = Duration::from_secs(290);

// ---------------------------------------------------------------- 스레드 넘기기

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

pub fn app() -> Option<Rc<App>> {
    APP.with(|a| a.borrow().clone())
}

struct Pending {
    pane: [AtomicBool; 2],
    tabs: AtomicBool,
}

static PENDING: Pending = Pending { pane: [AtomicBool::new(false), AtomicBool::new(false)], tabs: AtomicBool::new(false) };

/// `fd_host::Notify`. 아무 스레드에서나 불린다 → Slint 이벤트 루프로 넘긴다.
/// 칸·탭 갱신은 합친다(열거 배치마다 오므로). FirstItem 은 합치지 않는다.
pub fn notify(e: UiEvent) {
    let schedule = || {
        let _ = slint::invoke_from_event_loop(|| {
            if let Some(a) = app() {
                a.drain();
            }
        });
    };
    match e {
        UiEvent::Pane(p) => {
            if !PENDING.pane[p & 1].swap(true, Ordering::AcqRel) {
                schedule();
            }
        }
        UiEvent::Tabs => {
            if !PENDING.tabs.swap(true, Ordering::AcqRel) {
                schedule();
            }
        }
        UiEvent::FirstItem(p) => {
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(a) = app() {
                    a.on_first_item(p & 1);
                }
            });
        }
    }
}

// ---------------------------------------------------------------- 상태

struct PaneView {
    rows: Rc<PaneRows>,
    details: ModelRc<ItemData>,
    icons: ModelRc<IconRow>,
    tabs: Rc<VecModel<TabData>>,
    snap: RefCell<Option<PaneSnap>>,
    shown_icons: Cell<Option<bool>>,
    drop_target: Cell<i32>,
    viewport: Cell<(f32, f32)>,
}

struct EditSession {
    gate: EnterGate,
    text: String,
    preedit: String,
    preedit_seen_ms: Option<u64>,
}

impl EditSession {
    fn new() -> Self {
        Self { gate: EnterGate::new(), text: String::new(), preedit: String::new(), preedit_seen_ms: None }
    }
    fn note(&mut self, text: &str, preedit: &str, now: u64) {
        self.text = text.to_string();
        self.preedit = preedit.to_string();
        if !preedit.is_empty() {
            self.preedit_seen_ms = Some(now);
        }
    }
}

struct DragOut {
    pane: usize,
    paths: Vec<PathBuf>,
    mode: Transfer,
    internal_drop: bool,
}

type Waiter = Box<dyn FnOnce(&App, Instant)>;

#[derive(Default)]
struct Frames {
    /// 렌더 알림(AfterRendering)을 쓰는가. 아니면 그리기 탐침(app.slint 맨 끝 사각형).
    notifier: Cell<bool>,
    queued: RefCell<Vec<Instant>>,
    flush_scheduled: Cell<bool>,
    waiters: RefCell<Vec<(Instant, Waiter)>>,
    ticking: Cell<Option<bool>>,
}

pub struct App {
    pub host: Host,
    ui: slint::Weak<AppWindow>,
    panes: [PaneView; 2],
    origin: Instant,
    guard: RefCell<ImeGuard>,
    edits: RefCell<[[Option<EditSession>; 2]; 2]>,
    deferred_click: Cell<Option<(usize, usize)>>,
    drag: RefCell<Option<DragOut>>,
    drag_cache: RefCell<Option<(usize, i32, DataTransfer)>>,
    last_proposed: [Cell<i32>; 2],
    frames: Frames,
    perf: RefCell<Option<PerfPlan>>,
    exit_code: Cell<Option<i32>>,
}

fn pane_of(p: i32) -> usize {
    (p.max(0) as usize) & 1
}

fn sort_code(k: SortKey) -> i32 {
    match k {
        SortKey::Name => 0,
        SortKey::Modified => 1,
        SortKey::Size => 2,
        SortKey::Type => 3,
    }
}

fn sort_key(code: i32) -> SortKey {
    match code {
        1 => SortKey::Modified,
        2 => SortKey::Size,
        3 => SortKey::Type,
        _ => SortKey::Name,
    }
}

fn files(paths: &[PathBuf]) -> DataTransfer {
    let mut d = DataTransfer::default();
    if !paths.is_empty() {
        d.set_file_paths(paths);
    }
    d
}

fn paths_of(d: &DataTransfer) -> Vec<PathBuf> {
    d.file_paths().map(|it| it.map(Path::to_path_buf).collect()).unwrap_or_default()
}

fn active_tab_id(s: &PaneSnap) -> u64 {
    s.tabs.get(s.active_tab).map_or(0, |t| t.id)
}

impl App {
    /// Bridge 콜백을 걸고 스레드 로컬에 넣는다.
    pub fn install(host: Host, ui: &AppWindow) -> Rc<App> {
        let mk = |p: usize| {
            let rows = PaneRows::new(p, host.clone());
            PaneView {
                details: ModelRc::new(DetailsModel(rows.clone())),
                icons: ModelRc::new(IconsModel(rows.clone())),
                rows,
                tabs: Rc::new(VecModel::default()),
                snap: RefCell::new(None),
                shown_icons: Cell::new(None),
                drop_target: Cell::new(-1),
                viewport: Cell::new((0.0, 0.0)),
            }
        };
        let app = Rc::new(App {
            host: host.clone(),
            ui: ui.as_weak(),
            panes: [mk(0), mk(1)],
            origin: Instant::now(),
            guard: RefCell::new(ImeGuard::default()),
            edits: RefCell::new([[None, None], [None, None]]),
            deferred_click: Cell::new(None),
            drag: RefCell::new(None),
            drag_cache: RefCell::new(None),
            last_proposed: [Cell::new(-1), Cell::new(-1)],
            frames: Frames::default(),
            perf: RefCell::new(None),
            exit_code: Cell::new(None),
        });
        ui.set_window_title(host.config().window_title().into());
        ui.set_left_tabs(ModelRc::from(app.panes[0].tabs.clone()));
        ui.set_right_tabs(ModelRc::from(app.panes[1].tabs.clone()));
        APP.with(|a| *a.borrow_mut() = Some(app.clone()));
        wire(ui);
        app.install_frame_clock(ui);
        if host.config().perf_auto {
            app.start_perf();
        }
        app
    }

    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code.get()
    }

    fn now_ms(&self) -> u64 {
        self.origin.elapsed().as_millis() as u64
    }

    fn ms(&self, t: Instant) -> f64 {
        t.saturating_duration_since(self.origin).as_secs_f64() * 1000.0
    }

    fn log(&self, kind: &str, fields: &[&str]) {
        let v: Vec<String> = fields.iter().map(|s| s.to_string()).collect();
        self.host.log_event(kind, &v);
    }

    fn with_ui(&self, f: impl FnOnce(&AppWindow)) {
        if let Some(ui) = self.ui.upgrade() {
            f(&ui);
        }
    }

    // ------------------------------------------------------------ Host → 화면
    fn drain(&self) {
        let tabs = PENDING.tabs.swap(false, Ordering::AcqRel);
        for (p, flag) in PENDING.pane.iter().enumerate() {
            let dirty = flag.swap(false, Ordering::AcqRel);
            if dirty || tabs {
                self.refresh_pane(p);
            }
        }
    }

    pub fn refresh_pane(&self, p: usize) {
        let Some(ui) = self.ui.upgrade() else { return };
        let snap = self.host.snapshot(p);
        let view = &self.panes[p];
        let old = view.snap.borrow_mut().replace(snap.clone());
        let structural = old.as_ref().is_none_or(|o| {
            active_tab_id(o) != active_tab_id(&snap)
                || o.folder != snap.folder
                || o.len != snap.len
                || o.sort != snap.sort
                || o.descending != snap.descending
                || o.view != snap.view
                || o.columns != snap.columns
        });
        if structural {
            view.rows.reset(snap.len, snap.columns);
        } else if old.as_ref().is_some_and(|o| o.version != snap.version) {
            view.rows.refresh();
        }
        // 보기에 맞는 모델만 붙인다 (숨은 ListView 는 빈 모델)
        let icons = snap.view == ViewMode::LargeIcons;
        if view.shown_icons.get() != Some(icons) {
            view.shown_icons.set(Some(icons));
            let (d, i) = if icons {
                (ModelRc::default(), view.icons.clone())
            } else {
                (view.details.clone(), ModelRc::default())
            };
            if p == 0 {
                ui.set_left_details(d);
                ui.set_left_icons(i);
            } else {
                ui.set_right_details(d);
                ui.set_right_icons(i);
            }
        }
        // 탭 제목
        let titles: Vec<SharedString> = snap.tabs.iter().map(|t| SharedString::from(t.title.as_str())).collect();
        let same_tabs = view.tabs.row_count_eq(titles.len()) && titles.iter().enumerate().all(|(i, t)| view.tabs.title_at(i) == *t);
        if !same_tabs {
            view.tabs.set_vec(titles.into_iter().map(|title| TabData { title }).collect::<Vec<_>>());
        }
        self.push_state(&ui, p, &snap);
        // VM 이 이름 바꾸기를 끝냈으면(클릭·이동) 입력칸도 닫는다
        if snap.renaming.is_none() && self.edits.borrow()[p][RENAME].is_some() {
            self.finish_edit(p, RENAME, None, "vm-closed");
        }
        if p == 0 {
            let cmds = self
                .perf
                .borrow_mut()
                .as_mut()
                .map(|pl| pl.on_pane(&snap.folder, snap.loading, self.ms(Instant::now())))
                .unwrap_or_default();
            self.exec(cmds);
        }
    }

    fn push_state(&self, ui: &AppWindow, p: usize, s: &PaneSnap) {
        let st = PaneState {
            folder: s.folder.as_str().into(),
            status: s.status.as_str().into(),
            loading: s.loading,
            icons: s.view == ViewMode::LargeIcons,
            sort_key: sort_code(s.sort),
            descending: s.descending,
            columns: s.columns as i32,
            len: s.len as i32,
            focus: s.focus.map_or(-1, |f| f as i32),
            renaming: s.renaming.map_or(-1, |f| f as i32),
            selected_count: s.selected_count as i32,
            version: s.version as i32,
            active_tab: s.active_tab as i32,
            drop_target: self.panes[p].drop_target.get(),
        };
        if p == 0 { ui.set_left_state(st) } else { ui.set_right_state(st) }
    }

    fn repush_state(&self, p: usize) {
        let snap = self.panes[p].snap.borrow().clone();
        if let (Some(ui), Some(s)) = (self.ui.upgrade(), snap) {
            self.push_state(&ui, p, &s);
        }
    }

    fn snap(&self, p: usize) -> Option<PaneSnap> {
        self.panes[p].snap.borrow().clone()
    }

    fn set_active(&self, p: usize) {
        self.with_ui(|ui| {
            if ui.get_active_pane() != p as i32 {
                ui.set_active_pane(p as i32);
            }
        });
    }

    fn on_first_item(&self, p: usize) {
        self.refresh_pane(p);
        if p != 0 {
            return;
        }
        if self.perf.borrow().as_ref().and_then(PerfPlan::awaiting_first_item).is_some() {
            self.after_next_frame(Box::new(|a, t| {
                let cmds = a.perf.borrow_mut().as_mut().map(|pl| pl.on_first_item_painted(a.ms(t))).unwrap_or_default();
                a.exec(cmds);
            }));
        }
    }

    // ------------------------------------------------------------ 목록 사건
    fn row_down(&self, p: usize, pos: usize, selected: bool, ctrl: bool, shift: bool) {
        self.set_active(p);
        if selected && !ctrl && !shift {
            // 이미 선택된 줄: 끌기일 수 있으니 놓을 때(clicked) 단일 선택으로 바꾼다
            self.deferred_click.set(Some((p, pos)));
        } else {
            self.deferred_click.set(None);
            self.host.dispatch(p, PaneMsg::Click { pos, ctrl, shift });
        }
    }

    fn row_click(&self, p: usize, pos: usize) {
        if self.deferred_click.take() == Some((p, pos)) {
            self.host.dispatch(p, PaneMsg::Click { pos, ctrl: false, shift: false });
        }
    }

    fn list_key(&self, p: usize, text: &str, m: Mods) -> bool {
        let Some(snap) = self.snap(p) else { return false };
        let (_, vh) = self.panes[p].viewport.get();
        let page_rows = ((vh / snap.row_height.max(1.0)).floor() as usize).max(1);
        let ctx = KeyCtx { icons: snap.view == ViewMode::LargeIcons, columns: snap.columns, page_rows, len: snap.len };
        let Some(cmd) = keys::map_list_key(text, m, ctx) else {
            // 글자는 입력칸이 받게 둔다 → edited → type-ahead. 그 밖의 특수 키는 삼킨다.
            return !keys::is_printable(text);
        };
        let now = self.now_ms();
        if cmd == ListCmd::Activate && self.guard.borrow().enter_is_echo(now) {
            self.log("shortcut-ignored", &["Enter", "echo-after-commit"]);
            return true;
        }
        if let Some((k, a)) = keys::shortcut_label(cmd, text, m) {
            self.log("shortcut", &[k, a]);
        }
        match cmd {
            ListCmd::Move { delta, shift } => self.host.dispatch(p, PaneMsg::MoveFocus { delta, shift }),
            ListCmd::Activate => {
                if let Some(f) = snap.focus {
                    self.host.dispatch(p, PaneMsg::Activate(f));
                }
            }
            ListCmd::Up => self.host.dispatch(p, PaneMsg::Up),
            ListCmd::Rename => self.begin_rename(p),
            ListCmd::Refresh => self.host.dispatch(p, PaneMsg::Refresh),
            ListCmd::SelectAll => self.host.dispatch(p, PaneMsg::SelectAll),
            ListCmd::Address => self.with_ui(|ui| ui.invoke_focus_address(p as i32)),
            ListCmd::OtherPane => self.with_ui(|ui| ui.invoke_focus_list(1 - p as i32)),
            ListCmd::Escape | ListCmd::Swallow => {}
        }
        true
    }

    fn typed(&self, p: usize, text: &str) {
        let now = self.now_ms();
        if self.guard.borrow().typing_blocked(now) {
            self.log("ime-guard", &["drop-committed", text]);
            return;
        }
        self.host.type_ahead(p, text, true, now);
    }

    fn composing(&self, p: usize, text: &str) {
        if text.is_empty() {
            return;
        }
        let now = self.now_ms();
        if self.guard.borrow().typing_blocked(now) {
            self.log("ime-guard", &["drop-composing", text]);
            return;
        }
        self.host.type_ahead(p, text, false, now);
    }

    fn viewport(&self, p: usize, w: f32, h: f32) {
        if w <= 0.0 || h <= 0.0 || self.panes[p].viewport.replace((w, h)) == (w, h) {
            return;
        }
        self.host.dispatch(p, PaneMsg::SetViewport { width: w, height: h });
    }

    // ------------------------------------------------------------ 이름 바꾸기·주소줄 (계약 §5)
    fn begin_rename(&self, p: usize) {
        self.host.dispatch(p, PaneMsg::BeginRename);
        let snap = self.host.snapshot(p);
        let Some(pos) = snap.renaming else { return };
        let Some(name) = self.host.item_name(p, pos) else { return };
        let stem = match name.rfind('.') {
            Some(i) if i > 0 => i,
            _ => name.len(),
        };
        self.edits.borrow_mut()[p][RENAME] = Some(EditSession::new());
        // 입력칸 위치·보임이 state.renaming 에 달려 있다 → 상태를 먼저 밀고 연다 (안 보이는 칸은 포커스를 못 받는다)
        self.refresh_pane(p);
        self.with_ui(|ui| ui.invoke_open_rename(p as i32, name.as_str().into(), stem as i32));
    }

    fn session<R>(&self, p: usize, kind: usize, f: impl FnOnce(&mut EditSession) -> R) -> Option<R> {
        let mut e = self.edits.borrow_mut();
        let slot = &mut e[p][kind];
        if slot.is_none() && kind == ADDRESS {
            *slot = Some(EditSession::new()); // 주소줄은 입력이 오면 그때 연다
        }
        slot.as_mut().map(f)
    }

    fn edit_enter(&self, p: usize, kind: usize, text: &str, preedit: &str) {
        let now = self.now_ms();
        let gate = self.session(p, kind, |s| {
            s.note(text, preedit, now);
            s.gate.enter(text, preedit, now)
        });
        match gate {
            Some(Gate::Run(v)) => self.finish_edit(p, kind, Some(v), "enter"),
            Some(Gate::Wait) => {
                self.log("ime-enter-wait", &[if kind == RENAME { "rename" } else { "address" }, text, preedit]);
                Timer::single_shot(Duration::from_millis(ime::COMMIT_TIMEOUT_MS + 20), move || {
                    if let Some(a) = app() {
                        a.edit_timeout(p, kind);
                    }
                });
            }
            Some(Gate::Ignore) | None => {}
        }
    }

    fn edit_preedit(&self, p: usize, kind: usize, text: &str, preedit: &str) {
        let now = self.now_ms();
        let run = {
            let mut e = self.edits.borrow_mut();
            e[p][kind].as_mut().and_then(|s| {
                s.note(text, preedit, now);
                s.gate.preedit_changed(text, preedit)
            })
        };
        if let Some(v) = run {
            self.finish_edit(p, kind, Some(v), "enter-after-commit");
        }
    }

    fn edit_timeout(&self, p: usize, kind: usize) {
        let now = self.now_ms();
        let run = {
            let mut e = self.edits.borrow_mut();
            e[p][kind].as_mut().and_then(|s| {
                let (t, pe) = (s.text.clone(), s.preedit.clone());
                s.gate.timeout(&t, &pe, now)
            })
        };
        if let Some(v) = run {
            self.finish_edit(p, kind, Some(v), "enter-timeout");
        }
    }

    fn edit_escape(&self, p: usize, kind: usize, preedit: &str) {
        let now = self.now_ms();
        let open = self.session(p, kind, |s| {
            s.note(&s.text.clone(), preedit, now);
        });
        if open.is_some() {
            self.log("shortcut", &["Esc", if kind == RENAME { "rename-cancel" } else { "address-cancel" }]);
            self.finish_edit(p, kind, None, "escape");
        }
        self.with_ui(|ui| ui.invoke_focus_list(p as i32));
    }

    fn edit_focus_lost(&self, p: usize, kind: usize) {
        if self.edits.borrow()[p][kind].is_some() {
            self.finish_edit(p, kind, None, "focus-lost");
        } else if kind == ADDRESS {
            self.with_ui(|ui| ui.invoke_reset_address(p as i32));
        }
    }

    /// 편집 한 번을 끝낸다. `value` 가 있으면 실행(정확히 한 번), 없으면 취소.
    fn finish_edit(&self, p: usize, kind: usize, value: Option<String>, why: &str) {
        let Some(mut s) = self.edits.borrow_mut()[p][kind].take() else { return };
        s.gate.cancel();
        let now = self.now_ms();
        let leftover = s.preedit_seen_ms.is_some_and(|t| now.saturating_sub(t) < 2000) && value.is_none();
        self.guard.borrow_mut().edit_ended(now, leftover, value.is_some());
        let what = if kind == RENAME { "rename" } else { "address" };
        self.log("edit-end", &[what, why, value.as_deref().unwrap_or("-"), &format!("pane={p}")]);
        match (kind, value) {
            (RENAME, Some(v)) => {
                self.host.dispatch(p, PaneMsg::CommitRename(v));
                self.with_ui(|ui| {
                    ui.invoke_close_rename(p as i32);
                    ui.invoke_focus_list(p as i32);
                });
            }
            (RENAME, None) => {
                self.host.dispatch(p, PaneMsg::CancelRename);
                // 다른 곳을 눌러 끝났으면 포커스는 그쪽에 둔다 (close-rename 은 입력칸이 포커스일 때만 목록으로 옮긴다)
                self.with_ui(|ui| ui.invoke_close_rename(p as i32));
            }
            (_, Some(v)) => {
                self.host.dispatch(p, PaneMsg::Navigate(v));
                self.with_ui(|ui| ui.invoke_focus_list(p as i32));
            }
            (_, None) => self.with_ui(|ui| ui.invoke_reset_address(p as i32)),
        }
    }

    // ------------------------------------------------------------ 탭
    fn tab_can_drop(&self, p: usize, data: &DataTransfer) -> bool {
        data.plain_text().ok().and_then(|t| dnd::parse_tab_payload(&t)).is_some_and(|(from, _)| from != p)
    }

    fn tab_dropped(&self, p: usize, data: &DataTransfer, x: f32) {
        let Some((from, idx)) = data.plain_text().ok().and_then(|t| dnd::parse_tab_payload(&t)) else { return };
        if from == p {
            return;
        }
        let n = self.snap(p).map_or(0, |s| s.tabs.len());
        let at = dnd::tab_insert_index(x, 150.0, n);
        self.log("dnd-raw", &["tab-drop", &format!("from={from}"), &format!("idx={idx}"), &format!("to={p}"), &format!("at={at}")]);
        self.host.move_tab(from, idx, p, Some(at));
    }

    // ------------------------------------------------------------ 끌어다 놓기 (계약 §6)
    fn drag_data(&self, p: usize, pos: usize, selected: bool, version: i32) -> DataTransfer {
        if !selected {
            return self.host.item_path(p, pos).map(|x| files(&[x])).unwrap_or_default();
        }
        // 선택 전체. 같은 판(version)이면 다시 만들지 않는다 (Ctrl+A 10만 개에서 줄마다 만들지 않게)
        if let Some((cp, cv, d)) = self.drag_cache.borrow().as_ref()
            && *cp == p
            && *cv == version
        {
            return d.clone();
        }
        let d = files(&self.host.selected_paths(p));
        *self.drag_cache.borrow_mut() = Some((p, version, d.clone()));
        d
    }

    fn drag_started(&self, p: usize, data: &DataTransfer, shift: bool) {
        let paths = paths_of(data);
        let mode = dnd::mode_for_shift(shift);
        let (copy, mv) = dnd::allowed_actions(shift);
        self.log(
            "dnd-raw",
            &[
                "drag-start",
                &format!("pane={p}"),
                &format!("allowed={}{}", if copy { "copy" } else { "" }, if mv { "move" } else { "" }),
                &format!("shift={shift}"),
                &paths.len().to_string(),
            ],
        );
        *self.drag.borrow_mut() = Some(DragOut { pane: p, paths, mode, internal_drop: false });
    }

    fn drag_finished(&self, p: usize, data: &DataTransfer, action: i32) {
        let d = self.drag.borrow_mut().take();
        let (pane, paths, mode, internal) = match d {
            Some(d) => (d.pane, d.paths, d.mode, d.internal_drop),
            None => (p, paths_of(data), Transfer::Copy, false),
        };
        self.log("dnd-raw", &["drag-finished", dnd::action_name(action), &format!("internal={internal}"), &format!("pane={pane}")]);
        if !internal {
            self.host.drag_out_finished(pane, &paths, mode, dnd::drag_result(action));
        }
    }

    fn set_drop_target(&self, p: usize, t: i32) {
        if self.panes[p].drop_target.replace(t) != t {
            self.repush_state(p);
        }
    }

    fn target_folder(&self, p: usize, x: f32, y: f32) -> (Option<usize>, bool) {
        let hit = self.host.hit(p, x, y);
        let is_dir = hit.is_some_and(|pos| self.host.rows(p, pos, 1).first().is_some_and(|r| r.is_dir));
        (hit, is_dir)
    }

    fn can_drop(&self, p: usize, data: &DataTransfer, x: f32, y: f32, proposed: i32) -> i32 {
        if !data.has_file_paths() {
            self.set_drop_target(p, -1);
            return dnd::ACT_NONE;
        }
        let (hit, is_dir) = self.target_folder(p, x, y);
        self.set_drop_target(p, if is_dir { hit.map_or(-1, |h| h as i32) } else { -1 });
        if self.last_proposed[p].replace(proposed) != proposed {
            self.log("dnd-raw", &["hover", &format!("pane={p}"), &format!("proposed={}", dnd::action_name(proposed))]);
        }
        dnd::drop_action(proposed)
    }

    fn dropped(&self, p: usize, data: &DataTransfer, x: f32, y: f32, proposed: i32) -> i32 {
        self.set_drop_target(p, -1);
        self.last_proposed[p].set(-1);
        let paths = paths_of(data);
        if paths.is_empty() {
            return dnd::ACT_NONE;
        }
        let (target, _) = self.target_folder(p, x, y);
        let act = dnd::drop_action(proposed);
        let mode = dnd::transfer_of(act).unwrap_or(Transfer::Copy);
        let internal = match self.drag.borrow_mut().as_mut() {
            Some(d) if dnd::same_paths(&d.paths, &paths) => {
                d.internal_drop = true;
                true
            }
            _ => false,
        };
        let origin = if internal { "internal" } else { "os" };
        self.log(
            "dnd-raw",
            &[
                "drop",
                origin,
                &format!("proposed={}", dnd::action_name(proposed)),
                &format!("chosen={}", dnd::action_name(act)),
                &format!("pane={p}"),
            ],
        );
        self.host.drop_paths(p, target, paths, mode, origin);
        act
    }

    fn drop_left(&self, p: usize) {
        self.set_drop_target(p, -1);
        self.last_proposed[p].set(-1);
    }

    // ------------------------------------------------------------ 프레임 시계
    fn install_frame_clock(&self, ui: &AppWindow) {
        let ok = ui
            .window()
            .set_rendering_notifier(|state, _| {
                if let slint::RenderingState::AfterRendering = state {
                    let t = Instant::now();
                    if let Some(a) = app() {
                        a.frame_rendered(t);
                    }
                }
            })
            .is_ok();
        self.frames.notifier.set(ok);
        let method = if ok {
            "rendering-notifier(AfterRendering); pacing=animation-tick changed"
        } else {
            "render-probe(animation-tick binding read while painting); pacing=animation-tick changed"
        };
        self.log("perf-method", &[method]);
        // ColdStart: 첫 화면을 그린 직후
        self.after_next_frame(Box::new(|a, t| {
            a.host.cold_start_painted();
            a.log("first-frame", &[&format!("{:.1}", a.ms(t))]);
        }));
    }

    /// 그리기 중에 불릴 수 있다 — Slint 를 건드리지 않고 시각만 쌓는다.
    fn frame_rendered(&self, t: Instant) {
        if let Ok(mut q) = self.frames.queued.try_borrow_mut() {
            q.push(t);
        }
        if !self.frames.flush_scheduled.replace(true) {
            Timer::single_shot(Duration::ZERO, || {
                if let Some(a) = app() {
                    a.flush_frames();
                }
            });
        }
    }

    fn render_probe(&self) {
        if !self.frames.notifier.get() {
            self.frame_rendered(Instant::now());
        }
    }

    fn flush_frames(&self) {
        self.frames.flush_scheduled.set(false);
        let frames = std::mem::take(&mut *self.frames.queued.borrow_mut());
        for t in frames {
            if let Some(pl) = self.perf.borrow_mut().as_mut() {
                pl.on_frame(self.ms(t));
            }
            let ready: Vec<(Instant, Waiter)> = {
                let mut w = self.frames.waiters.borrow_mut();
                let (ready, keep): (Vec<_>, Vec<_>) = w.drain(..).partition(|(reg, _)| t > *reg);
                *w = keep;
                ready
            };
            for (_, f) in ready {
                f(self, t);
            }
        }
        self.update_ticking();
    }

    fn after_next_frame(&self, f: Waiter) {
        self.frames.waiters.borrow_mut().push((Instant::now(), f));
        self.update_ticking();
        self.with_ui(|ui| ui.window().request_redraw());
    }

    fn update_ticking(&self) {
        let want = !self.frames.waiters.borrow().is_empty() || self.perf.borrow().as_ref().is_some_and(PerfPlan::is_scrolling);
        if self.frames.ticking.replace(Some(want)) != Some(want) {
            self.with_ui(|ui| {
                ui.set_ticking(want);
                if want {
                    ui.window().request_redraw();
                }
            });
        }
    }

    fn frame_tick(&self) {
        let now = self.ms(Instant::now());
        let cmds = self.perf.borrow_mut().as_mut().map(|pl| pl.on_tick(now)).unwrap_or_default();
        self.exec(cmds);
    }

    // ------------------------------------------------------------ 자동 측정 (계약 §2)
    fn start_perf(&self) {
        let Some(target) = self.host.perf_target() else {
            self.log("perf-error", &["no-target-folder-argument"]);
            self.exit(2);
            return;
        };
        self.log("perf-start", &[&target]);
        *self.perf.borrow_mut() = Some(PerfPlan::new(target));
        Timer::single_shot(PERF_WATCHDOG, || {
            if let Some(a) = app() {
                let phase = a.perf.borrow().as_ref().map(PerfPlan::phase_name).unwrap_or_default();
                a.log("perf-timeout", &[&phase]);
                a.exit(1);
            }
        });
    }

    fn exec(&self, cmds: Vec<Cmd>) {
        for c in cmds {
            match c {
                Cmd::Up => self.host.dispatch(0, PaneMsg::Up),
                Cmd::Navigate(t) => self.host.dispatch(0, PaneMsg::Navigate(t)),
                Cmd::SetView(v) => self.host.dispatch(0, PaneMsg::SetView(v)),
                Cmd::ScrollTo(y) => self.with_ui(|ui| ui.invoke_scroll_to(0, y as f32)),
                Cmd::Settle(ms) => Timer::single_shot(Duration::from_millis(ms), || {
                    if let Some(a) = app() {
                        let now = a.ms(Instant::now());
                        let cmds = a.perf.borrow_mut().as_mut().map(|pl| pl.on_settled(now)).unwrap_or_default();
                        a.exec(cmds);
                    }
                }),
                Cmd::Ticking(_) => self.update_ticking(),
                Cmd::RecordFirstItem(ms) => self.host.perf(Point::FirstItem, ms),
                Cmd::RecordFrames(point, v) => {
                    self.host.perf_frames(point, &v);
                    self.log("perf-frames", &[point.name(), &v.len().to_string()]);
                }
                Cmd::Exit(code) => self.exit(code),
            }
        }
    }

    fn exit(&self, code: i32) {
        if self.exit_code.get().is_none() {
            self.exit_code.set(Some(code));
            self.log("exit", &[&code.to_string()]);
            if slint::quit_event_loop().is_err() {
                std::process::exit(code);
            }
        }
    }
}

// VecModel 비교 도우미 (탭 제목이 그대로면 모델을 건드리지 않는다)
trait TabTitles {
    fn row_count_eq(&self, n: usize) -> bool;
    fn title_at(&self, i: usize) -> SharedString;
}

impl TabTitles for VecModel<TabData> {
    fn row_count_eq(&self, n: usize) -> bool {
        slint::Model::row_count(self) == n
    }
    fn title_at(&self, i: usize) -> SharedString {
        slint::Model::row_data(self, i).map(|t| t.title).unwrap_or_default()
    }
}

// ---------------------------------------------------------------- Bridge 연결

fn wire(ui: &AppWindow) {
    let b = ui.global::<Bridge>();
    b.on_row_down(|p, pos, sel, ctrl, shift| {
        if let Some(a) = app() {
            a.row_down(pane_of(p), pos.max(0) as usize, sel, ctrl, shift);
        }
    });
    b.on_row_click(|p, pos| {
        if let Some(a) = app() {
            a.row_click(pane_of(p), pos.max(0) as usize);
        }
    });
    b.on_row_activate(|p, pos| {
        if let Some(a) = app() {
            a.host.dispatch(pane_of(p), PaneMsg::Activate(pos.max(0) as usize));
        }
    });
    b.on_empty_click(|p| {
        if let Some(a) = app() {
            a.set_active(pane_of(p));
            a.deferred_click.set(None);
            a.host.dispatch(pane_of(p), PaneMsg::ClickEmpty);
        }
    });
    b.on_list_key(|p, text, ctrl, shift, alt, meta| {
        app().is_some_and(|a| a.list_key(pane_of(p), &text, Mods { ctrl, shift, alt, meta }))
    });
    b.on_typed(|p, text| {
        if let Some(a) = app() {
            a.typed(pane_of(p), &text);
        }
    });
    b.on_composing(|p, text| {
        if let Some(a) = app() {
            a.composing(pane_of(p), &text);
        }
    });
    b.on_list_focused(|p| {
        if let Some(a) = app() {
            a.set_active(pane_of(p));
        }
    });
    b.on_viewport(|p, w, h| {
        if let Some(a) = app() {
            a.viewport(pane_of(p), w, h);
        }
    });
    b.on_go_up(|p| {
        if let Some(a) = app() {
            a.set_active(pane_of(p));
            a.host.dispatch(pane_of(p), PaneMsg::Up);
        }
    });
    b.on_refresh(|p| {
        if let Some(a) = app() {
            a.set_active(pane_of(p));
            a.host.dispatch(pane_of(p), PaneMsg::Refresh);
        }
    });
    b.on_set_view(|p, icons| {
        if let Some(a) = app() {
            a.set_active(pane_of(p));
            a.host.dispatch(pane_of(p), PaneMsg::SetView(if icons { ViewMode::LargeIcons } else { ViewMode::Details }));
        }
    });
    b.on_sort(|p, key| {
        if let Some(a) = app() {
            a.host.dispatch(pane_of(p), PaneMsg::SetSort(sort_key(key)));
        }
    });
    b.on_edit_enter(|p, kind, text, preedit| {
        if let Some(a) = app() {
            a.edit_enter(pane_of(p), (kind as usize) & 1, &text, &preedit);
        }
    });
    b.on_edit_preedit(|p, kind, text, preedit| {
        if let Some(a) = app() {
            a.edit_preedit(pane_of(p), (kind as usize) & 1, &text, &preedit);
        }
    });
    b.on_edit_escape(|p, kind, preedit| {
        if let Some(a) = app() {
            a.edit_escape(pane_of(p), (kind as usize) & 1, &preedit);
        }
    });
    b.on_edit_focus_lost(|p, kind, _text| {
        if let Some(a) = app() {
            a.edit_focus_lost(pane_of(p), (kind as usize) & 1);
        }
    });
    b.on_tab_activate(|p, idx| {
        if let Some(a) = app() {
            a.set_active(pane_of(p));
            a.host.activate_tab(pane_of(p), idx.max(0) as usize);
        }
    });
    b.on_tab_new(|p| {
        if let Some(a) = app() {
            a.host.new_tab(pane_of(p));
        }
    });
    b.on_tab_close(|p, idx| {
        if let Some(a) = app() {
            a.host.close_tab(pane_of(p), idx.max(0) as usize);
        }
    });
    b.on_tab_drag_data(|p, idx| {
        let mut d = DataTransfer::default();
        d.set_plain_text(dnd::tab_payload(pane_of(p), idx.max(0) as usize).into());
        d
    });
    b.on_tab_can_drop(|p, data| app().is_some_and(|a| a.tab_can_drop(pane_of(p), &data)));
    b.on_tab_dropped(|p, data, x| {
        if let Some(a) = app() {
            a.tab_dropped(pane_of(p), &data, x);
        }
    });
    b.on_drag_data(|p, pos, sel, version| {
        app().map(|a| a.drag_data(pane_of(p), pos.max(0) as usize, sel, version)).unwrap_or_default()
    });
    b.on_drag_started(|p, data, shift| {
        if let Some(a) = app() {
            a.drag_started(pane_of(p), &data, shift);
        }
    });
    b.on_drag_finished(|p, data, action| {
        if let Some(a) = app() {
            a.drag_finished(pane_of(p), &data, action);
        }
    });
    b.on_can_drop(|p, data, x, y, proposed| {
        app().map_or(dnd::ACT_NONE, |a| a.can_drop(pane_of(p), &data, x, y, proposed))
    });
    b.on_dropped(|p, data, x, y, proposed| {
        app().map_or(dnd::ACT_NONE, |a| a.dropped(pane_of(p), &data, x, y, proposed))
    });
    b.on_drop_left(|p| {
        if let Some(a) = app() {
            a.drop_left(pane_of(p));
        }
    });
    b.on_frame_tick(|| {
        if let Some(a) = app() {
            a.frame_tick();
        }
    });
    b.on_render_probe(|_| {
        if let Some(a) = app() {
            a.render_probe();
        }
        true
    });
}
