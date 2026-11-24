use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use fd_core::{format, location, NamePolicy, PathFlavor, Point};
use fd_viewmodel::{Effect, PaneMsg, PaneVm, RequestId, SortKey, Transfer, ViewMode};

use crate::{Config, EventLog, PerfLog};

/// View 에게 "다시 읽어라" 를 알린다. 어느 스레드에서든 불린다 — View 는 자기 UI 스레드로 넘겨야 한다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiEvent {
    /// 칸 내용(목록·선택·상태)이 바뀌었다. `snapshot`/`rows` 를 다시 읽는다.
    Pane(usize),
    /// 탭 구성이 바뀌었다. 양쪽 칸 모두 다시 읽는다.
    Tabs,
    /// 폴더 전환 뒤 첫 항목이 들어왔다. View 는 **그린 직후** 시각을 재서 FirstItem 으로 남긴다.
    FirstItem(usize),
}

pub type Notify = Arc<dyn Fn(UiEvent) + Send + Sync>;

#[derive(Clone, Debug)]
pub struct TabInfo {
    pub id: u64,
    pub title: String,
}

#[derive(Clone, Debug)]
pub struct PaneSnap {
    pub version: u64,
    pub folder: String,
    pub len: usize,
    pub loading: bool,
    pub status: String,
    pub focus: Option<usize>,
    pub renaming: Option<usize>,
    pub selected_count: usize,
    pub sort: SortKey,
    pub descending: bool,
    pub view: ViewMode,
    /// 한 행 항목 수 (자세히 = 1)
    pub columns: usize,
    pub row_height: f32,
    /// 행 수 (큰 아이콘은 합성 행)
    pub rows: usize,
    pub tabs: Vec<TabInfo>,
    pub active_tab: usize,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub pos: usize,
    pub name: String,
    pub is_dir: bool,
    pub size: String,
    pub modified: String,
    pub selected: bool,
    pub focused: bool,
    /// 확장자 (소문자). 아이콘 고르기용.
    pub ext: String,
}

struct Tab {
    id: u64,
    vm: PaneVm,
    cancels: HashMap<RequestId, Arc<AtomicBool>>,
    seen_mtime: Option<SystemTime>,
}

struct PaneState {
    tabs: Vec<Tab>,
    active: usize,
}

struct State {
    panes: [PaneState; 2],
    next_tab: u64,
}

impl State {
    fn locate(&self, tab_id: u64) -> Option<(usize, usize)> {
        self.panes
            .iter()
            .enumerate()
            .find_map(|(p, ps)| ps.tabs.iter().position(|t| t.id == tab_id).map(|i| (p, i)))
    }
    fn tab_mut(&mut self, tab_id: u64) -> Option<(usize, &mut Tab)> {
        let (p, i) = self.locate(tab_id)?;
        Some((p, &mut self.panes[p].tabs[i]))
    }
    fn active(&self, pane: usize) -> &Tab {
        let ps = &self.panes[pane];
        &ps.tabs[ps.active]
    }
    fn active_id(&self, pane: usize) -> u64 {
        self.active(pane).id
    }
}

struct Inner {
    cfg: Config,
    flavor: PathFlavor,
    policy: NamePolicy,
    st: Mutex<State>,
    notify: Notify,
    events: EventLog,
    perf: PerfLog,
    cold_start_done: AtomicBool,
}

/// 앱 계층. `Clone` 은 같은 상태를 가리킨다.
#[derive(Clone)]
pub struct Host(Arc<Inner>);

const POLL_INTERVAL: Duration = Duration::from_millis(1000);

impl Host {
    pub fn new(cfg: Config, notify: Notify) -> Self {
        let flavor = PathFlavor::native();
        let policy = NamePolicy::native();
        let _ = std::fs::create_dir_all(&cfg.out_dir);
        let events = EventLog::new(&cfg.out_dir);
        let perf = PerfLog::new(&cfg.out_dir, cfg.perf_auto);
        let mut next_tab = 0;
        let mut mk = || {
            next_tab += 1;
            Tab { id: next_tab, vm: PaneVm::new(flavor, policy), cancels: HashMap::new(), seen_mtime: None }
        };
        let panes = [
            PaneState { tabs: vec![mk(), mk()], active: 0 },
            PaneState { tabs: vec![mk(), mk()], active: 0 },
        ];
        let st = State { panes, next_tab };
        Host(Arc::new(Inner {
            cfg,
            flavor,
            policy,
            st: Mutex::new(st),
            notify,
            events,
            perf,
            cold_start_done: AtomicBool::new(false),
        }))
    }

    pub fn config(&self) -> &Config {
        &self.0.cfg
    }

    /// 첫 폴더들을 연다. View 가 알림을 받을 준비가 된 뒤 부른다.
    /// 왼쪽 = [인자1, 인자1의 상위], 오른쪽 = [인자2 (없으면 인자1의 상위), 홈]. 칸마다 탭 2개 (부록 A-9).
    pub fn start(&self) {
        let home = home_dir();
        let a = self.0.cfg.folders.first().cloned().unwrap_or_else(|| home.clone());
        let a_parent = self.parent_of(&a).unwrap_or_else(|| home.clone());
        let b = self.0.cfg.folders.get(1).cloned().unwrap_or_else(|| a_parent.clone());
        let plan = [[a, a_parent], [b, home]];
        let ids: Vec<(u64, String)> = {
            let st = self.lock();
            (0..2).flat_map(|p| (0..2).map(move |i| (p, i))).map(|(p, i)| (st.panes[p].tabs[i].id, plan[p][i].clone())).collect()
        };
        for (id, folder) in ids {
            self.apply_to_tab(id, PaneMsg::Navigate(folder));
        }
        if !self.0.cfg.perf_auto {
            let h = self.clone();
            std::thread::Builder::new().name("fd-refresh-poll".into()).spawn(move || h.poll_loop()).ok();
        }
        self.0.events.log(
            "start",
            &[self.0.cfg.stack.to_string(), self.0.cfg.candidate.clone(), format!("perf_auto={}", self.0.cfg.perf_auto)],
        );
    }

    // ------------------------------------------------------------------ 읽기
    pub fn snapshot(&self, pane: usize) -> PaneSnap {
        let st = self.lock();
        let ps = &st.panes[pane];
        let vm = &ps.tabs[ps.active].vm;
        let layout = vm.layout();
        let (sort, descending) = vm.sort();
        PaneSnap {
            version: vm.version(),
            folder: vm.folder_display(),
            len: vm.len(),
            loading: vm.loading(),
            status: vm.status().to_string(),
            focus: vm.focus(),
            renaming: vm.renaming(),
            selected_count: vm.selected_count(),
            sort,
            descending,
            view: layout.view,
            columns: layout.columns(),
            row_height: layout.row_height(),
            rows: layout.rows(vm.len()),
            tabs: ps.tabs.iter().map(|t| TabInfo { id: t.id, title: tab_title(&t.vm) }).collect(),
            active_tab: ps.active,
        }
    }

    /// 위치 [start, start+count) 의 항목. 범위를 넘으면 잘라서 준다.
    pub fn rows(&self, pane: usize, start: usize, count: usize) -> Vec<Row> {
        let st = self.lock();
        let vm = &st.active(pane).vm;
        let focus = vm.focus();
        let end = start.saturating_add(count).min(vm.len());
        (start.min(end)..end)
            .filter_map(|pos| {
                let it = vm.item(pos)?;
                Some(Row {
                    pos,
                    name: it.name.clone(),
                    is_dir: it.is_dir,
                    size: if it.is_dir { String::new() } else { format::size_text(it.size) },
                    modified: it.modified.map(modified_text).unwrap_or_default(),
                    selected: vm.is_selected(pos),
                    focused: focus == Some(pos),
                    ext: it.extension().to_lowercase(),
                })
            })
            .collect()
    }

    /// 콘텐츠 좌표(스크롤 반영, 논리 px) → 항목 위치. 판정은 VM 기하가 한다.
    pub fn hit(&self, pane: usize, x: f32, y: f32) -> Option<usize> {
        let st = self.lock();
        let vm = &st.active(pane).vm;
        vm.layout().hit(x, y, vm.len())
    }

    pub fn selected_paths(&self, pane: usize) -> Vec<PathBuf> {
        self.lock().active(pane).vm.selected_paths()
    }

    pub fn item_path(&self, pane: usize, pos: usize) -> Option<PathBuf> {
        self.lock().active(pane).vm.item(pos).map(|i| i.path.clone())
    }

    pub fn item_name(&self, pane: usize, pos: usize) -> Option<String> {
        self.lock().active(pane).vm.item(pos).map(|i| i.name.clone())
    }

    // ------------------------------------------------------------------ 쓰기
    /// 활성 탭에 메시지를 보낸다.
    pub fn dispatch(&self, pane: usize, msg: PaneMsg) {
        let id = self.lock().active_id(pane);
        self.apply_to_tab(id, msg);
    }

    /// type-ahead. `committed == false` 면 IME 조합 중 미리 찾기. 결과를 events.log 에 남긴다.
    pub fn type_ahead(&self, pane: usize, text: &str, committed: bool, now_ms: u64) {
        let msg = if committed {
            PaneMsg::TypeAhead { text: text.to_string(), now_ms }
        } else {
            PaneMsg::TypeAheadPreview { composing: text.to_string(), now_ms }
        };
        self.dispatch(pane, msg);
        let moved = {
            let st = self.lock();
            let vm = &st.active(pane).vm;
            vm.focus().and_then(|f| vm.item(f)).map(|i| i.name.clone()).unwrap_or_default()
        };
        let kind = if committed { "committed" } else { "composing" };
        self.0.events.log("type-ahead", &[text.to_string(), moved, kind.to_string(), format!("pane={pane}")]);
    }

    /// 놓기 (OS 에서 왔든 다른 칸에서 왔든). target = 놓은 자리 항목 위치 (빈 곳이면 None).
    pub fn drop_paths(&self, pane: usize, target: Option<usize>, paths: Vec<PathBuf>, mode: Transfer, origin: &str) {
        let n = paths.len();
        self.0.events.log(
            "drop-request",
            &[format!("pane={pane}"), format!("target={target:?}"), mode_text(mode).into(), n.to_string(), origin.to_string()],
        );
        self.dispatch(pane, PaneMsg::Drop { paths, target, mode });
    }

    /// 앱에서 밖으로 끌기가 끝났다. 받는 쪽이 옮겼을 수 있으니 목록을 다시 읽는다.
    pub fn drag_out_finished(&self, pane: usize, paths: &[PathBuf], mode: Transfer, result: &str) {
        let mut f = vec![mode_text(mode).to_string(), result.to_string(), paths.len().to_string()];
        f.extend(paths.iter().map(|p| p.display().to_string()));
        f.push(format!("pane={pane}"));
        self.0.events.log("drag-out", &f);
        let h = self.clone();
        let dirs: Vec<PathBuf> = paths.iter().filter_map(|p| p.parent().map(Path::to_path_buf)).collect();
        std::thread::spawn(move || {
            // 받는 쪽(파일 관리자)이 옮기기를 끝낼 시간을 준다. 감시 폴링도 따로 돈다.
            for wait in [300, 1500] {
                std::thread::sleep(Duration::from_millis(wait));
                h.refresh_folders(&dirs);
            }
        });
    }

    pub fn log_event(&self, kind: &str, fields: &[String]) {
        self.0.events.log(kind, fields);
    }

    // ------------------------------------------------------------------ 탭
    pub fn activate_tab(&self, pane: usize, idx: usize) {
        {
            let mut st = self.lock();
            if idx >= st.panes[pane].tabs.len() {
                return;
            }
            st.panes[pane].active = idx;
        }
        (self.0.notify)(UiEvent::Tabs);
    }

    /// 현재 폴더를 새 탭으로 연다.
    pub fn new_tab(&self, pane: usize) {
        let (id, folder) = {
            let mut st = self.lock();
            let folder = st.active(pane).vm.folder_display();
            st.next_tab += 1;
            let id = st.next_tab;
            st.panes[pane].tabs.push(Tab {
                id,
                vm: PaneVm::new(self.0.flavor, self.0.policy),
                cancels: HashMap::new(),
                seen_mtime: None,
            });
            let ps = &mut st.panes[pane];
            ps.active = ps.tabs.len() - 1;
            (id, folder)
        };
        self.apply_to_tab(id, PaneMsg::Navigate(folder));
        (self.0.notify)(UiEvent::Tabs);
    }

    pub fn close_tab(&self, pane: usize, idx: usize) {
        {
            let mut st = self.lock();
            let ps = &mut st.panes[pane];
            if ps.tabs.len() <= 1 || idx >= ps.tabs.len() {
                return;
            }
            let t = ps.tabs.remove(idx);
            for f in t.cancels.values() {
                f.store(true, Ordering::Relaxed);
            }
            if ps.active >= ps.tabs.len() || ps.active > idx {
                ps.active = ps.active.saturating_sub(1).min(ps.tabs.len() - 1);
            }
        }
        (self.0.notify)(UiEvent::Tabs);
    }

    /// 탭을 다른 칸으로 옮긴다 (끌-7). 옮긴 탭이 그 칸의 활성 탭이 된다.
    /// 원래 칸의 마지막 탭이면 같은 폴더로 새 탭을 하나 남긴다 — 칸이 비지 않게.
    pub fn move_tab(&self, from: usize, idx: usize, to: usize, insert_at: Option<usize>) {
        if from == to || from > 1 || to > 1 {
            return;
        }
        let mut refill: Option<(u64, String)> = None;
        let title;
        {
            let mut st = self.lock();
            if idx >= st.panes[from].tabs.len() {
                return;
            }
            let tab = st.panes[from].tabs.remove(idx);
            title = tab_title(&tab.vm);
            if st.panes[from].tabs.is_empty() {
                st.next_tab += 1;
                let id = st.next_tab;
                let folder = tab.vm.folder_display();
                st.panes[from].tabs.push(Tab {
                    id,
                    vm: PaneVm::new(self.0.flavor, self.0.policy),
                    cancels: HashMap::new(),
                    seen_mtime: None,
                });
                refill = Some((id, folder));
            }
            let fp = &mut st.panes[from];
            if fp.active >= fp.tabs.len() || fp.active > idx {
                fp.active = fp.active.saturating_sub(1).min(fp.tabs.len() - 1);
            }
            let tp = &mut st.panes[to];
            let at = insert_at.unwrap_or(tp.tabs.len()).min(tp.tabs.len());
            tp.tabs.insert(at, tab);
            tp.active = at;
        }
        if let Some((id, folder)) = refill {
            self.apply_to_tab(id, PaneMsg::Navigate(folder));
        }
        self.0.events.log("tab-move", &[format!("from={from}"), format!("to={to}"), title]);
        (self.0.notify)(UiEvent::Tabs);
    }

    // ------------------------------------------------------------------ 계측
    pub fn perf(&self, point: Point, ms: f64) {
        self.0.perf.record(point, ms);
    }

    pub fn perf_frames(&self, point: Point, intervals_ms: &[f64]) {
        self.0.perf.record_many(point, intervals_ms);
    }

    /// 첫 화면을 그린 직후 한 번 부른다. 두 번째부터는 무시한다.
    pub fn cold_start_painted(&self) {
        if !self.0.cold_start_done.swap(true, Ordering::Relaxed) {
            let ms = crate::since_process_start_ms();
            self.0.perf.record(Point::ColdStart, ms);
            self.0.events.log("cold-start", &[format!("{ms:.1}")]);
        }
    }

    pub fn perf_target(&self) -> Option<String> {
        let f = self.0.cfg.folders.first()?;
        location::parse(f, self.0.flavor).ok().map(|l| location::display(&l, self.0.flavor))
    }

    pub fn parent_of(&self, folder: &str) -> Option<String> {
        let l = location::parse(folder, self.0.flavor).ok()?;
        location::parent(&l, self.0.flavor).map(|p| location::display(&p, self.0.flavor))
    }

    // ------------------------------------------------------------------ 내부
    fn lock(&self) -> MutexGuard<'_, State> {
        // 한 스레드가 패닉해도 나머지는 계속 돈다 — 스파이크에서 상태 하나 잃는 편이 앱이 굳는 것보다 낫다
        self.0.st.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn apply_to_tab(&self, tab_id: u64, msg: PaneMsg) {
        let mut out = Vec::new();
        {
            let mut st = self.lock();
            let Some((pane, tab)) = st.tab_mut(tab_id) else { return };
            let fx = tab.vm.update(msg);
            let is_active = st.panes[pane].tabs[st.panes[pane].active].id == tab_id;
            if is_active {
                out.push(UiEvent::Pane(pane));
            } else {
                out.push(UiEvent::Tabs); // 탭 제목이 바뀌었을 수 있다
            }
            self.run_effects(&mut st, tab_id, fx, &mut out);
        }
        out.dedup();
        for e in out {
            (self.0.notify)(e);
        }
    }

    fn run_effects(&self, st: &mut State, tab_id: u64, fx: Vec<Effect>, out: &mut Vec<UiEvent>) {
        for e in fx {
            match e {
                Effect::Enumerate { req, folder } => {
                    let flag = Arc::new(AtomicBool::new(false));
                    if let Some((_, tab)) = st.tab_mut(tab_id) {
                        tab.cancels.insert(req, flag.clone());
                    }
                    let h = self.clone();
                    let path = PathBuf::from(location::display(&folder, self.0.flavor));
                    std::thread::Builder::new()
                        .name("fd-enumerate".into())
                        .spawn(move || h.enumerate_job(tab_id, req, path, flag))
                        .ok();
                }
                Effect::Cancel(req) => {
                    if let Some((_, tab)) = st.tab_mut(tab_id)
                        && let Some(f) = tab.cancels.remove(&req)
                    {
                        f.store(true, Ordering::Relaxed);
                    }
                }
                Effect::Open(path) => {
                    let h = self.clone();
                    std::thread::spawn(move || {
                        let r = fd_portable::ops::open_with_default(&path);
                        h.0.events.log("open", &[path.display().to_string(), result_text(&r)]);
                    });
                }
                Effect::Rename { from, to } => {
                    let h = self.clone();
                    std::thread::spawn(move || {
                        let r = fd_portable::ops::rename(&from, &to);
                        let old = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let new = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        h.0.events.log("rename", &[old, new, result_text(&r)]);
                        if let Err(e) = &r {
                            h.apply_to_tab(tab_id, PaneMsg::Status(format!("이름을 바꾸지 못했습니다: {e}")));
                        }
                        h.refresh_folders(&[from.parent().map(Path::to_path_buf).unwrap_or_default()]);
                    });
                }
                Effect::Transfer { sources, dest, mode } => {
                    let h = self.clone();
                    std::thread::spawn(move || {
                        let mut f = vec![dest.display().to_string(), mode_text(mode).to_string(), sources.len().to_string()];
                        f.extend(sources.iter().map(|p| p.display().to_string()));
                        h.0.events.log("drop", &f);
                        let r = fd_portable::ops::transfer(&sources, &dest, mode == Transfer::Move);
                        if let Err(e) = &r {
                            h.0.events.log("drop-error", &[e.to_string()]);
                        }
                        let mut dirs = vec![dest.clone()];
                        dirs.extend(sources.iter().filter_map(|p| p.parent().map(Path::to_path_buf)));
                        h.refresh_folders(&dirs);
                    });
                }
                Effect::FirstItemReady => {
                    // 보이는 탭만 잰다 — 뒤에 있는 탭의 첫 배치가 FirstItem·ColdStart 를 앞당기면 안 된다
                    if let Some((p, i)) = st.locate(tab_id)
                        && st.panes[p].active == i
                    {
                        out.push(UiEvent::FirstItem(p));
                    }
                }
            }
        }
    }

    fn enumerate_job(&self, tab_id: u64, req: RequestId, path: PathBuf, cancel: Arc<AtomicBool>) {
        let res = fd_portable::enumerate::enumerate(&path, &cancel, |batch| {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            self.apply_to_tab(tab_id, PaneMsg::Batch(req, batch));
            !cancel.load(Ordering::Relaxed)
        });
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        {
            let mut st = self.lock();
            if let Some((_, tab)) = st.tab_mut(tab_id) {
                tab.cancels.remove(&req);
                tab.seen_mtime = mtime;
            }
        }
        self.apply_to_tab(tab_id, PaneMsg::Finished(req, res));
    }

    /// 이 폴더들을 보고 있는 탭을 모두 새로 읽는다 (선택 유지).
    fn refresh_folders(&self, dirs: &[PathBuf]) {
        let want: Vec<String> = dirs.iter().map(|d| d.display().to_string().trim_end_matches('/').to_string()).collect();
        let ids: Vec<u64> = {
            let st = self.lock();
            st.panes
                .iter()
                .flat_map(|p| p.tabs.iter())
                .filter(|t| {
                    let f = t.vm.folder_display();
                    let f = if f.len() > 1 { f.trim_end_matches('/') } else { f.as_str() };
                    want.iter().any(|w| w == f)
                })
                .map(|t| t.id)
                .collect()
        };
        for id in ids {
            self.apply_to_tab(id, PaneMsg::Refresh);
        }
    }

    /// 자동 새로고침: 활성 탭 폴더의 수정 시각이 바뀌면 새로 읽는다 (부록 A-8 끌-2·끌-4).
    /// 파일 감시(inotify) 대신 1초 폴링 — 스파이크 범위에서는 OS 셋에서 똑같이 도는 쪽이 낫다.
    fn poll_loop(&self) {
        loop {
            std::thread::sleep(POLL_INTERVAL);
            let targets: Vec<(u64, PathBuf, Option<SystemTime>)> = {
                let st = self.lock();
                (0..2)
                    .map(|p| st.active(p))
                    .filter(|t| !t.vm.loading() && t.vm.folder().is_some())
                    .map(|t| (t.id, PathBuf::from(t.vm.folder_display()), t.seen_mtime))
                    .collect()
            };
            for (id, path, seen) in targets {
                let now = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                if now.is_some() && now != seen {
                    {
                        let mut st = self.lock();
                        if let Some((_, t)) = st.tab_mut(id) {
                            t.seen_mtime = now;
                        }
                    }
                    if seen.is_some() {
                        self.apply_to_tab(id, PaneMsg::Refresh);
                    }
                }
            }
        }
    }
}

fn tab_title(vm: &PaneVm) -> String {
    let f = vm.folder_display();
    let t = f.trim_end_matches(['/', '\\']);
    match t.rsplit(['/', '\\']).next() {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => if f.is_empty() { "…".into() } else { f },
    }
}

fn modified_text(t: SystemTime) -> String {
    use chrono::{Datelike, Timelike};
    let l: chrono::DateTime<chrono::Local> = t.into();
    format::timestamp_text(l.year(), l.month(), l.day(), l.hour(), l.minute())
}

fn mode_text(m: Transfer) -> &'static str {
    match m {
        Transfer::Copy => "copy",
        Transfer::Move => "move",
    }
}

fn result_text<T>(r: &std::io::Result<T>) -> String {
    match r {
        Ok(_) => "ok".into(),
        Err(e) => format!("err:{e}"),
    }
}

pub fn home_dir() -> String {
    std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| "/".into())
}

pub fn parse_mode(s: &str) -> Transfer {
    if s.eq_ignore_ascii_case("move") { Transfer::Move } else { Transfer::Copy }
}
