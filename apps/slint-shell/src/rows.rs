//! 10만 항목 가상화 (계약 §3, L3).
//!
//! Slint `ListView` 는 보이는 줄만 만든다. 그 줄이 `row_data` 로 묻는 위치 주변만 `Host::rows` 로
//! 끌어와 들고 있다가(창 = `WINDOW` 항목), 목록이 바뀌면 버리거나(구조 변경) 다시 읽어 달라진 줄만 알린다.
//! 큰 아이콘은 합성 행: 행 r = 항목 r*cols .. r*cols+cols-1 (열 수는 VM `Layout` 이 정한다).
//!
//! 이 파일 앞부분(`RowCache`)은 Slint 를 모르는 순수 코드라 단위 시험이 된다.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use fd_host::{Host, Row};
use slint::{Model, ModelNotify, ModelRc, ModelTracker, SharedString, VecModel};

use crate::{IconRow, ItemData};

/// 한 번에 끌어오는 항목 수. 큰 아이콘 8열 × 보이는 행 6 = 48 항목보다 넉넉하다.
pub const WINDOW: usize = 256;

/// 위치 `pos` 를 덮는 창의 시작. 스크롤은 대개 아래로 가므로 앞 1/4, 뒤 3/4 를 잡는다.
pub fn window_start(pos: usize, total: usize) -> usize {
    pos.saturating_sub(WINDOW / 4).min(total.saturating_sub(WINDOW))
}

/// 큰 아이콘 합성 행 `row` 에 들어가는 항목 위치.
pub fn icon_row_range(row: usize, columns: usize, total: usize) -> Range<usize> {
    let cols = columns.max(1);
    let start = row.saturating_mul(cols).min(total);
    start..(start + cols).min(total)
}

pub fn icon_rows(total: usize, columns: usize) -> usize {
    total.div_ceil(columns.max(1))
}

/// 화면에 영향을 주는 칸이 모두 같은가 (`Row` 는 PartialEq 가 없다 — fd-host 를 건드리지 않으려고 여기서 비교).
pub fn same_row(a: &Row, b: &Row) -> bool {
    a.pos == b.pos
        && a.selected == b.selected
        && a.focused == b.focused
        && a.is_dir == b.is_dir
        && a.name == b.name
        && a.size == b.size
        && a.modified == b.modified
}

#[derive(Default)]
pub struct RowCache {
    start: usize,
    rows: Vec<Row>,
    total: usize,
}

impl RowCache {
    pub fn total(&self) -> usize {
        self.total
    }

    pub fn set_total(&mut self, total: usize) {
        self.total = total;
        let keep = total.saturating_sub(self.start).min(self.rows.len());
        self.rows.truncate(keep);
    }

    pub fn clear(&mut self) {
        self.start = 0;
        self.rows.clear();
    }

    pub fn cached_range(&self) -> Range<usize> {
        self.start..self.start + self.rows.len()
    }

    /// `pos` 의 줄. 창 밖이면 `fetch(start, count)` 로 새 창을 읽는다.
    pub fn get(&mut self, pos: usize, fetch: &mut dyn FnMut(usize, usize) -> Vec<Row>) -> Option<Row> {
        if pos >= self.total {
            return None;
        }
        if !self.cached_range().contains(&pos) {
            self.start = window_start(pos, self.total);
            self.rows = fetch(self.start, WINDOW);
        }
        self.rows.get(pos - self.start).cloned()
    }

    /// 들고 있는 창을 다시 읽고, 내용이 달라진 위치를 돌려준다 (선택·포커스만 바뀐 경우).
    pub fn refresh(&mut self, fetch: &mut dyn FnMut(usize, usize) -> Vec<Row>) -> Vec<usize> {
        if self.rows.is_empty() {
            return Vec::new();
        }
        let fresh = fetch(self.start, self.rows.len());
        let mut changed: Vec<usize> = self
            .rows
            .iter()
            .zip(fresh.iter())
            .filter(|(a, b)| !same_row(a, b))
            .map(|(_, b)| b.pos)
            .collect();
        // 창 끝에서 줄어든 부분도 바뀐 것으로 본다
        changed.extend((self.start + fresh.len())..(self.start + self.rows.len()));
        self.rows = fresh;
        changed
    }
}

// ---------------------------------------------------------------- Slint 모델

pub fn item_data(r: &Row) -> ItemData {
    ItemData {
        pos: r.pos as i32,
        name: SharedString::from(r.name.as_str()),
        is_dir: r.is_dir,
        size: SharedString::from(r.size.as_str()),
        modified: SharedString::from(r.modified.as_str()),
        selected: r.selected,
        focused: r.focused,
    }
}

/// 한 칸(pane)의 활성 탭 목록. 자세히·큰 아이콘 두 모델이 같은 캐시를 본다.
pub struct PaneRows {
    pub pane: usize,
    pub host: Host,
    pub cache: RefCell<RowCache>,
    pub columns: std::cell::Cell<usize>,
    details_notify: ModelNotify,
    icons_notify: ModelNotify,
}

impl PaneRows {
    pub fn new(pane: usize, host: Host) -> Rc<Self> {
        Rc::new(Self {
            pane,
            host,
            cache: RefCell::new(RowCache::default()),
            columns: std::cell::Cell::new(1),
            details_notify: ModelNotify::default(),
            icons_notify: ModelNotify::default(),
        })
    }

    fn row(&self, pos: usize) -> Option<Row> {
        let (host, pane) = (&self.host, self.pane);
        self.cache.borrow_mut().get(pos, &mut |s, n| host.rows(pane, s, n))
    }

    /// 목록 구조가 바뀌었다 (폴더·정렬·개수·보기·열 수·탭). 보이는 줄을 모두 다시 묻게 한다.
    pub fn reset(&self, total: usize, columns: usize) {
        {
            let mut c = self.cache.borrow_mut();
            c.set_total(total);
            c.clear();
        }
        self.columns.set(columns.max(1));
        self.details_notify.reset();
        self.icons_notify.reset();
    }

    /// 선택·포커스만 바뀌었다. 들고 있는 창에서 달라진 줄만 알린다.
    pub fn refresh(&self) {
        let (host, pane) = (&self.host, self.pane);
        let changed = self.cache.borrow_mut().refresh(&mut |s, n| host.rows(pane, s, n));
        let cols = self.columns.get();
        let mut last_icon_row = usize::MAX;
        for pos in changed {
            self.details_notify.row_changed(pos);
            let r = pos / cols;
            if r != last_icon_row {
                self.icons_notify.row_changed(r);
                last_icon_row = r;
            }
        }
    }
}

/// 자세히 보기: 줄 = 항목.
pub struct DetailsModel(pub Rc<PaneRows>);

impl Model for DetailsModel {
    type Data = ItemData;
    fn row_count(&self) -> usize {
        self.0.cache.borrow().total()
    }
    fn row_data(&self, row: usize) -> Option<ItemData> {
        self.0.row(row).map(|r| item_data(&r))
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.0.details_notify
    }
}

/// 큰 아이콘: 줄 = 합성 행 (열 수만큼의 항목).
pub struct IconsModel(pub Rc<PaneRows>);

impl Model for IconsModel {
    type Data = IconRow;
    fn row_count(&self) -> usize {
        icon_rows(self.0.cache.borrow().total(), self.0.columns.get())
    }
    fn row_data(&self, row: usize) -> Option<IconRow> {
        let total = self.0.cache.borrow().total();
        let range = icon_row_range(row, self.0.columns.get(), total);
        if range.is_empty() {
            return None;
        }
        let items: Vec<ItemData> = range.filter_map(|p| self.0.row(p)).map(|r| item_data(&r)).collect();
        Some(IconRow { items: ModelRc::new(VecModel::from(items)) })
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.0.icons_notify
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(total: usize, selected: &[usize]) -> impl FnMut(usize, usize) -> Vec<Row> {
        move |s, n| {
            (s..(s + n).min(total))
                .map(|pos| Row {
                    pos,
                    name: format!("f{pos}"),
                    is_dir: false,
                    size: String::new(),
                    modified: String::new(),
                    selected: selected.contains(&pos),
                    focused: false,
                    ext: String::new(),
                })
                .collect()
        }
    }

    #[test]
    fn window_covers_position_and_stays_in_bounds() {
        assert_eq!(window_start(0, 100_000), 0);
        assert_eq!(window_start(1000, 100_000), 1000 - WINDOW / 4);
        assert_eq!(window_start(99_999, 100_000), 100_000 - WINDOW, "끝에서는 창을 앞으로 당긴다");
        assert_eq!(window_start(5, 10), 0, "창보다 짧은 목록");
    }

    #[test]
    fn cache_fetches_once_per_window_not_per_row() {
        let calls = std::cell::Cell::new(0);
        let mut inner = fake(100_000, &[]);
        let mut fetch = |s: usize, n: usize| {
            calls.set(calls.get() + 1);
            inner(s, n)
        };
        let mut c = RowCache::default();
        c.set_total(100_000);
        for pos in 5000..5100 {
            assert_eq!(c.get(pos, &mut fetch).unwrap().pos, pos);
        }
        assert_eq!(calls.get(), 1, "보이는 줄 100개 = 한 번 읽기");
        assert!(c.get(100_000, &mut fetch).is_none(), "범위 밖");
        c.get(90_000, &mut fetch).unwrap();
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn refresh_reports_only_changed_rows() {
        let mut c = RowCache::default();
        c.set_total(1000);
        c.get(10, &mut fake(1000, &[])).unwrap();
        let changed = c.refresh(&mut fake(1000, &[12, 40]));
        assert_eq!(changed, vec![12, 40]);
        assert!(c.refresh(&mut fake(1000, &[12, 40])).is_empty(), "그대로면 알리지 않는다");
        assert!(c.get(12, &mut fake(1000, &[])).unwrap().selected, "다시 읽은 값을 쓴다");
    }

    #[test]
    fn shrinking_total_drops_rows_past_the_end() {
        let mut c = RowCache::default();
        c.set_total(300);
        c.get(250, &mut fake(300, &[])).unwrap();
        c.set_total(100);
        assert!(c.cached_range().end <= 100);
        assert!(c.get(150, &mut fake(100, &[])).is_none());
    }

    #[test]
    fn icon_rows_split_items_by_columns() {
        assert_eq!(icon_rows(100_000, 8), 12_500);
        assert_eq!(icon_rows(10, 3), 4);
        assert_eq!(icon_row_range(3, 3, 10), 9..10, "마지막 행은 덜 찬다");
        assert_eq!(icon_row_range(0, 0, 10), 0..1, "열 수 0 은 1 로 본다");
        assert!(icon_row_range(5, 3, 10).is_empty());
    }
}
