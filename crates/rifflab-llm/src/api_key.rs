/// Load the Anthropic API key from environment variables.
///
/// Checks `ANTH_API_KEY` first (the RiffLab convention; sourced from
/// `~/.rystad_api_keys`), then falls back to the upstream `ANTHROPIC_API_KEY`.
/// Returns `None` if neither is set or both are empty.
pub fn get_api_key() -> Option<String> {
    let pick = |name: &str| {
        std::env::var(name).ok().filter(|s| !s.is_empty())
    };
    pick("ANTH_API_KEY").or_else(|| pick("ANTHROPIC_API_KEY"))
}
