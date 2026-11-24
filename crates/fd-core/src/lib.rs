//! fd-core 스파이크 최소판. OS·GUI 를 모른다 (tech.md §5 R1).
//!
//! 브라이언 qa.md §1.4 의 결함 4건을 처음부터 막는 것이 이 크레이트의 존재 이유다.
//! - POSIX 절대 경로를 '상대 경로' 로 거부하지 않는다 (`location`).
//! - 대소문자 구분 디스크에서 `a.txt`·`A.txt` 를 합치지 않는다 (`name::NamePolicy`).
//! - NFD 한글을 NFC 와 같은 자리로 정렬하고 같은 이름으로 판정한다 (`name`, `natural`).
pub mod format;
pub mod item;
pub mod location;
pub mod name;
pub mod natural;
pub mod perf;

pub use item::FileItem;
pub use location::{LocationError, PathFlavor};
pub use name::NamePolicy;
pub use perf::Point;
