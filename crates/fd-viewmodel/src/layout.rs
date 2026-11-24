//! 뷰 기하. 열 수·행 수·좌표 → 항목 판정을 VM 이 한다 (C# ADR-016 과 같은 경계).
//! View 는 뷰포트 크기만 밀고, 여기서 나온 숫자로 그린다. 그래야 이 규칙이 테스트로 채점된다.

/// DESIGN.md §2
pub const DETAILS_ROW_H: f32 = 24.0;
pub const ICON_TILE_W: f32 = 116.0;
pub const ICON_TILE_H: f32 = 140.0;
pub const ICON_GAP: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    Details,
    LargeIcons,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub view: ViewMode,
    pub width: f32,
    pub height: f32,
}

impl Layout {
    /// 한 행에 놓이는 항목 수 (자세히 = 1).
    pub fn columns(&self) -> usize {
        match self.view {
            ViewMode::Details => 1,
            ViewMode::LargeIcons => (((self.width - ICON_GAP).max(0.0) / (ICON_TILE_W + ICON_GAP)) as usize).max(1),
        }
    }

    pub fn row_height(&self) -> f32 {
        match self.view {
            ViewMode::Details => DETAILS_ROW_H,
            ViewMode::LargeIcons => ICON_TILE_H + ICON_GAP,
        }
    }

    /// 항목 n 개를 그릴 때의 행 수 (합성 행).
    pub fn rows(&self, n: usize) -> usize {
        n.div_ceil(self.columns())
    }

    /// 콘텐츠 좌표(스크롤 반영) → 항목 위치. 항목 위가 아니면 None (빈 곳 = 현재 폴더).
    pub fn hit(&self, x: f32, y: f32, n: usize) -> Option<usize> {
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let row = (y / self.row_height()) as usize;
        let pos = match self.view {
            ViewMode::Details => row,
            ViewMode::LargeIcons => {
                let x_in = x - ICON_GAP;
                if x_in < 0.0 {
                    return None;
                }
                let col = (x_in / (ICON_TILE_W + ICON_GAP)) as usize;
                let within = x_in - col as f32 * (ICON_TILE_W + ICON_GAP);
                if col >= self.columns() || within > ICON_TILE_W {
                    return None;
                }
                row * self.columns() + col
            }
        };
        (pos < n).then_some(pos)
    }

    /// 항목 위치 → 그 항목이 있는 행 (포커스 따라가기).
    pub fn row_of(&self, pos: usize) -> usize {
        pos / self.columns()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_columns_follow_width_and_never_zero() {
        let l = |w| Layout { view: ViewMode::LargeIcons, width: w, height: 600.0 };
        assert_eq!(l(0.0).columns(), 1);
        assert_eq!(l(8.0 + 116.0 + 8.0).columns(), 1);
        assert_eq!(l(8.0 + 2.0 * 124.0).columns(), 2);
        assert_eq!(l(1000.0).columns(), 8, "(1000-8)/124 = 8");
        assert_eq!(l(1000.0).rows(100_000), 12_500);
    }

    #[test]
    fn hit_maps_points_to_items_and_gaps_to_none() {
        let d = Layout { view: ViewMode::Details, width: 800.0, height: 600.0 };
        assert_eq!(d.hit(10.0, 0.0, 3), Some(0));
        assert_eq!(d.hit(10.0, 24.0 * 2.5, 3), Some(2));
        assert_eq!(d.hit(10.0, 24.0 * 3.5, 3), None, "목록 끝 아래는 빈 곳");
        let i = Layout { view: ViewMode::LargeIcons, width: 8.0 + 3.0 * 124.0, height: 600.0 };
        assert_eq!(i.columns(), 3);
        assert_eq!(i.hit(8.0 + 124.0 + 10.0, 148.0 + 5.0, 10), Some(4));
        assert_eq!(i.hit(8.0 + 116.0 + 3.0, 5.0, 10), None, "타일 사이 간격");
    }
}
