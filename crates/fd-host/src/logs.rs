use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use fd_core::perf::{line, Point};

/// 로컬 시각 `2026-11-25T21:03:04.123+09:00`
pub fn now_local_text() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string()
}

fn append(path: &Path, lines: &[String]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // 계측 실패로 앱이 죽으면 안 된다 (C# PerformanceLog 와 같은 판단). 대신 stderr 에 남긴다.
    match OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut f) => {
            let mut buf = String::new();
            for l in lines {
                buf.push_str(l);
                buf.push('\n');
            }
            if let Err(e) = f.write_all(buf.as_bytes()) {
                eprintln!("[fd-host] 기록 실패 {}: {e}", path.display());
            }
        }
        Err(e) => eprintln!("[fd-host] 기록 파일을 열 수 없음 {}: {e}", path.display()),
    }
}

/// perf.log. `enabled == false` (손 확인 세션) 이면 아무것도 쓰지 않는다.
pub struct PerfLog {
    path: PathBuf,
    enabled: bool,
    lock: Mutex<()>,
}

impl PerfLog {
    pub fn new(out_dir: &Path, enabled: bool) -> Self {
        Self { path: out_dir.join("perf.log"), enabled, lock: Mutex::new(()) }
    }

    pub fn record(&self, point: Point, ms: f64) {
        self.record_many(point, &[ms]);
    }

    pub fn record_many(&self, point: Point, samples: &[f64]) {
        if !self.enabled || samples.is_empty() {
            return;
        }
        let at = now_local_text();
        let lines: Vec<String> = samples.iter().map(|&ms| line(&at, point, ms)).collect();
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        append(&self.path, &lines);
        eprintln!("[perf] {} x{} (첫 값 {:.2}ms)", point.name(), samples.len(), samples[0]);
    }
}

/// events.log — 탭 구분, 한 줄에 한 사건: `시각\t종류\t칸…`. 대표님 ✅/❌ 를 기계 기록으로 대조하는 데 쓴다.
pub struct EventLog {
    path: PathBuf,
    lock: Mutex<()>,
}

impl EventLog {
    pub fn new(out_dir: &Path) -> Self {
        Self { path: out_dir.join("events.log"), lock: Mutex::new(()) }
    }

    pub fn log(&self, kind: &str, fields: &[String]) {
        let mut l = now_local_text();
        l.push('\t');
        l.push_str(kind);
        for f in fields {
            l.push('\t');
            l.extend(f.chars().map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c }));
        }
        eprintln!("[event] {l}");
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        append(&self.path, &[l]);
    }
}
