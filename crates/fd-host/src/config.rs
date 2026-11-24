use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    /// "A" | "B" (브라이언 fd-check.sh 가 준다). 없으면 스택 이름에서 고른다.
    pub candidate: String,
    /// "tauri" | "slint"
    pub stack: &'static str,
    pub out_dir: PathBuf,
    pub perf_auto: bool,
    /// 명령줄 폴더 (왼쪽, 오른쪽)
    pub folders: Vec<String>,
}

impl Config {
    pub fn from_env(stack: &'static str) -> Self {
        let candidate = std::env::var("FD_SPIKE_CANDIDATE")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| if stack == "tauri" { "A".into() } else { "B".into() });
        let out_dir = std::env::var_os("FD_SPIKE_OUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| default_data_dir().join("out"));
        let perf_auto = std::env::var("FD_SPIKE_PERF_AUTO").is_ok_and(|v| v == "1");
        // 인자 중 옵션처럼 생긴 것(-psn_… 등 OS 가 붙이는 것)은 건너뛴다
        let folders = std::env::args().skip(1).filter(|a| !a.starts_with('-')).take(2).collect();
        Self { candidate, stack, out_dir, perf_auto, folders }
    }

    pub fn window_title(&self) -> String {
        let name = if self.stack == "tauri" { "Tauri" } else { "Slint" };
        let auto = if self.perf_auto { " · 자동 측정 중" } else { "" };
        format!("시험 앱 {} — {name} (flex-dir spike){auto}", self.candidate)
    }
}

/// 사용자 데이터 폴더. C#판 `%APPDATA%\flex-dir` 과 겹치지 않는다 (tech.md §8).
pub fn default_data_dir() -> PathBuf {
    const APP_DIR: &str = "flex-dir-rs-spike";
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.unwrap_or_else(std::env::temp_dir).join(APP_DIR)
}
