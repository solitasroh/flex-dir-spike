//! OS 공통 구현 (tech.md §5 `fd-portable`). 스파이크에서는 열거·조작·메모리 계측을 둔다.
//! 시험 앱의 View 는 이 크레이트를 직접 부르지 않는다 — `fd-host` 의 효과 실행기만 부른다.
pub mod enumerate;
pub mod ops;
pub mod procmem;
