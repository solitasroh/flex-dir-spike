use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use fd_core::FileItem;

/// 첫 배치는 작게 (첫 항목을 빨리 그리려고), 이후는 크게 (채널 왕복·병합 횟수를 줄이려고).
pub const FIRST_BATCH: usize = 256;
pub const NEXT_BATCH: usize = 4096;

/// `folder` 를 열거해 배치마다 `sink` 에 넘긴다. `sink` 가 false 를 내거나 `cancel` 이 서면 멈춘다.
/// UI 스레드에서 부르지 않는다 (CLAUDE.md §3) — 호출자가 워커 스레드를 띄운다.
pub fn enumerate(folder: &Path, cancel: &AtomicBool, mut sink: impl FnMut(Vec<FileItem>) -> bool) -> Result<(), String> {
    let rd = fs::read_dir(folder).map_err(|e| e.to_string())?;
    let mut batch = Vec::with_capacity(FIRST_BATCH);
    let mut limit = FIRST_BATCH;
    for entry in rd {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let Ok(entry) = entry else { continue }; // 열거 중 사라진 항목 하나 때문에 전체를 실패시키지 않는다
        let path = entry.path();
        let ft = entry.file_type().ok();
        let md = if ft.is_some_and(|t| t.is_symlink()) { fs::metadata(&path).ok() } else { entry.metadata().ok() };
        let is_dir = md.as_ref().is_some_and(|m| m.is_dir());
        let size = md.as_ref().map_or(0, |m| if m.is_dir() { 0 } else { m.len() });
        let modified = md.as_ref().and_then(|m| m.modified().ok());
        batch.push(FileItem::new(path, is_dir, size, modified));
        if batch.len() >= limit {
            if !sink(std::mem::take(&mut batch)) {
                return Ok(());
            }
            limit = NEXT_BATCH;
            batch.reserve(limit);
        }
    }
    if !batch.is_empty() {
        sink(batch);
    }
    Ok(())
}
