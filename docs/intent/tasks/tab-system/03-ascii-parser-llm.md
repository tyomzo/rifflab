# Task 03: LLM-Based ASCII Tab Parser

## Goal
Implement the LLM-based parser from spec section 3.2.1 using the Anthropic API.

## Deliverables
1. `crates/rifflab-tab/src/ascii_llm.rs` — LLM parser module
2. Functions:
   - `parse_ascii_tab_llm(text: &str, api_key: &str) -> Result<Vec<TabNote>>` — async
   - Build system prompt per spec section 3.2.1
   - Send raw tab text as user message
   - Parse JSON response (strip markdown fences if present)
   - Convert `AsciiNote` JSON objects to `TabNote` structs
   - On parse failure: retry once with simplified prompt
   - On second failure: fall back to rule-based parser (task 02)
3. `AsciiNote` struct for intermediate JSON deserialization
4. Configuration: model name, max input bytes, default confidence
5. Add `reqwest` (with `rustls-tls`) to dependencies for HTTP calls
6. Tests:
   - Mock test with pre-recorded LLM response JSON
   - Fallback test: bad JSON → falls back to rule-based

## Dependencies
- Task 01 (data model)
- Task 02 (fallback parser)

## Files
- `crates/rifflab-tab/src/ascii_llm.rs`
- Update `Cargo.toml` for reqwest, tokio dependencies

## Notes
- API key loaded from env var `ANTHROPIC_API_KEY` or config file
- Use `claude-sonnet-4-20250514` model by default
- Max input: 50KB

## Verification
- `cargo build --workspace` compiles
- Mock tests pass
- Manual test: paste real tab, verify parsed output
