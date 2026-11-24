//! perf.log 한 줄 = 탭 구분 **정확히 5칸** (C#판 PerformanceLog, 브라이언 judge.py 와 같은 모양).
//! `시각\t지점\tms\t예산ms\tok|over`
//! 프레임은 한 프레임에 한 줄, 간격 원시값(소수 2자리). 요약값은 쓰지 않는다 — 판정은 QA 가 원시 줄에서 한다.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Point {
    ColdStart,
    FirstItem,
    FrameDetails,
    FrameIcons,
}

impl Point {
    pub fn budget_ms(self) -> f64 {
        match self {
            Point::ColdStart => 1500.0,
            Point::FirstItem => 150.0,
            Point::FrameDetails | Point::FrameIcons => 16.7,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Point::ColdStart => "ColdStart",
            Point::FirstItem => "FirstItem",
            Point::FrameDetails => "Frame:details",
            Point::FrameIcons => "Frame:icons",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [Point::ColdStart, Point::FirstItem, Point::FrameDetails, Point::FrameIcons].into_iter().find(|p| p.name() == s)
    }
}

/// `at` 은 호출자가 만든 시각 문자열. 순수 함수라 테스트가 된다.
/// 탭·줄바꿈이 섞이면 칸 수가 깨지므로 시각 문자열에서 걸러낸다.
pub fn line(at: &str, point: Point, elapsed_ms: f64) -> String {
    let ms = if elapsed_ms.is_finite() { elapsed_ms.max(0.0) } else { f64::MAX };
    let budget = point.budget_ms();
    let verdict = if ms <= budget { "ok" } else { "over" };
    let at: String = at.chars().filter(|c| !c.is_control()).collect();
    format!("{at}\t{}\t{ms:.2}\t{budget}\t{verdict}", point.name())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_has_exactly_five_tab_separated_columns() {
        let l = line("2026-11-25T10:00:00+00:00", Point::FirstItem, 14.8);
        assert_eq!(l, "2026-11-25T10:00:00+00:00\tFirstItem\t14.80\t150\tok");
        let f = line("t\tx", Point::FrameIcons, 33.333);
        assert_eq!(f.split('\t').count(), 5);
        assert_eq!(f, "tx\tFrame:icons\t33.33\t16.7\tover");
        assert_eq!(Point::parse("Frame:details"), Some(Point::FrameDetails));
        assert_eq!(Point::parse("Frame:other"), None);
    }
}
