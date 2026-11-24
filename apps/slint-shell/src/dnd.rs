//! 끌어다 놓기 규칙 (계약 §6). 순수 코드.
//!
//! Slint `DragArea` 는 수정키가 없을 때 "허용된 것 중 move → copy → link 순서의 첫째" 를 기본으로 제안하고,
//! Qt 백엔드는 그 값을 `QDrag::exec(actions, default)` 에 그대로 넘긴다 (i-slint-backend-qt `start_drag`).
//! copy 와 move 를 함께 허용하면 Nautilus 가 같은 디스크에서 이동을 고른다 → 끌-3 ❌.
//! 그래서 **끌기 시작 순간의 Shift** 로 하나만 허용한다.

use std::path::PathBuf;

use fd_host::Transfer;

/// Slint DragAction 을 .slint 와 Rust 사이에서 정수로 주고받는다 (DragAction 은 공개 Rust API 가 아니다).
pub const ACT_NONE: i32 = 0;
pub const ACT_COPY: i32 = 1;
pub const ACT_MOVE: i32 = 2;
pub const ACT_LINK: i32 = 3;

/// (allow-copy, allow-move). Shift = move 만, 아니면 copy 만.
pub fn allowed_actions(shift_at_start: bool) -> (bool, bool) {
    (!shift_at_start, shift_at_start)
}

pub fn mode_for_shift(shift_at_start: bool) -> Transfer {
    if shift_at_start { Transfer::Move } else { Transfer::Copy }
}

/// 놓는 쪽: OS 가 알려 준 제안이 move 면 이동, 그 밖(copy·link·none)은 복사 (기본 복사).
pub fn drop_action(proposed: i32) -> i32 {
    if proposed == ACT_MOVE { ACT_MOVE } else { ACT_COPY }
}

pub fn transfer_of(action: i32) -> Option<Transfer> {
    match action {
        ACT_COPY => Some(Transfer::Copy),
        ACT_MOVE => Some(Transfer::Move),
        _ => None,
    }
}

pub fn action_name(action: i32) -> &'static str {
    match action {
        ACT_COPY => "copy",
        ACT_MOVE => "move",
        ACT_LINK => "link",
        _ => "none",
    }
}

/// 끌기 결과 → `drag_out_finished` 의 result 칸.
pub fn drag_result(action: i32) -> &'static str {
    if action == ACT_NONE { "cancel" } else { "dropped" }
}

/// 탭 끌기 짐. Qt 백엔드는 OS 끌기(QMimeData)를 거치므로 Rust 객체를 실을 수 없다 → 글자로 싣는다.
pub fn tab_payload(pane: usize, idx: usize) -> String {
    format!("fd-spike-tab:{pane}:{idx}")
}

pub fn parse_tab_payload(s: &str) -> Option<(usize, usize)> {
    let rest = s.strip_prefix("fd-spike-tab:")?;
    let (p, i) = rest.split_once(':')?;
    let p: usize = p.parse().ok()?;
    (p < 2).then_some(())?;
    Some((p, i.trim().parse().ok()?))
}

/// 탭 줄의 x(논리 px) → 끼울 자리. 탭 너비는 고정 (ui 의 TAB-W 와 같은 값).
pub fn tab_insert_index(x: f32, tab_w: f32, n_tabs: usize) -> usize {
    if x <= 0.0 || tab_w <= 0.0 {
        return 0;
    }
    ((x / tab_w).round() as usize).min(n_tabs)
}

/// 놓인 경로가 이 앱이 방금 끌기 시작한 것인가 (Qt 는 앱 안 끌기도 QMimeData 로 돌아온다).
pub fn same_paths(a: &[PathBuf], b: &[PathBuf]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut x: Vec<&PathBuf> = a.iter().collect();
    let mut y: Vec<&PathBuf> = b.iter().collect();
    x.sort();
    y.sort();
    x == y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_offers_exactly_one_action() {
        assert_eq!(allowed_actions(false), (true, false), "기본 = copy 만");
        assert_eq!(allowed_actions(true), (false, true), "Shift = move 만");
        assert_eq!(mode_for_shift(true), Transfer::Move);
    }

    #[test]
    fn drop_defaults_to_copy_unless_os_says_move() {
        assert_eq!(drop_action(ACT_MOVE), ACT_MOVE);
        assert_eq!(drop_action(ACT_COPY), ACT_COPY);
        assert_eq!(drop_action(ACT_LINK), ACT_COPY);
        assert_eq!(drop_action(ACT_NONE), ACT_COPY);
        assert_eq!(transfer_of(ACT_LINK), None);
        assert_eq!(drag_result(ACT_NONE), "cancel");
        assert_eq!(drag_result(ACT_MOVE), "dropped");
    }

    #[test]
    fn tab_payload_round_trips_and_rejects_noise() {
        assert_eq!(parse_tab_payload(&tab_payload(1, 3)), Some((1, 3)));
        assert_eq!(parse_tab_payload("fd-spike-tab:2:0"), None, "칸은 0·1 뿐");
        assert_eq!(parse_tab_payload("/home/u/a.txt"), None);
        assert_eq!(tab_insert_index(10.0, 150.0, 2), 0);
        assert_eq!(tab_insert_index(80.0, 150.0, 2), 1);
        assert_eq!(tab_insert_index(9999.0, 150.0, 2), 2);
    }

    #[test]
    fn internal_drop_is_recognised_regardless_of_order() {
        let a = vec![PathBuf::from("/t/a"), PathBuf::from("/t/b")];
        let b = vec![PathBuf::from("/t/b"), PathBuf::from("/t/a")];
        assert!(same_paths(&a, &b));
        assert!(!same_paths(&a, &b[..1]));
    }
}
