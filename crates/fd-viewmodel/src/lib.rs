//! fd-viewmodel 스파이크 최소판 — `update(msg) -> Vec<Effect>`. async 도 I/O 도 없다 (tech.md §4.1).
//! 두 후보(Tauri·Slint)가 **같은 이 코드**를 쓴다. 스택 차이는 View 에만 있다.
mod layout;
mod pane;

pub use layout::{Layout, ViewMode, DETAILS_ROW_H, ICON_GAP, ICON_TILE_H, ICON_TILE_W};
pub use pane::{Effect, PaneMsg, PaneVm, RequestId, SortKey, Transfer};
