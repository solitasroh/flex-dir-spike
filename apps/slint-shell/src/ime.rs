//! 한글 입력 규칙 (계약 §5, 엘레나 design §3-5). 순수 코드 — 시각은 ms 로 받는다.
//!
//! Qt 백엔드에서 IME 는 `QInputMethodEvent(commit, preedit)` 로 오고, Slint `TextInput` 은 조합 글자를
//! `preedit-text`(내부 속성) 에 따로 든다. 한국어 입력기는 조합 중 Enter 에서 보통 "확정 → Enter 전달" 순서지만
//! 입력기·프레임워크에 따라 Enter 가 확정보다 먼저 올 수 있다. 두 순서 모두 **정확히 한 번, 최종 값으로** 실행한다.

/// Enter 를 받았는데 확정이 안 오면 이만큼 기다린 뒤 (값 + 남은 조합 글자) 로 실행한다.
pub const COMMIT_TIMEOUT_MS: u64 = 300;
/// 편집 확정 직후 같은 Enter 가 목록 Activate 로 새지 않게 막는 시간 (계약 §5).
pub const ENTER_ECHO_MS: u64 = 150;
/// 편집이 끝난 뒤 목록 type-ahead 가 IME 잔여 글자를 받지 않는 시간.
pub const GUARD_MS: u64 = 300;
/// 조합 중에 편집이 끝났으면(Esc·다른 곳 클릭) 더 길게 막는다 — 잔여 조합(`소`, `ㅅ`)이 늦게 확정될 수 있다.
pub const GUARD_LEFTOVER_MS: u64 = 1000;

#[derive(Debug, PartialEq, Eq)]
pub enum Gate {
    /// 지금 이 값으로 한 번 실행한다.
    Run(String),
    /// 조합 확정을 기다린다 (예약).
    Wait,
    /// 이미 실행했거나 예약돼 있다 — 무시.
    Ignore,
}

/// 입력칸 하나(이름 바꾸기 또는 주소줄)의 한 번 편집.
#[derive(Debug, Default)]
pub struct EnterGate {
    pending: Option<(u64, String)>,
    done: bool,
}

impl EnterGate {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub fn is_done(&self) -> bool {
        self.done
    }

    pub fn enter(&mut self, text: &str, preedit: &str, now_ms: u64) -> Gate {
        if self.done || self.pending.is_some() {
            return Gate::Ignore;
        }
        if preedit.is_empty() {
            self.done = true;
            Gate::Run(text.to_string())
        } else {
            self.pending = Some((now_ms, preedit.to_string()));
            Gate::Wait
        }
    }

    /// 조합 글자가 바뀌었다. 예약이 있고 조합이 끝났으면(확정됨) 지금 값으로 실행한다.
    pub fn preedit_changed(&mut self, text: &str, preedit: &str) -> Option<String> {
        if self.done || !preedit.is_empty() || self.pending.is_none() {
            return None;
        }
        self.pending = None;
        self.done = true;
        Some(text.to_string())
    }

    /// 확정이 끝내 오지 않았다. 남은 조합 글자를 붙여 한 번 실행한다 (`회` + `의`).
    pub fn timeout(&mut self, text: &str, preedit_now: &str, now_ms: u64) -> Option<String> {
        let (at, at_enter) = self.pending.as_ref()?;
        if self.done || now_ms.saturating_sub(*at) < COMMIT_TIMEOUT_MS {
            return None;
        }
        let tail = if preedit_now.is_empty() { at_enter.clone() } else { preedit_now.to_string() };
        self.pending = None;
        self.done = true;
        Some(format!("{text}{tail}"))
    }

    /// Esc·포커스 잃음 — 실행하지 않고 닫는다.
    pub fn cancel(&mut self) -> bool {
        let was_open = !self.done;
        self.pending = None;
        self.done = true;
        was_open
    }
}

/// 편집이 끝난 뒤 목록 쪽 보호.
#[derive(Debug, Default)]
pub struct ImeGuard {
    until_ms: u64,
    last_commit_ms: Option<u64>,
}

impl ImeGuard {
    pub fn edit_ended(&mut self, now_ms: u64, leftover_preedit: bool, committed: bool) {
        let span = if leftover_preedit { GUARD_LEFTOVER_MS } else { GUARD_MS };
        self.until_ms = self.until_ms.max(now_ms + span);
        if committed {
            self.last_commit_ms = Some(now_ms);
        }
    }

    /// 목록 type-ahead 가 지금 글자를 받아도 되는가.
    pub fn typing_blocked(&self, now_ms: u64) -> bool {
        now_ms < self.until_ms
    }

    /// 확정 직후의 Enter (목록 Activate 로 새면 안 된다).
    pub fn enter_is_echo(&self, now_ms: u64) -> bool {
        self.last_commit_ms.is_some_and(|t| now_ms.saturating_sub(t) < ENTER_ECHO_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_after_commit_runs_once_with_full_text() {
        // 확정 → Enter 순서 (ibus-hangul 흔한 경우): Enter 시점에 조합이 이미 비었다
        let mut g = EnterGate::new();
        assert_eq!(g.enter("회의.txt", "", 0), Gate::Run("회의.txt".into()));
        assert_eq!(g.enter("회의.txt", "", 5), Gate::Ignore, "두 번째 Enter 는 무시");
        assert_eq!(g.preedit_changed("회의.txt", ""), None);
    }

    #[test]
    fn enter_before_commit_waits_for_commit_then_runs_once() {
        // Enter → 확정 순서: Enter 때 `의` 가 아직 조합 중 → 예약 → 확정 뒤 최종 값으로 한 번
        let mut g = EnterGate::new();
        assert_eq!(g.enter("회", "의", 0), Gate::Wait);
        assert_eq!(g.enter("회", "의", 1), Gate::Ignore, "IME 가 Enter 를 두 번 보내도 한 번");
        assert_eq!(g.preedit_changed("회", "의"), None, "아직 조합 중");
        assert_eq!(g.preedit_changed("회의", ""), Some("회의".into()));
        assert_eq!(g.timeout("회의", "", 1000), None, "이미 실행함 — 겹침(회의의) 금지");
    }

    #[test]
    fn missing_commit_falls_back_to_text_plus_preedit() {
        let mut g = EnterGate::new();
        assert_eq!(g.enter("회", "의", 100), Gate::Wait);
        assert_eq!(g.timeout("회", "의", 200), None, "아직 기다린다");
        assert_eq!(g.timeout("회", "의", 100 + COMMIT_TIMEOUT_MS), Some("회의".into()), "빠짐(회) 금지");
        assert!(g.is_done());
    }

    #[test]
    fn escape_cancels_even_while_composing() {
        let mut g = EnterGate::new();
        assert!(g.cancel());
        assert_eq!(g.enter("취소", "", 0), Gate::Ignore);
        assert_eq!(g.preedit_changed("취소", ""), None);
        assert!(!g.cancel(), "두 번 닫지 않는다");
    }

    #[test]
    fn guard_blocks_leftover_typing_and_enter_echo() {
        let mut g = ImeGuard::default();
        g.edit_ended(1000, true, false);
        assert!(g.typing_blocked(1000 + GUARD_MS));
        assert!(!g.typing_blocked(1000 + GUARD_LEFTOVER_MS));
        assert!(!g.enter_is_echo(1001), "취소는 Enter 를 막지 않는다");
        g.edit_ended(5000, false, true);
        assert!(g.enter_is_echo(5000 + ENTER_ECHO_MS - 1));
        assert!(!g.enter_is_echo(5000 + ENTER_ECHO_MS));
        assert!(!g.typing_blocked(5000 + GUARD_MS));
    }
}
