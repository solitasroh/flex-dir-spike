//! 자동 측정 순서 (계약 §2, `FD_SPIKE_PERF_AUTO=1`). 순수 상태 기계 — 시각은 ms(f64)로 받고
//! 할 일(`Cmd`)을 돌려준다. Slint·시계·Host 연결은 `app.rs` 가 한다.
//!
//! 1. 왼쪽 칸 첫 탭이 인자 폴더를 다 읽을 때까지 기다린다.
//! 2. `OPEN_REPEATS` 회: `Up` → 다 읽음 → `Navigate(인자)` 시각 t0 → FirstItem 뒤 **그려진 프레임** t1 → `t1-t0`.
//!    다 읽을 때까지 기다린 뒤 다음 회.
//! 3. 자세히 보기, 맨 위, `SETTLE_MS` 쉼 → `SCROLL_SECONDS` 동안 오프셋 = 경과 × `SCROLL_PX_PER_S`.
//!    그려진 프레임 시각의 간격(첫 간격 없음) → `Frame:details`.
//! 4. 큰 아이콘으로 같은 방식 → `Frame:icons`. 5. 종료 코드 0.

use fd_host::{perf_plan, Point, ViewMode};

#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Up,
    Navigate(String),
    SetView(ViewMode),
    ScrollTo(f64),
    /// ms 뒤 `on_settled` 를 불러 달라
    Settle(u64),
    /// 매 프레임 진행(`on_tick`)이 필요한가
    Ticking(bool),
    RecordFirstItem(f64),
    RecordFrames(Point, Vec<f64>),
    Exit(i32),
}

#[derive(Debug)]
enum Phase {
    WaitInitial,
    UpWait { rep: usize },
    NavWait { rep: usize, t0: f64, painted: bool, loaded: bool },
    Settle { view: ViewMode },
    Scroll { view: ViewMode, start: f64, frames: Vec<f64> },
    Done,
}

#[derive(Debug)]
pub struct PerfPlan {
    target: String,
    phase: Phase,
}

pub fn intervals(frames: &[f64]) -> Vec<f64> {
    frames.windows(2).map(|w| w[1] - w[0]).collect()
}

impl PerfPlan {
    pub fn new(target: String) -> Self {
        Self { target, phase: Phase::WaitInitial }
    }

    /// Navigate 를 보냈고 첫 항목 프레임을 아직 못 쟀으면 그 t0.
    pub fn awaiting_first_item(&self) -> Option<f64> {
        match self.phase {
            Phase::NavWait { t0, painted: false, .. } => Some(t0),
            _ => None,
        }
    }

    pub fn is_scrolling(&self) -> bool {
        matches!(self.phase, Phase::Scroll { .. })
    }

    #[cfg(test)]
    pub fn is_done(&self) -> bool {
        matches!(self.phase, Phase::Done)
    }

    pub fn phase_name(&self) -> String {
        match &self.phase {
            Phase::WaitInitial => "wait-initial".into(),
            Phase::UpWait { rep } => format!("up-wait#{rep}"),
            Phase::NavWait { rep, .. } => format!("nav-wait#{rep}"),
            Phase::Settle { view } => format!("settle-{view:?}"),
            Phase::Scroll { view, .. } => format!("scroll-{view:?}"),
            Phase::Done => "done".into(),
        }
    }

    /// 왼쪽 칸 스냅숏이 바뀔 때마다 (현재 폴더, 읽는 중).
    pub fn on_pane(&mut self, folder: &str, loading: bool, now: f64) -> Vec<Cmd> {
        let at_target = folder == self.target;
        match &mut self.phase {
            Phase::WaitInitial if !loading && at_target => {
                self.phase = Phase::UpWait { rep: 0 };
                vec![Cmd::Up]
            }
            Phase::UpWait { rep } if !loading && !at_target && !folder.is_empty() => {
                self.phase = Phase::NavWait { rep: *rep, t0: now, painted: false, loaded: false };
                vec![Cmd::Navigate(self.target.clone())]
            }
            Phase::NavWait { loaded, painted, .. } if !loading && at_target => {
                *loaded = true;
                if *painted { self.advance() } else { vec![] }
            }
            _ => vec![],
        }
    }

    /// FirstItem 뒤 첫 항목이 그려진 프레임 시각.
    pub fn on_first_item_painted(&mut self, now: f64) -> Vec<Cmd> {
        let Phase::NavWait { t0, painted, loaded, .. } = &mut self.phase else { return vec![] };
        if *painted {
            return vec![];
        }
        *painted = true;
        let mut out = vec![Cmd::RecordFirstItem(now - *t0)];
        if *loaded {
            out.extend(self.advance());
        }
        out
    }

    fn advance(&mut self) -> Vec<Cmd> {
        let Phase::NavWait { rep, .. } = self.phase else { return vec![] };
        if rep + 1 < perf_plan::OPEN_REPEATS {
            self.phase = Phase::UpWait { rep: rep + 1 };
            vec![Cmd::Up]
        } else {
            self.settle(ViewMode::Details)
        }
    }

    fn settle(&mut self, view: ViewMode) -> Vec<Cmd> {
        self.phase = Phase::Settle { view };
        vec![Cmd::SetView(view), Cmd::ScrollTo(0.0), Cmd::Settle(perf_plan::SETTLE_MS)]
    }

    pub fn on_settled(&mut self, now: f64) -> Vec<Cmd> {
        let Phase::Settle { view } = self.phase else { return vec![] };
        self.phase = Phase::Scroll { view, start: now, frames: Vec::new() };
        vec![Cmd::ScrollTo(0.0), Cmd::Ticking(true)]
    }

    /// 매 프레임(그리기 직전/직후의 틱). 스크롤 위치를 정하고 끝나면 기록한다.
    pub fn on_tick(&mut self, now: f64) -> Vec<Cmd> {
        let Phase::Scroll { view, start, frames } = &mut self.phase else { return vec![] };
        let elapsed = now - *start;
        if elapsed < perf_plan::SCROLL_SECONDS * 1000.0 {
            return vec![Cmd::ScrollTo(elapsed / 1000.0 * perf_plan::SCROLL_PX_PER_S)];
        }
        let point = if *view == ViewMode::Details { Point::FrameDetails } else { Point::FrameIcons };
        let mut out = vec![Cmd::Ticking(false), Cmd::RecordFrames(point, intervals(frames))];
        if *view == ViewMode::Details {
            out.extend(self.settle(ViewMode::LargeIcons));
        } else {
            self.phase = Phase::Done;
            out.push(Cmd::Exit(0));
        }
        out
    }

    /// 그려진 프레임 시각.
    pub fn on_frame(&mut self, at: f64) {
        if let Phase::Scroll { start, frames, .. } = &mut self.phase
            && at >= *start
        {
            frames.push(at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "/t/big";

    fn open_once(p: &mut PerfPlan, now: &mut f64) -> Vec<Cmd> {
        // Up 이 나간 상태에서: 상위 다 읽음 → Navigate
        assert_eq!(p.on_pane("/t", false, *now), vec![Cmd::Navigate(T.into())]);
        let t0 = *now;
        *now += 3.0;
        assert!(p.on_pane(T, true, *now).is_empty(), "읽는 중");
        *now += 40.0;
        let cmds = p.on_first_item_painted(*now);
        assert_eq!(cmds[0], Cmd::RecordFirstItem(*now - t0));
        *now += 500.0;
        p.on_pane(T, false, *now)
    }

    #[test]
    fn ten_first_item_lines_then_details_then_icons_then_exit() {
        let mut p = PerfPlan::new(T.into());
        let mut now = 0.0;
        assert!(p.on_pane(T, true, now).is_empty(), "처음 읽기를 기다린다");
        assert_eq!(p.on_pane(T, false, now), vec![Cmd::Up]);
        let mut records = 0;
        for rep in 0..perf_plan::OPEN_REPEATS {
            let next = open_once(&mut p, &mut now);
            records += 1;
            if rep + 1 < perf_plan::OPEN_REPEATS {
                assert_eq!(next, vec![Cmd::Up]);
            } else {
                assert_eq!(
                    next,
                    vec![Cmd::SetView(ViewMode::Details), Cmd::ScrollTo(0.0), Cmd::Settle(perf_plan::SETTLE_MS)]
                );
            }
        }
        assert_eq!(records, 10, "FirstItem 정확히 10줄");
        assert_eq!(p.on_settled(now), vec![Cmd::ScrollTo(0.0), Cmd::Ticking(true)]);
        let start = now;
        // 60Hz 로 10초 + α
        let mut last = vec![];
        for i in 0..=620 {
            let t = start + i as f64 * 16.67;
            last = p.on_tick(t);
            p.on_frame(t + 1.0);
            if !p.is_scrolling() {
                break;
            }
            assert_eq!(last, vec![Cmd::ScrollTo((t - start) / 1000.0 * perf_plan::SCROLL_PX_PER_S)]);
        }
        let Cmd::RecordFrames(Point::FrameDetails, v) = &last[1] else { panic!("{last:?}") };
        assert!(v.len() >= 590, "프레임마다 한 줄 ({})", v.len());
        assert!(v.iter().all(|&d| (d - 16.67).abs() < 0.01));
        assert_eq!(last[2], Cmd::SetView(ViewMode::LargeIcons));
        p.on_settled(now + 20_000.0);
        let end = p.on_tick(now + 20_000.0 + perf_plan::SCROLL_SECONDS * 1000.0);
        assert!(matches!(end[1], Cmd::RecordFrames(Point::FrameIcons, _)));
        assert_eq!(end.last(), Some(&Cmd::Exit(0)));
        assert!(p.is_done());
    }

    #[test]
    fn empty_folder_loaded_before_paint_still_advances_once() {
        let mut p = PerfPlan::new(T.into());
        p.on_pane(T, false, 0.0);
        p.on_pane("/t", false, 10.0);
        assert!(p.on_pane(T, false, 12.0).is_empty(), "다 읽었지만 아직 안 그렸다");
        assert_eq!(p.awaiting_first_item(), Some(10.0));
        let c = p.on_first_item_painted(30.0);
        assert_eq!(c, vec![Cmd::RecordFirstItem(20.0), Cmd::Up]);
        assert!(p.on_first_item_painted(40.0).is_empty(), "두 번 재지 않는다");
    }

    #[test]
    fn intervals_drop_the_first_frame() {
        assert_eq!(intervals(&[0.0, 16.0, 33.0]), vec![16.0, 17.0]);
        assert!(intervals(&[5.0]).is_empty());
    }
}
