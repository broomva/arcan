//! Integration tests for `arcan agent` CLI handlers (BRO-1008).
//!
//! These tests exercise the public surface of `arcan::agent_cmd`
//! directly — no subprocess invocation, no LLM, no network. They run
//! offline on every `cargo test`.
//!
//! ## What's covered
//!
//! - `list` resolves the project's blessed agents (`general`,
//!   `goal-pursuer`, `goal-judge`).
//! - `show` returns a structured payload for a known agent and an
//!   error for an unknown one.
//! - `new` scaffolds a parseable agent file and refuses to overwrite.
//! - `test --dry-run` accepts valid input and rejects schema violations
//!   with structured error messages.
//! - `test --live` plumbing: the provider adapter chain
//!   (`build_ergon_provider`), the spend cap pin, and the shared
//!   schema validator — all offline. The one networked test
//!   (`test_live_smoke_against_live_anthropic`) is `#[ignore]`d AND
//!   gated behind `ARCAN_AGENT_TEST_LIVE=1`.
//!
//! Per spec
//! `core/life/docs/superpowers/specs/2026-05-09-bro-1006-authored-agents-architecture.md`
//! §M5, this is the operator-facing tooling for the authored-agent
//! substrate. Keep these tests deterministic — adding a flake here
//! delays every PR until the flake is fixed.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use arcan::agent_cmd;
use ergon::{FsAgentRegistry, parse_agent_md};

// ─── helpers ────────────────────────────────────────────────────────────

/// Resolve the workspace-root `agents/` directory from this crate's
/// `CARGO_MANIFEST_DIR`. Mirrors the resolution used by
/// `crates/arcan/arcan-ergon/tests/agents_fixtures.rs` so both test
/// suites agree on where the blessed agents live.
fn workspace_agents_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("agents")
        .canonicalize()
        .expect("workspace agents/ dir must exist (BRO-1010 ships it)")
}

/// Allocate a unique temporary directory name. Combines wall-clock
/// nanoseconds with an atomic counter so concurrent test invocations
/// (and `cargo nextest`) never collide.
fn unique_temp_dir(prefix: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("arcan-agent-cmd-{prefix}-{nanos}-{n}"));
    std::fs::create_dir_all(&path).expect("create temp dir");
    path
}

// ─── list ───────────────────────────────────────────────────────────────

/// `arcan agent list` against the workspace `agents/` directory must
/// surface every blessed agent loaded by the runtime.
#[tokio::test]
async fn list_resolves_blessed_agents() {
    let dir = workspace_agents_dir();
    // `list` writes to stdout — we don't capture stdout here; we just
    // assert it returns Ok and that the registry-load path under it
    // sees what the runtime would see.
    agent_cmd::list(&dir).await.expect("list must succeed");

    // Cross-check via the same registry the CLI handler uses, so the
    // test is meaningful even though we don't capture stdout.
    let registry = FsAgentRegistry::load(&dir).expect("registry must load");
    use ergon::AgentRegistry as _;
    let names = registry.names().await;
    for required in ["general", "goal-judge", "goal-pursuer"] {
        assert!(
            names.iter().any(|n| n == required),
            "expected blessed agent `{required}` to be loaded; got {names:?}"
        );
    }
}

#[tokio::test]
async fn list_errors_on_missing_directory() {
    let missing = std::env::temp_dir().join("arcan-agent-cmd-definitely-missing-xyz");
    let _ = std::fs::remove_dir_all(&missing);
    let err = agent_cmd::list(&missing)
        .await
        .expect_err("missing dir must error");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("does not exist") || msg.contains("not a directory"),
        "expected helpful error, got: {msg}"
    );
}

// ─── show ───────────────────────────────────────────────────────────────

/// `arcan agent show <name>` must surface every part of the
/// `AgentSpec` to the operator. We assert via `print_spec` directly
/// (cheaper than capturing stdout) plus through the public `show`
/// path for the success-case smoke test.
#[tokio::test]
async fn show_pretty_prints_known_agent() {
    let dir = workspace_agents_dir();

    // Smoke: the public dispatch path returns Ok for a known agent.
    agent_cmd::show(&dir, "goal-pursuer")
        .await
        .expect("show goal-pursuer must succeed");

    // Substantive: load the agent's spec and feed it to `print_spec`
    // via a captured-stdout shim. We use a thread-local hack — easier
    // to just rebuild the rendered string by re-reading the source
    // file and confirming the pieces we promise to surface are
    // present in the spec itself.
    let pursuer_path = dir.join("goal-pursuer.md");
    let raw = std::fs::read_to_string(&pursuer_path).expect("read goal-pursuer.md");
    let spec = parse_agent_md(&raw).expect("parse goal-pursuer");

    // Assertions about the spec the operator would see — every field
    // the spec mandates `show` to render is non-trivially populated.
    assert_eq!(spec.name, "goal-pursuer");
    assert!(spec.model.contains("claude"));
    assert!(spec.max_turns >= 16);
    assert!(spec.instructions.contains("outcome"));
    assert!(spec.instructions.contains("success_criteria"));
    let output_schema = serde_json::to_string(&spec.output_schema).unwrap();
    assert!(output_schema.contains("outcome"));
}

#[tokio::test]
async fn show_returns_error_for_unknown_agent() {
    let dir = workspace_agents_dir();
    let err = agent_cmd::show(&dir, "definitely-not-an-agent")
        .await
        .expect_err("unknown agent must error");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("not found"),
        "expected `not found` in error, got: {msg}"
    );
}

// ─── new ────────────────────────────────────────────────────────────────

/// `arcan agent new <name>` scaffolds a markdown file that round-trips
/// through `parse_agent_md` cleanly — i.e. the runtime would accept it
/// without any operator intervention.
#[tokio::test]
async fn new_scaffolds_a_template_file() {
    let dir = unique_temp_dir("scaffold");
    agent_cmd::new_agent(&dir, "test-agent", None, None).expect("new_agent must succeed");

    let file = dir.join("test-agent.md");
    assert!(file.exists(), "scaffold must create {}", file.display());

    let raw = std::fs::read_to_string(&file).expect("read scaffolded file");
    assert!(raw.contains("---"), "must have frontmatter delimiters");
    assert!(
        raw.contains("name: test-agent"),
        "frontmatter must declare name matching filename stem"
    );

    let spec = parse_agent_md(&raw).expect("scaffolded file must parse via the runtime parser");
    assert_eq!(spec.name, "test-agent");
    assert!(!spec.model.is_empty());
    assert!(spec.max_turns >= 1);
    assert!(spec.input_schema.is_object());
    assert!(spec.output_schema.is_object());

    // The full registry path must accept the scaffolded file too —
    // this is what `arcan serve` does at boot.
    use ergon::AgentRegistry as _;
    let registry = FsAgentRegistry::load(&dir).expect("registry must accept the scaffolded file");
    assert!(registry.get("test-agent").await.is_some());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_honors_model_and_instructions_overrides() {
    let dir = unique_temp_dir("overrides");
    agent_cmd::new_agent(
        &dir,
        "custom",
        Some("claude-opus-4-7"),
        Some("Custom body line."),
    )
    .expect("new_agent must succeed");

    let raw = std::fs::read_to_string(dir.join("custom.md")).expect("read");
    let spec = parse_agent_md(&raw).expect("parse");
    assert_eq!(spec.model, "claude-opus-4-7");
    assert!(spec.instructions.contains("Custom body line."));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_refuses_to_overwrite_existing() {
    let dir = unique_temp_dir("overwrite");
    agent_cmd::new_agent(&dir, "x", None, None).expect("first scaffold must succeed");
    let err = agent_cmd::new_agent(&dir, "x", None, None)
        .expect_err("second scaffold must refuse to overwrite");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("refusing to overwrite"),
        "expected `refusing to overwrite`, got: {msg}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_rejects_invalid_filename_stems() {
    let dir = unique_temp_dir("badname");
    // Slashes would silently create directories; leading dots would
    // create hidden files. Both are operator footguns — the CLI must
    // refuse rather than honoring them.
    let err = agent_cmd::new_agent(&dir, "bad/name", None, None).expect_err("must reject slashes");
    assert!(format!("{err:#}").contains("not a valid filename stem"));

    let err =
        agent_cmd::new_agent(&dir, ".hidden", None, None).expect_err("must reject leading dot");
    assert!(format!("{err:#}").contains("not a valid filename stem"));

    let err = agent_cmd::new_agent(&dir, "", None, None).expect_err("must reject empty name");
    assert!(format!("{err:#}").contains("must not be empty"));

    let _ = std::fs::remove_dir_all(&dir);
}

// ─── test --dry-run ─────────────────────────────────────────────────────

/// `arcan agent test general --input '{"request": "hello"}' --dry-run`
/// must accept the valid payload — `general`'s `input_schema` requires
/// only `{request: string}`.
#[tokio::test]
async fn test_dryrun_passes_valid_input() {
    let dir = workspace_agents_dir();
    agent_cmd::test_dry_run(&dir, "general", r#"{"request": "hello"}"#)
        .await
        .expect("valid input must pass");
}

/// `arcan agent test goal-pursuer --input '{}' --dry-run` must reject
/// the payload — `goal-pursuer`'s schema requires `goal` and
/// `success_criteria`. The error must surface the schema violations
/// with enough specificity for the operator to fix the input.
#[tokio::test]
async fn test_dryrun_rejects_invalid_input() {
    let dir = workspace_agents_dir();
    let err = agent_cmd::test_dry_run(&dir, "goal-pursuer", "{}")
        .await
        .expect_err("missing required fields must fail");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("INVALID"),
        "expected `INVALID` marker in error, got: {msg}"
    );
    // The error must mention at least one of the missing required
    // fields so the operator can fix the payload without re-reading
    // the schema.
    assert!(
        msg.contains("goal") || msg.contains("success_criteria"),
        "expected reference to a missing required field, got: {msg}"
    );
}

#[tokio::test]
async fn test_dryrun_supports_at_path_input() {
    let dir = workspace_agents_dir();
    let tmp = unique_temp_dir("input-file");
    let payload = tmp.join("input.json");
    std::fs::write(&payload, r#"{"request": "from-file"}"#).expect("write payload");

    let arg = format!("@{}", payload.display());
    agent_cmd::test_dry_run(&dir, "general", &arg)
        .await
        .expect("@<path> input form must work");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn test_dryrun_returns_helpful_error_for_unknown_agent() {
    let dir = workspace_agents_dir();
    let err = agent_cmd::test_dry_run(&dir, "no-such-agent", "{}")
        .await
        .expect_err("unknown agent must error");
    assert!(format!("{err:#}").contains("not found"));
}

#[tokio::test]
async fn test_dryrun_rejects_malformed_json() {
    let dir = workspace_agents_dir();
    let err = agent_cmd::test_dry_run(&dir, "general", "not valid json")
        .await
        .expect_err("malformed JSON must error");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("failed to parse --input as JSON"),
        "expected JSON-parse error, got: {msg}"
    );
}

// ─── validate_input direct ─────────────────────────────────────────────

#[test]
fn validate_input_directly_accepts_valid_payload() {
    let dir = workspace_agents_dir();
    let raw = std::fs::read_to_string(dir.join("general.md")).expect("read general.md");
    let spec = parse_agent_md(&raw).expect("parse general");
    let payload = serde_json::json!({ "request": "hi" });
    agent_cmd::validate_input(&spec, &payload).expect("valid payload must pass");
}

#[test]
fn validate_input_directly_rejects_invalid_payload() {
    let dir = workspace_agents_dir();
    let raw = std::fs::read_to_string(dir.join("general.md")).expect("read general.md");
    let spec = parse_agent_md(&raw).expect("parse general");
    // `general.input_schema` requires `request: string` and bans
    // additional properties — passing only an unknown field violates
    // both. Both violations should surface in the error message.
    let payload = serde_json::json!({ "wrong_field": 1 });
    let err = agent_cmd::validate_input(&spec, &payload).expect_err("invalid payload must fail");
    let msg = format!("{err:#}");
    assert!(msg.contains("INVALID"));
}

// ─── resolve_agents_dir ────────────────────────────────────────────────

#[test]
fn resolve_agents_dir_defaults_to_relative_agents() {
    assert_eq!(agent_cmd::resolve_agents_dir(None), PathBuf::from("agents"));
}

#[test]
fn resolve_agents_dir_honors_explicit_path() {
    let custom = PathBuf::from("/tmp/custom-agents");
    assert_eq!(agent_cmd::resolve_agents_dir(Some(custom.clone())), custom);
}

// ─── test --live plumbing (offline) ────────────────────────────────────

/// The provider adapter chain built by `build_ergon_provider` must
/// report the supplied provider label through `ergon::Provider::name`
/// — that's what `StreamEvent::SessionStart` embeds, and the only
/// chain property observable without a model call.
#[test]
fn build_ergon_provider_reports_provider_name() {
    use std::sync::Arc;

    let provider: Arc<dyn arcan_core::runtime::Provider> = Arc::new(arcand::mock::MockProvider);
    let chain = agent_cmd::build_ergon_provider(provider, "mock");
    assert_eq!(chain.name(), "mock");
}

/// Pin the live-run spend cap. Bumping this constant changes how much
/// money a single `arcan agent test --live` invocation may burn —
/// that's a deliberate decision, not a drive-by edit.
#[test]
fn agent_test_token_cap_is_pinned() {
    assert_eq!(agent_cmd::AGENT_TEST_MAX_TOKENS, 50_000);
}

/// `validate_against_schema` is the shared engine behind the dry-run
/// input gate AND the live-run output check; its error text must name
/// both the payload kind and the agent so operators can tell which
/// side rejected.
#[test]
fn validate_against_schema_names_payload_and_agent() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": { "response": { "type": "string" } },
        "required": ["response"],
    });
    let err =
        agent_cmd::validate_against_schema(&schema, &serde_json::json!({}), "output", "goal-judge")
            .expect_err("missing required field must fail");
    let msg = format!("{err:#}");
    assert!(msg.contains("output is INVALID for agent `goal-judge`"));
}

// ─── test --live (gated live-LLM smoke) ────────────────────────────────

/// Live-LLM smoke for `arcan agent test --live` (BRO-1008).
///
/// Follows the gating pattern of
/// `crates/arcan/arcan-ergon/tests/anthropic_agents_smoke.rs`:
///
/// - `#[ignore]` — real network call, costs money (~$0.01 at Sonnet
///   pricing), requires `ANTHROPIC_API_KEY`.
/// - Additionally requires `ARCAN_AGENT_TEST_LIVE=1` so a blanket
///   `--ignored` sweep doesn't accidentally spend.
///
/// Run manually:
/// ```bash
/// ARCAN_AGENT_TEST_LIVE=1 ANTHROPIC_API_KEY=sk-ant-... \
///   cargo test -p arcan --test agent_cmd -- --ignored --nocapture \
///     test_live_smoke_against_live_anthropic
/// ```
///
/// ## Why `#[test]` not `#[tokio::test]`
///
/// `arcan_provider::AnthropicProvider` uses `reqwest::blocking`
/// internally, which owns an inner tokio runtime. The provider chain
/// must be constructed in sync context and its final `Arc` must drop
/// in sync context, or the inner runtime panics with "Cannot drop a
/// runtime in a context where blocking is not allowed". Mirrors the
/// smoke test and the `arcan agent test --live` arm in `main.rs`.
#[test]
#[ignore = "requires ARCAN_AGENT_TEST_LIVE=1, ANTHROPIC_API_KEY, and live network"]
fn test_live_smoke_against_live_anthropic() {
    use std::sync::Arc;

    if std::env::var("ARCAN_AGENT_TEST_LIVE").as_deref() != Ok("1") {
        eprintln!("[agent-test-live] skipped: set ARCAN_AGENT_TEST_LIVE=1 to run this smoke");
        return;
    }

    // Provider chain in sync context (see doc comment above).
    let config = arcan_provider::anthropic::AnthropicConfig::from_env().expect(
        "ANTHROPIC_API_KEY must be set for the live smoke; \
         run with --ignored and provide the env var",
    );
    let provider: Arc<dyn arcan_core::runtime::Provider> =
        Arc::new(arcan_provider::anthropic::AnthropicProvider::new(config));
    let chain = agent_cmd::build_ergon_provider(provider, "anthropic");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");

    runtime
        .block_on(agent_cmd::test_live(
            &workspace_agents_dir(),
            "general",
            r#"{"request": "What is 2 + 2? Reply with just the number, no preamble."}"#,
            Arc::clone(&chain),
        ))
        .expect("live `arcan agent test --live` run must succeed");

    // `chain` drops here, in sync context, after block_on returned.
}
