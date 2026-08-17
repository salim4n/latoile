//! Output scraping: ANSI stripping, OAuth URL and device-code
//! matching, and the output tail shown when nothing matches. Pure.

/// Remove ANSI escape sequences: ESC, then everything up to a final letter.
pub(super) fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The first https URL on one of the provider's hosts — trailing shell
/// punctuation trimmed.
pub(super) fn find_oauth_url(text: &str, hosts: &[&str]) -> Option<String> {
    text.split_whitespace()
        .filter(|tok| tok.starts_with("https://"))
        .map(|url| {
            url.trim_end_matches([')', '>', '.', ',', ']', '"', '\''])
                .to_string()
        })
        .find(|url| {
            let authority = url
                .strip_prefix("https://")
                .and_then(|rest| rest.split(['/', '?', '#']).next());
            let Some(authority) = authority.filter(|value| !value.contains('@')) else {
                return false;
            };
            let host = authority
                .rsplit_once(':')
                .filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit()))
                .map_or(authority, |(host, _)| host);
            hosts
                .iter()
                .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
        })
}

/// A device code: two groups of 3–8 uppercase letters/digits around a dash
/// (`ABCD-EFGH`). Tolerant on purpose — the exact format is the CLI's.
pub(super) fn find_device_code(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|tok| tok.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-'))
        .find(|tok| {
            let mut parts = tok.split('-');
            let (Some(a), Some(b), None) = (parts.next(), parts.next(), parts.next()) else {
                return false;
            };
            [a, b].iter().all(|part| {
                (3..=8).contains(&part.len())
                    && part
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            })
        })
        .map(str::to_string)
}

/// The last non-empty lines — what the UI shows when scraping finds nothing.
pub(super) fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines
        .iter()
        .rev()
        .take(n)
        .rev()
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_and_punctuation_are_stripped() {
        let plain = strip_ansi("\u{1b}[1mOpen\u{1b}[0m this");
        assert_eq!(plain, "Open this");
        assert_eq!(
            find_oauth_url("Visit https://claude.com/oauth?x=1). now", &["claude.com"]),
            Some("https://claude.com/oauth?x=1".into())
        );
        assert_eq!(
            find_oauth_url("https://example.com/nope", &["claude.com"]),
            None
        );
        assert_eq!(
            find_oauth_url("https://claude.com.evil.example/login", &["claude.com"]),
            None
        );
        assert_eq!(
            find_oauth_url("https://evil.example/?next=claude.com", &["claude.com"]),
            None
        );
        assert_eq!(
            find_oauth_url(
                "see https://auth.openai.com/codex/device please",
                &["openai.com", "chatgpt.com"]
            ),
            Some("https://auth.openai.com/codex/device".into())
        );
    }

    #[test]
    fn device_code_patterns() {
        assert_eq!(
            find_device_code("enter code ABCD-EFGH to continue"),
            Some("ABCD-EFGH".into())
        );
        assert_eq!(find_device_code("code: X99-Z21Q."), Some("X99-Z21Q".into()));
        assert_eq!(find_device_code("no code here"), None);
        assert_eq!(find_device_code("lowercase-code nope"), None);
    }

    #[test]
    fn garbage_output_leaves_a_hint_not_a_match() {
        let text = "compiling things\nsomething odd happened\ntry again later";
        assert_eq!(find_oauth_url(text, &["claude.com"]), None);
        assert_eq!(find_device_code(text), None);
        assert_eq!(
            last_lines(text, 2),
            "something odd happened\ntry again later"
        );
    }
}
