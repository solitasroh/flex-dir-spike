//! 파일 조작 (스파이크 최소판). 충돌 정책은 "이름 (2)" 로 피하기 하나뿐이다 —
//! 진짜 충돌 질의 UI 는 `fd-ops` 몫이다 (tech.md §4.2 #5). 스파이크가 재는 것은 DnD 경로이지 복사 엔진이 아니다.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    if to.exists() && !same_file_case_only(from, to) {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("이미 있습니다: {}", to.display())));
    }
    fs::rename(from, to)
}

fn same_file_case_only(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

/// sources 를 dest 폴더로 복사/이동. 결과: 처리한 개수.
pub fn transfer(sources: &[PathBuf], dest: &Path, move_: bool) -> io::Result<usize> {
    let mut done = 0;
    for src in sources {
        let Some(name) = src.file_name() else { continue };
        let target = free_name(&dest.join(name));
        if move_ {
            if fs::rename(src, &target).is_err() {
                // 다른 볼륨 — 복사 후 원본 삭제
                copy_recursive(src, &target)?;
                if src.is_dir() { fs::remove_dir_all(src)? } else { fs::remove_file(src)? }
            }
        } else {
            copy_recursive(src, &target)?;
        }
        done += 1;
    }
    Ok(done)
}

fn copy_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    let md = fs::symlink_metadata(src)?;
    if md.is_dir() {
        fs::create_dir_all(dst)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_recursive(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(src, dst).map(|_| ())
    }
}

fn free_name(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..).map(|n| p.with_file_name(format!("{stem} ({n}){ext}"))).find(|c| !c.exists()).unwrap()
}

/// 연결 프로그램으로 연다.
pub fn open_with_default(path: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    let mut cmd = std::process::Command::new("xdg-open");
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    };
    cmd.arg(path).spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_avoids_overwrite_and_move_removes_source() {
        let base = std::env::temp_dir().join(format!("fd-ops-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (a, b) = (base.join("a"), base.join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("x.txt"), "1").unwrap();
        fs::write(b.join("x.txt"), "old").unwrap();
        assert_eq!(transfer(&[a.join("x.txt")], &b, false).unwrap(), 1);
        assert_eq!(fs::read_to_string(b.join("x (2).txt")).unwrap(), "1");
        assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "old", "덮어쓰지 않는다");
        transfer(&[a.join("x.txt")], &b, true).unwrap();
        assert!(!a.join("x.txt").exists());
        fs::remove_dir_all(&base).unwrap();
    }
}
