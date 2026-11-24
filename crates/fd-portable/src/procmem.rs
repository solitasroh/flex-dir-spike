//! 메모리 — **프로세스 트리 합계** (tech.md P4: WebView2/WebKit 자식 프로세스 포함).
//! Linux 만 구현한다 (/proc). 다른 OS 는 None — 스파이크 D4 에서 채운다.

/// 자기 자신과 모든 자손 프로세스의 RSS 합 (KiB).
pub fn tree_rss_kib() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        use std::collections::HashMap;
        let me = std::process::id();
        let mut parent: HashMap<u32, u32> = HashMap::new();
        let mut rss: HashMap<u32, u64> = HashMap::new();
        for e in std::fs::read_dir("/proc").ok()?.flatten() {
            let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
            let Ok(status) = std::fs::read_to_string(e.path().join("status")) else { continue };
            for l in status.lines() {
                if let Some(v) = l.strip_prefix("PPid:") {
                    parent.insert(pid, v.trim().parse().unwrap_or(0));
                } else if let Some(v) = l.strip_prefix("VmRSS:") {
                    rss.insert(pid, v.trim().trim_end_matches("kB").trim().parse().unwrap_or(0));
                }
            }
        }
        let in_tree = |mut p: u32| loop {
            if p == me {
                return true;
            }
            match parent.get(&p) {
                Some(&pp) if pp != 0 && pp != p => p = pp,
                _ => return false,
            }
        };
        Some(rss.iter().filter(|(p, _)| in_tree(**p)).map(|(_, k)| *k).sum())
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(target_os = "linux")]
    fn own_rss_is_counted() {
        assert!(super::tree_rss_kib().unwrap() > 0);
    }
}
