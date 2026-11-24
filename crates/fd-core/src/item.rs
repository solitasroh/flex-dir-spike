use std::time::SystemTime;

/// 목록 항목. 경로는 OS 바이트를 잃지 않게 `std::path::PathBuf` 로 든다 (브라이언 §4: OsString 계열).
#[derive(Clone, Debug)]
pub struct FileItem {
    /// 표시용 이름 (비 UTF-8 은 lossy).
    pub name: String,
    pub path: std::path::PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// 미리 계산한 정렬 키 (`name::sort_key`). 10만 항목 정렬에서 매 비교마다 정규화하지 않게.
    pub sort_key: String,
}

impl FileItem {
    pub fn new(path: std::path::PathBuf, is_dir: bool, size: u64, modified: Option<SystemTime>) -> Self {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let sort_key = crate::name::sort_key(&name);
        Self { name, path, is_dir, size, modified, sort_key }
    }

    /// 확장자 (선행 `.` 은 확장자가 아니다 — `.bashrc`).
    pub fn extension(&self) -> &str {
        match self.name.rfind('.') {
            Some(0) | None => "",
            Some(i) => &self.name[i + 1..],
        }
    }
}
