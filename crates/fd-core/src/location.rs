//! 경로 문법 두 벌. 둘 다 순수 함수이고 `cfg` 로 가르지 않는다 —
//! Windows 문법 테스트가 Linux CI 에서 돈다 (tech.md §4.1).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathFlavor {
    Windows,
    Posix,
}

impl PathFlavor {
    /// 지금 도는 OS 의 문법.
    pub const fn native() -> Self {
        if cfg!(windows) { PathFlavor::Windows } else { PathFlavor::Posix }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocationError {
    Empty,
    RelativePath,
    AlternateDataStream,
    InvalidCharacter,
    NetworkPathIncomplete,
}

/// 사용자가 친 주소를 내부 표현으로 바꾼다.
/// - Windows: `C:\a` → `\\?\C:\a`, `\\srv\share` → `\\?\UNC\srv\share` (C# LocationId 와 같은 문자열)
/// - POSIX: 절대 경로만 받는다. `~` 는 호출자가 미리 푼다. 끝의 `/` 는 루트가 아니면 뗀다.
pub fn parse(input: &str, flavor: PathFlavor) -> Result<String, LocationError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(LocationError::Empty);
    }
    match flavor {
        PathFlavor::Posix => parse_posix(s),
        PathFlavor::Windows => parse_windows(s),
    }
}

fn parse_posix(s: &str) -> Result<String, LocationError> {
    if !s.starts_with('/') {
        return Err(LocationError::RelativePath);
    }
    if s.contains('\0') {
        return Err(LocationError::InvalidCharacter);
    }
    let mut out = String::with_capacity(s.len());
    let mut prev_slash = false;
    for c in s.chars() {
        if c == '/' {
            if prev_slash {
                continue;
            }
            prev_slash = true;
        } else {
            prev_slash = false;
        }
        out.push(c);
    }
    if out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    Ok(out)
}

fn parse_windows(s: &str) -> Result<String, LocationError> {
    const EXT: &str = r"\\?\";
    const UNC_EXT: &str = r"\\?\UNC\";
    // UNC 판정은 구분자 정규화보다 먼저 한다 (SHELL_NOTES §경로 정규화 함정 1).
    let starts_unc = (s.starts_with(r"\\") || s.starts_with("//")) && !s.starts_with(EXT);
    let body: String = s.replace('/', "\\");
    if starts_unc {
        let rest = body.trim_start_matches('\\');
        let server = rest.split('\\').next().unwrap_or("");
        if server.is_empty() {
            return Err(LocationError::NetworkPathIncomplete);
        }
        let trimmed = rest.trim_end_matches('\\');
        if trimmed.chars().any(|c| matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|')) {
            return Err(LocationError::InvalidCharacter);
        }
        return Ok(format!("{UNC_EXT}{trimmed}"));
    }
    let local = body.strip_prefix(EXT).unwrap_or(&body);
    let bytes = local.as_bytes();
    let has_drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if !has_drive || (bytes.len() > 2 && bytes[2] != b'\\') {
        return Err(LocationError::RelativePath);
    }
    let tail = &local[2..];
    if tail.contains(':') {
        return Err(LocationError::AlternateDataStream);
    }
    if tail.chars().any(|c| matches!(c, '*' | '?' | '"' | '<' | '>' | '|')) {
        return Err(LocationError::InvalidCharacter);
    }
    let drive = local[..2].to_ascii_uppercase();
    let mut t = tail.to_string();
    while t.len() > 1 && t.ends_with('\\') {
        t.pop();
    }
    if t.is_empty() {
        t.push('\\');
    }
    Ok(format!("{EXT}{drive}{t}"))
}

/// 표시용 주소. 내부 확장 접두사를 뗀다.
pub fn display(internal: &str, flavor: PathFlavor) -> String {
    match flavor {
        PathFlavor::Posix => internal.to_string(),
        PathFlavor::Windows => {
            if let Some(rest) = internal.strip_prefix(r"\\?\UNC\") {
                format!(r"\\{rest}")
            } else {
                internal.strip_prefix(r"\\?\").unwrap_or(internal).to_string()
            }
        }
    }
}

/// 상위 폴더. 루트면 None.
pub fn parent(internal: &str, flavor: PathFlavor) -> Option<String> {
    match flavor {
        PathFlavor::Posix => {
            if internal == "/" {
                return None;
            }
            let idx = internal.rfind('/')?;
            Some(if idx == 0 { "/".to_string() } else { internal[..idx].to_string() })
        }
        PathFlavor::Windows => {
            let shown = display(internal, flavor);
            let t = shown.trim_end_matches('\\');
            if t.len() <= 2 {
                return None; // "C:"
            }
            let idx = t.rfind('\\')?;
            let p = &t[..idx];
            if p.starts_with(r"\\") && p.trim_start_matches('\\').is_empty() {
                return None;
            }
            parse(if p.len() == 2 { &t[..3] } else { p }, flavor).ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_absolute_path_is_accepted() {
        // 브라이언 결함 재현 1: C# Core 는 /home/... 을 '상대 경로' 로 거부했다.
        assert_eq!(parse("/home/soojang/문서", PathFlavor::Posix), Ok("/home/soojang/문서".into()));
        assert_eq!(parse("//home///a/", PathFlavor::Posix), Ok("/home/a".into()));
        assert_eq!(parse("/", PathFlavor::Posix), Ok("/".into()));
        assert_eq!(parse("home/a", PathFlavor::Posix), Err(LocationError::RelativePath));
    }

    #[test]
    fn windows_grammar_matches_csharp_internal_form() {
        assert_eq!(parse(r"c:\Users\a\", PathFlavor::Windows), Ok(r"\\?\C:\Users\a".into()));
        assert_eq!(parse(r"C:\", PathFlavor::Windows), Ok(r"\\?\C:\".into()));
        assert_eq!(parse("//srv/share/x", PathFlavor::Windows), Ok(r"\\?\UNC\srv\share\x".into()));
        assert_eq!(parse(r"\\srv", PathFlavor::Windows), Ok(r"\\?\UNC\srv".into()));
        assert_eq!(parse(r"\\", PathFlavor::Windows), Err(LocationError::NetworkPathIncomplete));
        assert_eq!(parse(r"C:\a:b", PathFlavor::Windows), Err(LocationError::AlternateDataStream));
        assert_eq!(parse(r"\\?\C:\a", PathFlavor::Windows), Ok(r"\\?\C:\a".into()));
        assert_eq!(parse("a\\b", PathFlavor::Windows), Err(LocationError::RelativePath));
    }

    #[test]
    fn parent_walks_up_to_root() {
        assert_eq!(parent("/home/a", PathFlavor::Posix), Some("/home".into()));
        assert_eq!(parent("/home", PathFlavor::Posix), Some("/".into()));
        assert_eq!(parent("/", PathFlavor::Posix), None);
        assert_eq!(parent(r"\\?\C:\a\b", PathFlavor::Windows), Some(r"\\?\C:\a".into()));
        assert_eq!(parent(r"\\?\C:\a", PathFlavor::Windows), Some(r"\\?\C:\".into()));
        assert_eq!(parent(r"\\?\C:\", PathFlavor::Windows), None);
    }
}
