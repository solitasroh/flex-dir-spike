//! 표시 문자열. C#판 SizeFormatter·TimestampFormatter(ko-KR) 와 같은 규칙.

/// `1 MB` 미만은 KB 올림 + 천 단위 쉼표, 그 위는 소수 한 자리 (반올림 AwayFromZero).
pub fn size_text(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes < MB {
        return format!("{} KB", thousands(bytes.div_ceil(KB)));
    }
    let (unit, suffix) = if bytes < GB { (MB, "MB") } else { (GB, "GB") };
    // 소수 한 자리 반올림을 정수로 한다 (부동소수 반올림 오차를 피한다)
    let tenths = (bytes * 10 + unit / 2) / unit;
    format!("{}.{} {suffix}", thousands(tenths / 10), tenths % 10)
}

/// ko-KR ShortDate + ShortTime: `2026-11-23 오후 6:05`
pub fn timestamp_text(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> String {
    let (ampm, h12) = match hour {
        0 => ("오전", 12),
        1..=11 => ("오전", hour),
        12 => ("오후", 12),
        _ => ("오후", hour - 12),
    };
    format!("{year:04}-{month:02}-{day:02} {ampm} {h12}:{minute:02}")
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_match_csharp_formatter() {
        assert_eq!(size_text(0), "0 KB");
        assert_eq!(size_text(1), "1 KB");
        assert_eq!(size_text(18_432), "18 KB");
        assert_eq!(size_text(1_048_575), "1,024 KB");
        assert_eq!(size_text(1_232_896), "1.2 MB");
        assert_eq!(size_text(90_636_288), "86.4 MB");
        assert_eq!(size_text(1_610_612_736), "1.5 GB");
    }

    #[test]
    fn timestamps_use_korean_ampm() {
        assert_eq!(timestamp_text(2026, 11, 23, 18, 5), "2026-11-23 오후 6:05");
        assert_eq!(timestamp_text(2026, 1, 2, 0, 9), "2026-01-02 오전 12:09");
        assert_eq!(timestamp_text(2026, 1, 2, 12, 0), "2026-01-02 오후 12:00");
    }
}
