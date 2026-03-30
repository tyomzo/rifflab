/// Strip markdown formatting (code fences, headers, bold) from tab text.
pub fn strip_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_code_block = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        let line = if trimmed.starts_with('#') {
            trimmed.trim_start_matches('#').trim()
        } else {
            trimmed
        };
        let line = line.replace("**", "").replace("__", "");
        out.push_str(&line);
        out.push('\n');
    }
    out
}
