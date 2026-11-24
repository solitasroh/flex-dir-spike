//! 목록 키보드 표 (계약 §4). 순수 함수 — 단위 시험이 된다.
//!
//! Slint 의 `KeyEvent` 는 **물리 키 코드(code)를 주지 않는다** (1.19 문서: 키 바인딩은 논리 키).
//! 그래서 `text`(논리 키) + 수정키로 판정한다. Linux 한국어 입력은 xkb 배열이 us/kr(글쇠는 라틴 keysym)이고
//! 한글은 IME 가 만들므로, Ctrl+A 는 Qt 에서 `Key_A` → `"a"` 로 온다 (i-slint-backend-qt `qt_key_to_string`).
//! 라틴 keysym 이 아닌 배열(러시아어 등)에서는 Ctrl+A 가 안 잡힐 수 있다 — README 위험 항목.

use slint::platform::Key;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListCmd {
    Move { delta: i64, shift: bool },
    Activate,
    Up,
    Rename,
    Refresh,
    SelectAll,
    Address,
    OtherPane,
    /// 목록에서 Esc — 할 일 없음 (편집 취소는 입력칸이 처리)
    Escape,
    /// 삼킨다 (Ctrl 조합 등 입력칸에 들어가면 안 되는 키)
    Swallow,
}

#[derive(Clone, Copy, Debug)]
pub struct KeyCtx {
    pub icons: bool,
    pub columns: usize,
    /// 한 화면에 보이는 행 수
    pub page_rows: usize,
    pub len: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

fn is(text: &str, k: Key) -> bool {
    let mut it = text.chars();
    it.next() == Some(char::from(k)) && it.next().is_none()
}

/// 키 → 명령. `None` 이면 목록이 처리하지 않는다 (글자는 입력칸이 받아 type-ahead 로 간다).
pub fn map_list_key(text: &str, m: Mods, ctx: KeyCtx) -> Option<ListCmd> {
    let cols = ctx.columns.max(1) as i64;
    let page = (ctx.page_rows.max(1) as i64) * cols;
    let all = ctx.len.max(1) as i64;
    let mv = |delta: i64| Some(ListCmd::Move { delta, shift: m.shift });
    if m.alt && is(text, Key::UpArrow) {
        return Some(ListCmd::Up);
    }
    if m.ctrl && !m.alt {
        // 논리 키. 대문자는 Shift 가 섞였을 때 — 소문자로 맞춘다
        return match text.to_lowercase().as_str() {
            "a" => Some(ListCmd::SelectAll),
            "l" => Some(ListCmd::Address),
            _ => Some(ListCmd::Swallow),
        };
    }
    if is(text, Key::UpArrow) {
        return mv(-cols);
    }
    if is(text, Key::DownArrow) {
        return mv(cols);
    }
    if is(text, Key::LeftArrow) {
        return if ctx.icons { mv(-1) } else { Some(ListCmd::Swallow) };
    }
    if is(text, Key::RightArrow) {
        return if ctx.icons { mv(1) } else { Some(ListCmd::Swallow) };
    }
    if is(text, Key::PageUp) {
        return mv(-page);
    }
    if is(text, Key::PageDown) {
        return mv(page);
    }
    if is(text, Key::Home) {
        return mv(-all);
    }
    if is(text, Key::End) {
        return mv(all);
    }
    if is(text, Key::Return) {
        return Some(ListCmd::Activate);
    }
    if is(text, Key::Backspace) {
        return Some(ListCmd::Up);
    }
    if is(text, Key::F2) {
        return Some(ListCmd::Rename);
    }
    if is(text, Key::F5) {
        return Some(ListCmd::Refresh);
    }
    if is(text, Key::Tab) || is(text, Key::Backtab) {
        return Some(ListCmd::OtherPane);
    }
    if is(text, Key::Escape) {
        return Some(ListCmd::Escape);
    }
    if m.alt || m.meta {
        return Some(ListCmd::Swallow);
    }
    None
}

/// events.log `shortcut` 줄 (키, 동작). 이동 키·Tab 은 남기지 않는다 (계약 §4 표의 `—`).
pub fn shortcut_label(cmd: ListCmd, text: &str, m: Mods) -> Option<(&'static str, &'static str)> {
    Some(match cmd {
        ListCmd::Activate => ("Enter", "activate"),
        ListCmd::Up if m.alt => ("Alt+Up", "up"),
        ListCmd::Up if is(text, Key::Backspace) => ("Backspace", "up"),
        ListCmd::Rename => ("F2", "rename"),
        ListCmd::Refresh => ("F5", "refresh"),
        ListCmd::SelectAll => ("Ctrl+A", "select-all"),
        ListCmd::Address => ("Ctrl+L", "address"),
        _ => return None,
    })
}

/// 목록이 글자로 받아야 하는 키인가 (type-ahead). 제어 문자·Slint 특수 키(사용자 영역 U+F700..)는 아니다.
pub fn is_printable(text: &str) -> bool {
    !text.is_empty() && !text.chars().any(|c| c.is_control() || ('\u{f700}'..='\u{f7ff}').contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(k: Key) -> String {
        char::from(k).to_string()
    }
    const DET: KeyCtx = KeyCtx { icons: false, columns: 1, page_rows: 20, len: 100_000 };
    const ICO: KeyCtx = KeyCtx { icons: true, columns: 8, page_rows: 4, len: 100_000 };
    const NONE: Mods = Mods { ctrl: false, shift: false, alt: false, meta: false };
    const CTRL: Mods = Mods { ctrl: true, shift: false, alt: false, meta: false };

    #[test]
    fn arrows_move_by_columns_in_icons_and_by_one_in_details() {
        assert_eq!(map_list_key(&s(Key::DownArrow), NONE, DET), Some(ListCmd::Move { delta: 1, shift: false }));
        assert_eq!(map_list_key(&s(Key::DownArrow), NONE, ICO), Some(ListCmd::Move { delta: 8, shift: false }));
        assert_eq!(map_list_key(&s(Key::LeftArrow), NONE, ICO), Some(ListCmd::Move { delta: -1, shift: false }));
        assert_eq!(map_list_key(&s(Key::PageDown), NONE, ICO), Some(ListCmd::Move { delta: 32, shift: false }));
        let sh = Mods { shift: true, ..NONE };
        assert_eq!(map_list_key(&s(Key::End), sh, DET), Some(ListCmd::Move { delta: 100_000, shift: true }));
    }

    #[test]
    fn shortcuts_follow_contract_table() {
        assert_eq!(map_list_key(&s(Key::Return), NONE, DET), Some(ListCmd::Activate));
        assert_eq!(map_list_key(&s(Key::Backspace), NONE, DET), Some(ListCmd::Up));
        let alt = Mods { alt: true, ..NONE };
        assert_eq!(map_list_key(&s(Key::UpArrow), alt, DET), Some(ListCmd::Up));
        assert_eq!(shortcut_label(ListCmd::Up, &s(Key::UpArrow), alt), Some(("Alt+Up", "up")));
        assert_eq!(map_list_key(&s(Key::F2), NONE, DET), Some(ListCmd::Rename));
        assert_eq!(map_list_key(&s(Key::F5), NONE, DET), Some(ListCmd::Refresh));
        assert_eq!(map_list_key(&s(Key::Tab), NONE, DET), Some(ListCmd::OtherPane));
        assert_eq!(map_list_key(&s(Key::Backtab), Mods { shift: true, ..NONE }, DET), Some(ListCmd::OtherPane));
    }

    #[test]
    fn ctrl_letters_are_shortcuts_never_text() {
        // 한글 상태여도 Qt 는 Ctrl+A 를 Key_A → "a" 로 준다
        assert_eq!(map_list_key("a", CTRL, DET), Some(ListCmd::SelectAll));
        assert_eq!(map_list_key("A", Mods { shift: true, ..CTRL }, DET), Some(ListCmd::SelectAll));
        assert_eq!(map_list_key("l", CTRL, DET), Some(ListCmd::Address));
        assert_eq!(map_list_key("v", CTRL, DET), Some(ListCmd::Swallow), "붙여넣기가 숨은 입력칸에 들어가지 않게");
        assert_eq!(shortcut_label(ListCmd::SelectAll, "a", CTRL), Some(("Ctrl+A", "select-all")));
    }

    #[test]
    fn plain_letters_go_to_type_ahead() {
        assert_eq!(map_list_key("g", NONE, DET), None);
        assert_eq!(map_list_key("한", NONE, DET), None);
        assert!(is_printable("한"));
        assert!(is_printable(" "));
        assert!(!is_printable(&s(Key::UpArrow)));
        assert!(!is_printable("\u{8}"));
    }
}
