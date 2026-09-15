use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Candidate filenames, first match wins.
pub const README_CANDIDATES: &[&str] = &["README.md", "README", "README.rst", "README.txt"];

/// Cap disk reads so a huge README cannot stall the TUI event loop.
pub const README_MAX_BYTES: usize = 16 * 1024;
/// Cap rendered lines after markdown stripping.
pub const README_MAX_LINES: usize = 200;

const EMPTY_STATE: &str = "No README in this repo";
const TRUNCATED_MARK: &str = "… (truncated)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadmePreview {
    /// Basename of the file that was read, if any.
    pub source: Option<String>,
    pub text: String,
    pub truncated: bool,
}

impl ReadmePreview {
    pub fn missing() -> Self {
        Self {
            source: None,
            text: EMPTY_STATE.into(),
            truncated: false,
        }
    }
}

pub fn discover_readme(repo_path: &Path) -> Option<PathBuf> {
    for name in README_CANDIDATES {
        let candidate = repo_path.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn load_readme_preview(repo_path: &Path) -> ReadmePreview {
    let Some(path) = discover_readme(repo_path) else {
        return ReadmePreview::missing();
    };
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "README".into());
    match read_capped(&path) {
        Ok((raw, byte_truncated)) => {
            let rendered = strip_simple_markdown(&raw);
            let mut lines: Vec<&str> = rendered.lines().collect();
            let line_truncated = lines.len() > README_MAX_LINES;
            lines.truncate(README_MAX_LINES);
            let mut text = lines.join("\n");
            let truncated = byte_truncated || line_truncated;
            if truncated {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(TRUNCATED_MARK);
            }
            ReadmePreview {
                source: Some(filename),
                text,
                truncated,
            }
        }
        Err(err) => ReadmePreview {
            source: Some(filename),
            text: format!("Could not read README: {err}"),
            truncated: false,
        },
    }
}

fn read_capped(path: &Path) -> std::io::Result<(String, bool)> {
    let file = File::open(path)?;
    let mut buf = Vec::new();
    file.take(README_MAX_BYTES as u64 + 1)
        .read_to_end(&mut buf)?;
    let truncated = buf.len() > README_MAX_BYTES;
    if truncated {
        buf.truncate(README_MAX_BYTES);
    }
    Ok((String::from_utf8_lossy(&buf).into_owned(), truncated))
}

/// Strip enough markdown to be readable in a terminal: ATX headings, fenced
/// code as plain text, and links as their labels. Leaves the rest alone.
pub fn strip_simple_markdown(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_fence = false;
    for line in input.lines() {
        let trimmed = line.trim_start();
        if is_fence_marker(trimmed) {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let block = strip_block_syntax(line);
        out.push_str(&strip_inline_markdown(&block));
        out.push('\n');
    }
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

fn is_fence_marker(trimmed: &str) -> bool {
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

fn strip_block_syntax(line: &str) -> String {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) {
        let after = &trimmed[hashes..];
        if after.is_empty() || after.starts_with(char::is_whitespace) {
            return after.trim().trim_end_matches('#').trim().to_string();
        }
    }
    let body = if let Some(rest) = trimmed.strip_prefix("> ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix('>') {
        rest.trim_start()
    } else {
        trimmed
    };
    let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() >= 3
        && (compact.chars().all(|c| c == '-')
            || compact.chars().all(|c| c == '*')
            || compact.chars().all(|c| c == '_'))
    {
        return "────────".into();
    }
    let indent_len = line.len() - trimmed.len();
    format!("{}{body}", &line[..indent_len])
}

fn strip_inline_markdown(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut out = String::with_capacity(input.len());
    while i < chars.len() {
        match chars[i] {
            '!' if chars.get(i + 1) == Some(&'[') => {
                if let Some((label, end)) = parse_md_link(&chars, i + 1) {
                    out.push_str(&label);
                    i = end;
                    continue;
                }
                out.push('!');
                i += 1;
            }
            '[' => {
                if let Some((label, end)) = parse_md_link(&chars, i) {
                    out.push_str(&label);
                    i = end;
                    continue;
                }
                out.push('[');
                i += 1;
            }
            '`' => {
                if let Some((code, end)) = parse_delimited(&chars, i, '`') {
                    out.push_str(&code);
                    i = end;
                    continue;
                }
                out.push('`');
                i += 1;
            }
            '*' if chars.get(i + 1) == Some(&'*') => {
                if let Some((inner, end)) = parse_double_star(&chars, i) {
                    out.push_str(&strip_inline_markdown(&inner));
                    i = end;
                    continue;
                }
                out.push('*');
                i += 1;
            }
            '*' => {
                if let Some((inner, end)) = parse_delimited(&chars, i, '*') {
                    out.push_str(&inner);
                    i = end;
                    continue;
                }
                out.push('*');
                i += 1;
            }
            _ => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

fn parse_md_link(chars: &[char], start: usize) -> Option<(String, usize)> {
    if chars.get(start) != Some(&'[') {
        return None;
    }
    let mut i = start + 1;
    let mut label = String::new();
    while i < chars.len() && chars[i] != ']' {
        if chars[i] == '\n' {
            return None;
        }
        label.push(chars[i]);
        i += 1;
    }
    if i >= chars.len() {
        return None;
    }
    i += 1;
    if chars.get(i) != Some(&'(') {
        return None;
    }
    i += 1;
    while i < chars.len() && chars[i] != ')' {
        if chars[i] == '\n' {
            return None;
        }
        i += 1;
    }
    if i >= chars.len() {
        return None;
    }
    Some((label, i + 1))
}

fn parse_delimited(chars: &[char], start: usize, delim: char) -> Option<(String, usize)> {
    if chars.get(start) != Some(&delim) {
        return None;
    }
    let mut i = start + 1;
    let mut inner = String::new();
    while i < chars.len() && chars[i] != delim {
        inner.push(chars[i]);
        i += 1;
    }
    if i >= chars.len() || inner.is_empty() {
        return None;
    }
    Some((inner, i + 1))
}

fn parse_double_star(chars: &[char], start: usize) -> Option<(String, usize)> {
    if chars.get(start) != Some(&'*') || chars.get(start + 1) != Some(&'*') {
        return None;
    }
    let mut i = start + 2;
    let mut inner = String::new();
    while i + 1 < chars.len() {
        if chars[i] == '*' && chars[i + 1] == '*' {
            if inner.is_empty() {
                return None;
            }
            return Some((inner, i + 2));
        }
        inner.push(chars[i]);
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn discovers_readme_md_before_plain_readme() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("README"), "plain").unwrap();
        fs::write(dir.path().join("README.md"), "# md\n").unwrap();
        let found = discover_readme(dir.path()).unwrap();
        assert_eq!(found.file_name().unwrap(), "README.md");
        let preview = load_readme_preview(dir.path());
        assert_eq!(preview.source.as_deref(), Some("README.md"));
        assert!(preview.text.contains("md"));
        assert!(!preview.truncated);
    }

    #[test]
    fn falls_back_readme_then_rst_then_txt() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("README.txt"), "txt").unwrap();
        assert_eq!(
            discover_readme(dir.path()).unwrap().file_name().unwrap(),
            "README.txt"
        );
        fs::write(dir.path().join("README.rst"), "rst").unwrap();
        assert_eq!(
            discover_readme(dir.path()).unwrap().file_name().unwrap(),
            "README.rst"
        );
        fs::write(dir.path().join("README"), "plain").unwrap();
        assert_eq!(
            discover_readme(dir.path()).unwrap().file_name().unwrap(),
            "README"
        );
    }

    #[test]
    fn missing_readme_shows_empty_state() {
        let dir = tempfile::tempdir().unwrap();
        let preview = load_readme_preview(dir.path());
        assert_eq!(preview, ReadmePreview::missing());
        assert_eq!(preview.text, "No README in this repo");
    }

    #[test]
    fn strips_headings_fences_and_links() {
        let raw = "\
# Hello

See [docs](https://example.com) and ![logo](logo.png).

```rust
fn x() {}
```

**bold** and `code`
";
        let out = strip_simple_markdown(raw);
        assert!(out.contains("Hello"));
        assert!(!out.contains("# Hello"));
        assert!(out.contains("See docs and logo."));
        assert!(!out.contains("https://example.com"));
        assert!(!out.contains("```"));
        assert!(out.contains("fn x() {}"));
        assert!(out.contains("bold"));
        assert!(out.contains("code"));
        assert!(!out.contains("**"));
    }

    #[test]
    fn caps_preview_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let huge = "x".repeat(README_MAX_BYTES + 4000);
        fs::write(dir.path().join("README.md"), &huge).unwrap();
        let preview = load_readme_preview(dir.path());
        assert!(preview.truncated);
        assert!(preview.text.len() < huge.len());
        assert!(preview.text.contains(TRUNCATED_MARK));
    }

    #[test]
    fn caps_preview_lines() {
        let dir = tempfile::tempdir().unwrap();
        let many = (0..README_MAX_LINES + 40)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(dir.path().join("README.md"), many).unwrap();
        let preview = load_readme_preview(dir.path());
        assert!(preview.truncated);
        let kept = preview
            .text
            .lines()
            .filter(|l| l.starts_with("line "))
            .count();
        assert_eq!(kept, README_MAX_LINES);
    }
}
