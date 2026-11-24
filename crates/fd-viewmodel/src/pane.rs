use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::PathBuf;

use fd_core::{location, natural, FileItem, NamePolicy, PathFlavor};

use crate::layout::{Layout, ViewMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RequestId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Type,
    Modified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transfer {
    Copy,
    Move,
}

#[derive(Debug)]
pub enum PaneMsg {
    /// 주소줄 입력 또는 프로그램이 연 폴더.
    Navigate(String),
    /// 더블클릭·Enter.
    Activate(usize),
    Up,
    Refresh,
    Batch(RequestId, Vec<FileItem>),
    Finished(RequestId, Result<(), String>),
    Click { pos: usize, ctrl: bool, shift: bool },
    ClickEmpty,
    SelectAll,
    /// 화살표·Page·Home·End. delta 는 위치 단위 (큰 아이콘의 위/아래 = ±열 수).
    MoveFocus { delta: i64, shift: bool },
    /// 확정된 글자 (IME 조합이 끝난 것만 온다).
    TypeAhead { text: String, now_ms: u64 },
    /// IME 조합 중인 글자 미리 찾기. 버퍼에 넣지 않는다 (확정되면 TypeAhead 로 다시 온다).
    /// `한` 을 조합하는 순간 움직이게 한다 — 엘레나 한-4 "한 글자에서 바로 움직였는지".
    TypeAheadPreview { composing: String, now_ms: u64 },
    BeginRename,
    CommitRename(String),
    CancelRename,
    SetSort(SortKey),
    SetView(ViewMode),
    SetViewport { width: f32, height: f32 },
    /// OS 또는 다른 페인에서 놓았다. target 은 놓은 자리의 항목 위치 (빈 곳이면 None).
    Drop { paths: Vec<PathBuf>, target: Option<usize>, mode: Transfer },
    /// 효과 실행 결과 문구 (상태표시줄).
    Status(String),
}

#[derive(Debug, PartialEq)]
pub enum Effect {
    Enumerate { req: RequestId, folder: String },
    Cancel(RequestId),
    Open(PathBuf),
    Rename { from: PathBuf, to: PathBuf },
    Transfer { sources: Vec<PathBuf>, dest: PathBuf, mode: Transfer },
    /// 폴더 전환 뒤 첫 항목이 VM 에 들어왔다. View 가 그린 시점을 재서 FirstItem 으로 남긴다.
    FirstItemReady,
}

pub struct PaneVm {
    flavor: PathFlavor,
    policy: NamePolicy,
    folder: Option<String>,
    next_req: u64,
    current: Option<RequestId>,
    loading: bool,
    items: Vec<FileItem>,
    /// 위치 → 항목 인덱스
    order: Vec<u32>,
    /// 항목 인덱스 → 위치
    pos_of: Vec<u32>,
    selected: Vec<bool>,
    selected_count: usize,
    anchor: Option<u32>,
    focus: Option<u32>,
    sort: SortKey,
    descending: bool,
    layout: Layout,
    typeahead: String,
    typeahead_at: u64,
    renaming: Option<u32>,
    status: String,
    /// 새로 고침 동안 선택을 유지하려고 들고 있는 동일성 키 (README: "갱신 중에도 선택은 유지된다")
    reselect: HashSet<String>,
    refocus: Option<String>,
    first_item_pending: bool,
    version: u64,
}

const TYPEAHEAD_RESET_MS: u64 = 1000;

impl PaneVm {
    pub fn new(flavor: PathFlavor, policy: NamePolicy) -> Self {
        Self {
            flavor,
            policy,
            folder: None,
            next_req: 0,
            current: None,
            loading: false,
            items: Vec::new(),
            order: Vec::new(),
            pos_of: Vec::new(),
            selected: Vec::new(),
            selected_count: 0,
            anchor: None,
            focus: None,
            sort: SortKey::Name,
            descending: false,
            layout: Layout { view: ViewMode::Details, width: 800.0, height: 600.0 },
            typeahead: String::new(),
            typeahead_at: 0,
            renaming: None,
            status: String::new(),
            reselect: HashSet::new(),
            refocus: None,
            first_item_pending: false,
            version: 0,
        }
    }

    // ---------- 읽기 (View 가 쓴다) ----------
    pub fn version(&self) -> u64 { self.version }
    pub fn folder(&self) -> Option<&str> { self.folder.as_deref() }
    pub fn folder_display(&self) -> String {
        self.folder.as_deref().map(|f| location::display(f, self.flavor)).unwrap_or_default()
    }
    pub fn len(&self) -> usize { self.order.len() }
    pub fn is_empty(&self) -> bool { self.order.is_empty() }
    pub fn loading(&self) -> bool { self.loading }
    pub fn item(&self, pos: usize) -> Option<&FileItem> {
        self.order.get(pos).map(|&i| &self.items[i as usize])
    }
    pub fn is_selected(&self, pos: usize) -> bool {
        self.order.get(pos).is_some_and(|&i| self.selected[i as usize])
    }
    pub fn focus(&self) -> Option<usize> { self.focus.map(|i| self.pos_of[i as usize] as usize) }
    pub fn renaming(&self) -> Option<usize> { self.renaming.map(|i| self.pos_of[i as usize] as usize) }
    pub fn layout(&self) -> Layout { self.layout }
    pub fn sort(&self) -> (SortKey, bool) { (self.sort, self.descending) }
    pub fn status(&self) -> &str { &self.status }
    pub fn selected_count(&self) -> usize { self.selected_count }
    pub fn flavor(&self) -> PathFlavor { self.flavor }
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.order
            .iter()
            .filter(|&&i| self.selected[i as usize])
            .map(|&i| self.items[i as usize].path.clone())
            .collect()
    }

    // ---------- 갱신 ----------
    pub fn update(&mut self, msg: PaneMsg) -> Vec<Effect> {
        let fx = self.apply(msg);
        self.version += 1;
        fx
    }

    fn apply(&mut self, msg: PaneMsg) -> Vec<Effect> {
        match msg {
            PaneMsg::Navigate(input) => match location::parse(&input, self.flavor) {
                Ok(folder) => self.open(folder, false),
                Err(e) => {
                    self.status = format!("열 수 없는 주소: {input} ({e:?})");
                    vec![]
                }
            },
            PaneMsg::Activate(pos) => {
                let Some(item) = self.item(pos) else { return vec![] };
                if item.is_dir {
                    let path = item.path.to_string_lossy().into_owned();
                    match location::parse(&path, self.flavor) {
                        Ok(folder) => self.open(folder, false),
                        Err(e) => {
                            self.status = format!("열 수 없는 폴더 ({e:?})");
                            vec![]
                        }
                    }
                } else {
                    vec![Effect::Open(item.path.clone())]
                }
            }
            PaneMsg::Up => match self.folder.as_deref().and_then(|f| location::parent(f, self.flavor)) {
                Some(p) => {
                    let child = self.folder.clone();
                    let fx = self.open(p, false);
                    // 상위로 가면 방금 있던 폴더에 포커스 (탐색기와 같다)
                    self.refocus = child.and_then(|c| {
                        std::path::Path::new(&location::display(&c, self.flavor))
                            .file_name()
                            .map(|n| self.policy.identity_key(&n.to_string_lossy()))
                    });
                    fx
                }
                None => vec![],
            },
            PaneMsg::Refresh => match self.folder.clone() {
                Some(f) => self.open(f, true),
                None => vec![],
            },
            PaneMsg::Batch(req, batch) => {
                if Some(req) != self.current || batch.is_empty() {
                    return vec![]; // 늦게 온 이전 요청의 배치는 버린다
                }
                let was_empty = self.items.is_empty();
                self.merge(batch);
                if was_empty && self.first_item_pending {
                    self.first_item_pending = false;
                    return vec![Effect::FirstItemReady];
                }
                vec![]
            }
            PaneMsg::Finished(req, result) => {
                if Some(req) == self.current {
                    self.loading = false;
                    self.reselect.clear();
                    self.refocus = None;
                    self.status = match result {
                        Ok(()) => format!("{}개 항목", self.len()),
                        Err(e) => format!("열거 실패: {e}"),
                    };
                    if self.first_item_pending {
                        self.first_item_pending = false; // 빈 폴더
                        return vec![Effect::FirstItemReady];
                    }
                }
                vec![]
            }
            PaneMsg::Click { pos, ctrl, shift } => {
                if pos >= self.len() {
                    return vec![];
                }
                self.renaming = None;
                let idx = self.order[pos];
                if shift {
                    let a = self.anchor.map(|i| self.pos_of[i as usize] as usize).unwrap_or(pos);
                    if !ctrl {
                        self.clear_selection();
                    }
                    let (lo, hi) = if a <= pos { (a, pos) } else { (pos, a) };
                    for p in lo..=hi {
                        self.set_selected(self.order[p], true);
                    }
                } else if ctrl {
                    let now = !self.selected[idx as usize];
                    self.set_selected(idx, now);
                    self.anchor = Some(idx);
                } else {
                    self.clear_selection();
                    self.set_selected(idx, true);
                    self.anchor = Some(idx);
                }
                self.focus = Some(idx);
                vec![]
            }
            PaneMsg::ClickEmpty => {
                self.renaming = None;
                self.clear_selection();
                vec![]
            }
            PaneMsg::SelectAll => {
                for i in 0..self.items.len() {
                    self.selected[i] = true;
                }
                self.selected_count = self.items.len();
                vec![]
            }
            PaneMsg::MoveFocus { delta, shift } => {
                if self.is_empty() {
                    return vec![];
                }
                let cur = self.focus().unwrap_or(0) as i64;
                let next = if self.focus.is_none() { 0 } else { (cur + delta).clamp(0, self.len() as i64 - 1) } as usize;
                self.apply(PaneMsg::Click { pos: next, ctrl: false, shift })
            }
            PaneMsg::TypeAhead { text, now_ms } => {
                if self.is_empty() || text.chars().all(char::is_control) {
                    return vec![];
                }
                if now_ms.saturating_sub(self.typeahead_at) > TYPEAHEAD_RESET_MS {
                    self.typeahead.clear();
                }
                self.typeahead_at = now_ms;
                let fresh = self.typeahead.is_empty();
                self.typeahead.push_str(&text);
                let needle = fd_core::name::sort_key(&self.typeahead);
                let start = self.focus().map(|f| if fresh { f + 1 } else { f }).unwrap_or(0);
                let n = self.len();
                let hit = (0..n)
                    .map(|k| (start + k) % n)
                    .find(|&p| self.items[self.order[p] as usize].sort_key.starts_with(&needle));
                match hit {
                    Some(p) => self.apply(PaneMsg::Click { pos: p, ctrl: false, shift: false }),
                    None => vec![],
                }
            }
            PaneMsg::TypeAheadPreview { composing, now_ms } => {
                if self.is_empty() || composing.is_empty() {
                    return vec![];
                }
                let prefix = if now_ms.saturating_sub(self.typeahead_at) > TYPEAHEAD_RESET_MS { "" } else { self.typeahead.as_str() };
                let needle = format!("{prefix}{composing}");
                let matches = |p: usize| fd_core::name::jamo_prefix(&self.items[self.order[p] as usize].sort_key, &needle);
                // 이미 맞는 항목에 있으면 움직이지 않는다 (조합이 진행돼도 깜빡이지 않게)
                if self.focus().is_some_and(matches) {
                    return vec![];
                }
                let start = self.focus().unwrap_or(0);
                let n = self.len();
                match (0..n).map(|k| (start + k) % n).find(|&p| matches(p)) {
                    Some(p) => self.apply(PaneMsg::Click { pos: p, ctrl: false, shift: false }),
                    None => vec![],
                }
            }
            PaneMsg::BeginRename => {
                if self.selected_count == 1 {
                    self.renaming = self.focus.filter(|&i| self.selected[i as usize]);
                }
                vec![]
            }
            PaneMsg::CancelRename => {
                self.renaming = None;
                vec![]
            }
            PaneMsg::CommitRename(new_name) => {
                let Some(idx) = self.renaming.take() else { return vec![] };
                let new_name = new_name.trim().to_string();
                let item = &self.items[idx as usize];
                if new_name.is_empty() || new_name == item.name {
                    return vec![];
                }
                if new_name.contains('/') || (self.flavor == PathFlavor::Windows && new_name.contains(['\\', ':', '*', '?', '"', '<', '>', '|'])) {
                    self.status = format!("이름에 쓸 수 없는 문자가 있습니다: {new_name}");
                    return vec![];
                }
                let to = item.path.with_file_name(&new_name);
                self.reselect.insert(self.policy.identity_key(&new_name));
                vec![Effect::Rename { from: item.path.clone(), to }]
            }
            PaneMsg::SetSort(key) => {
                if self.sort == key {
                    self.descending = !self.descending;
                } else {
                    self.sort = key;
                    self.descending = false;
                }
                self.resort();
                vec![]
            }
            PaneMsg::SetView(view) => {
                self.layout.view = view;
                vec![]
            }
            PaneMsg::SetViewport { width, height } => {
                self.layout.width = width;
                self.layout.height = height;
                vec![]
            }
            PaneMsg::Drop { paths, target, mode } => {
                let Some(folder) = &self.folder else { return vec![] };
                let dest = match target.and_then(|p| self.item(p)) {
                    Some(it) if it.is_dir => it.path.clone(),
                    _ => PathBuf::from(location::display(folder, self.flavor)),
                };
                // 자기 자신 속으로, 또는 이미 그 폴더에 있는 것을 그 폴더로 — 둘 다 하지 않는다
                let sources: Vec<PathBuf> = paths
                    .into_iter()
                    .filter(|p| p != &dest && p.parent() != Some(dest.as_path()) && !dest.starts_with(p))
                    .collect();
                if sources.is_empty() {
                    return vec![];
                }
                vec![Effect::Transfer { sources, dest, mode }]
            }
            PaneMsg::Status(s) => {
                self.status = s;
                vec![]
            }
        }
    }

    fn open(&mut self, folder: String, keep_selection: bool) -> Vec<Effect> {
        let mut fx = Vec::new();
        if let Some(old) = self.current.take()
            && self.loading
        {
            fx.push(Effect::Cancel(old));
        }
        self.reselect.clear();
        self.refocus = None;
        if keep_selection {
            for (i, it) in self.items.iter().enumerate() {
                if self.selected[i] {
                    self.reselect.insert(self.policy.identity_key(&it.name));
                }
            }
            self.refocus = self.focus.map(|i| self.policy.identity_key(&self.items[i as usize].name));
        } else {
            self.first_item_pending = true;
        }
        self.next_req += 1;
        let req = RequestId(self.next_req);
        self.current = Some(req);
        self.loading = true;
        self.folder = Some(folder.clone());
        self.items.clear();
        self.order.clear();
        self.pos_of.clear();
        self.selected.clear();
        self.selected_count = 0;
        self.anchor = None;
        self.focus = None;
        self.renaming = None;
        self.typeahead.clear();
        self.status = "불러오는 중…".into();
        fx.push(Effect::Enumerate { req, folder });
        fx
    }

    fn cmp(&self, a: u32, b: u32) -> Ordering {
        let (x, y) = (&self.items[a as usize], &self.items[b as usize]);
        // 폴더 먼저 — 방향과 무관
        y.is_dir.cmp(&x.is_dir).then_with(|| {
            let ord = match self.sort {
                SortKey::Name => Ordering::Equal,
                SortKey::Size => x.size.cmp(&y.size),
                SortKey::Type => natural::compare(&x.extension().to_lowercase(), &y.extension().to_lowercase()),
                SortKey::Modified => x.modified.cmp(&y.modified),
            }
            .then_with(|| natural::compare(&x.sort_key, &y.sort_key))
            .then_with(|| x.name.cmp(&y.name)); // 대소문자만 다른 두 이름도 순서가 정해진다
            if self.descending { ord.reverse() } else { ord }
        })
    }

    fn merge(&mut self, batch: Vec<FileItem>) {
        let base = self.items.len() as u32;
        let n = batch.len() as u32;
        for it in batch {
            let key = self.policy.identity_key(&it.name);
            let sel = self.reselect.contains(&key);
            if self.refocus.as_deref() == Some(key.as_str()) {
                self.focus = Some(self.items.len() as u32);
                self.anchor = self.focus;
            }
            self.items.push(it);
            self.selected.push(sel);
            if sel {
                self.selected_count += 1;
            }
        }
        let mut fresh: Vec<u32> = (base..base + n).collect();
        fresh.sort_by(|&a, &b| self.cmp(a, b));
        let old = std::mem::take(&mut self.order);
        let mut merged = Vec::with_capacity(old.len() + fresh.len());
        let (mut i, mut j) = (0, 0);
        while i < old.len() && j < fresh.len() {
            if self.cmp(old[i], fresh[j]) != Ordering::Greater {
                merged.push(old[i]);
                i += 1;
            } else {
                merged.push(fresh[j]);
                j += 1;
            }
        }
        merged.extend_from_slice(&old[i..]);
        merged.extend_from_slice(&fresh[j..]);
        self.order = merged;
        self.rebuild_pos();
    }

    fn resort(&mut self) {
        let mut order = std::mem::take(&mut self.order);
        order.sort_by(|&a, &b| self.cmp(a, b));
        self.order = order;
        self.rebuild_pos();
    }

    fn rebuild_pos(&mut self) {
        self.pos_of.resize(self.items.len(), 0);
        for (p, &i) in self.order.iter().enumerate() {
            self.pos_of[i as usize] = p as u32;
        }
    }

    fn clear_selection(&mut self) {
        self.selected.iter_mut().for_each(|s| *s = false);
        self.selected_count = 0;
    }

    fn set_selected(&mut self, idx: u32, on: bool) {
        let s = &mut self.selected[idx as usize];
        if *s != on {
            *s = on;
            if on { self.selected_count += 1 } else { self.selected_count -= 1 }
        }
    }
}

#[cfg(test)]
mod tests;
