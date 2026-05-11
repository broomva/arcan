//! Live-Anthropic end-to-end smoke test for the BRO-1010 blessed
//! authored agents (`general`, `goal-pursuer`, `goal-judge`).
//!
//! This is the **first runtime validation** of the authored-agents
//! architecture (spec
//! `core/life/docs/superpowers/specs/2026-05-09-bro-1006-authored-agents-architecture.md`).
//! Until this test exists, every other agents-related test in the
//! workspace is **offline** — they validate schemas, fixture parses,
//! and structural invariants, but never actually call a model and
//! see whether the agent's prompt produces a schema-conformant
//! answer.
//!
//! ## What this test exercises
//!
//! For each of the three blessed agents, end-to-end:
//!
//! 1. Load the agent's `.md` file from `<workspace>/agents/` through
//!    `ergon::FsAgentRegistry::load` (the production loader).
//! 2. Build the same provider adapter chain arcan uses at runtime:
//!    `arcan_provider::AnthropicProvider` → `ArcanProviderAdapter`
//!    (`aios_protocol::ModelProviderPort`) → `arcan_ergon::ModelProviderAdapter`
//!    (`ergon::Provider`).
//! 3. Build a `StepCtx` manually with that provider chain + empty
//!    tools + empty hooks + `BufferSink` + a minimal `RuntimeHandle`.
//! 4. Synthesize a small input that matches the agent's declared
//!    `input_schema`.
//! 5. Call `agent.run(&mut ctx, input)` — this drives the full
//!    autonomous loop: model emits `record_answer` tool_use, the
//!    interpreter captures + validates against `output_schema`,
//!    returns the typed answer.
//! 6. Assert the returned JSON validates against the agent's
//!    declared `output_schema` (using the production `jsonschema`
//!    validator) AND has the expected top-level keys.
//!
//! If this test passes, the architecture is no longer just
//! "validated by offline tests" — it's running real work against a
//! real model end-to-end.
//!
//! ## Why `#[ignore]`?
//!
//! - Real network call → flaky on offline runners.
//! - Costs money — Anthropic billing per call (~$0.05 total for
//!   all three agents at default Sonnet pricing).
//! - Requires `ANTHROPIC_API_KEY` at test time.
//!
//! Run manually:
//! ```bash
//! ANTHROPIC_API_KEY=sk-ant-... \
//!   cargo test -p arcan-ergon --test anthropic_agents_smoke \
//!     -- --ignored --nocapture
//! ```
//!
//! Override the model for ALL agents (cheaper validation) via
//! `ANTHROPIC_MODEL=claude-haiku-4-5`.

use std::path::PathBuf;
use std::sync::Arc;

use aios_protocol::{BranchId, ModelProviderPort, RunId, SessionId};
use arcan_aios_adapters::ArcanProviderAdapter;
use arcan_core::runtime::Provider as ArcanProvider;
use arcan_ergon::ModelProviderAdapter;
use arcan_provider::anthropic::{AnthropicConfig, AnthropicProvider};
use async_trait::async_trait;
use ergon::{
    Agent, AgentRegistry, BufferSink, ErgonError, FsAgentRegistry, HookRegistry, Provider,
    SessionId as ErgonSessionId, StepCtx, StreamSink, ToolCall, ToolDefinition, ToolRegistry,
    ToolResult,
};

/// Resolve `<workspace>/agents/` from the crate's `CARGO_MANIFEST_DIR`.
/// Path layout is fixed (this crate sits 3 levels deep in the
/// monorepo).
fn agents_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("agents")
        .canonicalize()
        .expect("workspace agents/ dir must exist (BRO-1010 ships it)")
}

/// Bring up the production provider adapter chain.
///
/// The shape mirrors `arcan/src/main.rs::run_serve` startup but
/// stripped to the bare minimum needed for `ergon::Agent::run`:
/// no event store, no policy gate, no approvals, no tool harness —
/// the agents under test don't use any of that.
///
/// Returns the chain pre-wrapped as an `ergon::Provider`, ready to
/// hand to `StepCtx::new`.
fn build_ergon_provider() -> Arc<dyn Provider> {
    let config = AnthropicConfig::from_env().expect(
        "ANTHROPIC_API_KEY must be set for the live-anthropic smoke test; \
         run with --ignored and provide the env var",
    );
    let arcan_provider: Arc<dyn ArcanProvider> = Arc::new(AnthropicProvider::new(config));

    // ArcanProviderAdapter wants a tools list (we pass empty — agents
    // synthesize their own `record_answer` tool via the chained
    // registry) and a streaming-sender handle (we pass an empty mutex
    // — we don't subscribe to its broadcast for this test).
    let streaming_sender = Arc::new(std::sync::Mutex::new(None));
    let port: Arc<dyn ModelProviderPort> = Arc::new(ArcanProviderAdapter::new(
        arcan_provider,
        Vec::new(),
        streaming_sender,
    ));

    Arc::new(ModelProviderAdapter::new(
        port,
        SessionId::from_string("smoke-1013".to_owned()),
        BranchId::main(),
        RunId::new_uuid(),
        "anthropic-smoke",
    ))
}

/// Empty tool registry — the agents we're testing don't use external
/// tools in this smoke test (the framework's own `record_answer`
/// tool is synthesized by `ergon::run_spec` regardless of what's
/// here).
#[derive(Default)]
struct EmptyTools;

#[async_trait]
impl ToolRegistry for EmptyTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        Vec::new()
    }
    async fn invoke(&self, call: ToolCall) -> Result<ToolResult, ErgonError> {
        Err(ErgonError::Tool(format!(
            "EmptyTools cannot invoke `{}`",
            call.name
        )))
    }
}

/// Minimal `RuntimeHandle` that just reports `Execute` mode. Agents
/// don't introspect operating mode in this smoke test.
struct ExecuteRuntime;

impl ergon::RuntimeHandle for ExecuteRuntime {
    fn operating_mode(&self) -> aios_protocol::mode::OperatingMode {
        aios_protocol::mode::OperatingMode::Execute
    }
}

fn build_step_ctx<'a>(workflow_name: &'a str, provider: Arc<dyn Provider>) -> StepCtx<'a> {
    StepCtx::new(
        ErgonSessionId::default(),
        workflow_name,
        provider,
        Arc::new(EmptyTools) as Arc<dyn ToolRegistry>,
        Arc::new(HookRegistry::default()),
        Arc::new(BufferSink::new()) as Arc<dyn StreamSink>,
        Arc::new(ExecuteRuntime) as Arc<dyn ergon::RuntimeHandle>,
        tracing::Span::current(),
    )
}

/// Validate `value` against `schema` using the same `jsonschema`
/// validator the production `record_answer` path uses (the schema
/// already validated there — this is a defense-in-depth check that
/// surfaces a clearer error if the agent's spec validation has been
/// bypassed somehow).
fn assert_schema(value: &serde_json::Value, schema: &serde_json::Value, agent_name: &str) {
    let compiled = jsonschema::JSONSchema::options()
        .compile(schema)
        .unwrap_or_else(|e| panic!("agent `{agent_name}` output_schema does not compile: {e}"));
    if let Err(errors) = compiled.validate(value) {
        let joined: Vec<String> = errors.map(|e| format!("{e}")).collect();
        panic!(
            "agent `{agent_name}` output failed schema validation:\n  errors: {}\n  value: {}",
            joined.join("; "),
            serde_json::to_string_pretty(value).unwrap_or_default()
        );
    }
}

fn init_tracing_once() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                    tracing_subscriber::EnvFilter::new("info,arcan_ergon=info,ergon=info")
                }),
            )
            .with_test_writer()
            .try_init();
    });
}

/// Run a single blessed agent against the live model. Returns the
/// agent's typed answer for downstream assertions / chaining.
///
/// `provider` MUST be constructed in a sync context (outside any
/// tokio runtime) and passed in — see the header comment about
/// `reqwest::blocking`. Constructing it inside an async block panics
/// when the inner reqwest runtime is dropped.
async fn run_one_agent(
    registry: &FsAgentRegistry,
    provider: Arc<dyn Provider>,
    name: &str,
    input: serde_json::Value,
) -> serde_json::Value {
    let agent: Arc<dyn Agent> = registry
        .get(name)
        .await
        .unwrap_or_else(|| panic!("agent `{name}` must be registered"));
    let spec = agent.spec();
    eprintln!(
        "[smoke] {name}: model={} max_turns={} input_keys={:?}",
        spec.model,
        spec.max_turns,
        input.as_object().map(|m| m.keys().collect::<Vec<_>>())
    );

    let mut ctx = build_step_ctx("smoke-1013", provider);

    let answer = agent
        .run(&mut ctx, input.clone())
        .await
        .unwrap_or_else(|e| panic!("agent `{name}` run failed: {e}"));

    assert_schema(&answer, &spec.output_schema, name);
    eprintln!(
        "[smoke] {name}: answer keys = {:?}",
        answer.as_object().map(|m| m.keys().collect::<Vec<_>>())
    );
    answer
}

// ── The actual smoke test ──────────────────────────────────────────────

/// Runs each of the three BRO-1010 blessed agents (`general`,
/// `goal-pursuer`, `goal-judge`) against the live Anthropic endpoint
/// and verifies each one returns a schema-conformant typed answer.
///
/// ## Composition test: `goal-judge` reads `goal-pursuer`'s output
///
/// Step 2 (`goal-pursuer`) produces an `outcome`-typed answer. Step 3
/// (`goal-judge`) takes that answer's fields verbatim as its
/// `claimed_outcome`, `evidence`, `unmet_criteria`, `pursuer_reasoning`
/// inputs. This is the production composition pattern — judge runs
/// after pursuer — and validates that the cross-agent enum invariant
/// (`goal-judge.claimed_outcome` mirrors `goal-pursuer.outcome`)
/// produces a schema-conformant judge run when fed real pursuer output.
///
/// ## Why `#[test]` not `#[tokio::test]`
///
/// `arcan_provider::AnthropicProvider` uses `reqwest::blocking`
/// internally, which spawns its own tokio runtime. Constructing the
/// provider inside an async context (which `#[tokio::test]` implies)
/// would later drop that inner runtime from an async context and
/// panic with "Cannot drop a runtime in a context where blocking is
/// not allowed". So we mirror `tests/anthropic_workflow.rs` and
/// `arcan/src/main.rs`: sync `#[test]` builds the runtime manually
/// and `block_on`s the smoke.
#[test]
#[ignore = "requires ANTHROPIC_API_KEY and live network; run with --ignored"]
fn three_blessed_agents_round_trip_against_live_anthropic() {
    init_tracing_once();

    // The provider adapter chain MUST be constructed in sync context.
    // `arcan_provider::AnthropicProvider` uses `reqwest::blocking`
    // internally, which spawns its own tokio runtime; dropping that
    // inner runtime from inside an outer async context panics with
    // "Cannot drop a runtime in a context where blocking is not
    // allowed". So we build ONE provider here (sync), then clone the
    // Arc into the async block — all three agents share the chain.
    let provider = build_ergon_provider();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");

    runtime.block_on(async move {
        let registry =
            FsAgentRegistry::load(agents_dir()).expect("agents/ directory must load cleanly");

        // ── 1. general ────────────────────────────────────────────
        //
        // Trivial single-shot ask. Confidence should be high (the
        // agent knows arithmetic). `used_tools` should be empty (no
        // tools advertised). `response` should be a non-empty string.
        let general_input = serde_json::json!({
            "request": "What is 2 + 2? Reply with just the number, no preamble.",
        });
        let general_answer =
            run_one_agent(&registry, Arc::clone(&provider), "general", general_input).await;
        let response = general_answer
            .get("response")
            .and_then(|v| v.as_str())
            .expect("general.response is a string");
        assert!(
            !response.is_empty(),
            "general.response must not be empty: {general_answer}"
        );
        let confidence = general_answer
            .get("confidence")
            .and_then(|v| v.as_f64())
            .expect("general.confidence is a number");
        assert!(
            (0.0..=1.0).contains(&confidence),
            "general.confidence in [0,1]: got {confidence}"
        );
        assert!(
            general_answer
                .get("used_tools")
                .and_then(|v| v.as_array())
                .map(|a| a.is_empty())
                .unwrap_or(false),
            "general.used_tools must be an empty array (no tools advertised): {general_answer}"
        );

        // ── 2. goal-pursuer ───────────────────────────────────────
        //
        // Single-criterion arithmetic goal with a hard constraint.
        // Outcome should be `success` (it's literally just arithmetic).
        // The pursuer's output feeds into goal-judge below.
        let pursuer_input = serde_json::json!({
            "goal": "Compute the sum of 17 and 25 and report it as a single integer.",
            "success_criteria": [
                "The agent reports a single integer that equals 17 + 25 = 42."
            ],
            "constraints": [
                "Reason from arithmetic; do not call any tools."
            ],
        });
        let pursuer_answer = run_one_agent(
            &registry,
            Arc::clone(&provider),
            "goal-pursuer",
            pursuer_input.clone(),
        )
        .await;
        let outcome = pursuer_answer
            .get("outcome")
            .and_then(|v| v.as_str())
            .expect("goal-pursuer.outcome is a string");
        assert!(
            matches!(outcome, "success" | "partial" | "failure"),
            "goal-pursuer.outcome must be a known enum value: got {outcome:?}"
        );
        let evidence = pursuer_answer
            .get("evidence")
            .and_then(|v| v.as_array())
            .expect("goal-pursuer.evidence is an array");
        assert!(
            !evidence.is_empty(),
            "goal-pursuer.evidence must be non-empty per spec (minItems: 1): {pursuer_answer}"
        );

        // ── 3. goal-judge (chained on pursuer's output) ───────────
        //
        // The judge takes the pursuer's fields verbatim. This pins
        // the cross-agent invariant (`claimed_outcome` enum matches
        // `outcome` enum) under live conditions.
        let judge_input = serde_json::json!({
            "goal": pursuer_input.get("goal").cloned().unwrap_or(serde_json::Value::Null),
            "success_criteria":
                pursuer_input.get("success_criteria").cloned().unwrap_or(serde_json::Value::Null),
            "claimed_outcome": pursuer_answer
                .get("outcome")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            "evidence":
                pursuer_answer.get("evidence").cloned().unwrap_or(serde_json::Value::Null),
            "unmet_criteria":
                pursuer_answer.get("unmet_criteria").cloned().unwrap_or(serde_json::json!([])),
            "pursuer_reasoning":
                pursuer_answer.get("reasoning").cloned().unwrap_or(serde_json::Value::Null),
        });
        let judge_answer =
            run_one_agent(&registry, Arc::clone(&provider), "goal-judge", judge_input).await;
        let score = judge_answer
            .get("score")
            .and_then(|v| v.as_i64())
            .expect("goal-judge.score is an integer");
        assert!(
            (0..=3).contains(&score),
            "goal-judge.score in 0..=3: got {score}"
        );
        let honest = judge_answer
            .get("honest")
            .and_then(|v| v.as_bool())
            .expect("goal-judge.honest is a bool");
        let criteria_assessment = judge_answer
            .get("criteria_assessment")
            .and_then(|v| v.as_array())
            .expect("goal-judge.criteria_assessment is an array");
        assert_eq!(
            criteria_assessment.len(),
            1,
            "goal-judge.criteria_assessment must have one entry per success_criterion (1 in this test): \
             got {} entries",
            criteria_assessment.len()
        );

        eprintln!(
            "[smoke] OK — general(conf={confidence:.2}) → pursuer(outcome={outcome}) → \
             judge(score={score} honest={honest})"
        );
    });
}
