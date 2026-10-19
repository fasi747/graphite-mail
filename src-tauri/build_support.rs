//! Shared by build.rs and the tests: where the Google OAuth settings come from at build time.

/// Picks a setting: an environment variable wins, then `google_oauth.json` (an untracked file).
/// Values with line breaks or quotes are refused, so nothing can be smuggled into the build.
pub fn pick(env_value: Option<String>, file_text: Option<&str>, key: &str) -> Option<String> {
    let clean = |s: String| {
        let s = s.trim().to_string();
        (!s.is_empty() && !s.chars().any(|c| c.is_control() || c == '"' || c == '\\')).then_some(s)
    };
    if let Some(v) = env_value.and_then(clean) {
        return Some(v);
    }
    let text = file_text?;
    // minimal JSON string lookup: "key": "value"   (no dependency needed for a flat file)
    let marker = format!("\"{key}\"");
    let after = &text[text.find(&marker)? + marker.len()..];
    let after = after.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    clean(after[..after.find('"')?].to_string())
}
