//! Fixture test for the blessed authored agents shipped at
//! `<workspace_root>/agents/` (BRO-1010).
//!
//! These tests load the project's `agents/` directory through
//! [`ergon::FsAgentRegistry`] and validate that each blessed agent
//! parses, that its filename matches the declared `name`, that the
//! schemas compile under the production `jsonschema` validator, and
//! that the standard set of agents arcan expects to find (`general`,
//! `goal-pursuer`, `goal-judge`) are all present.
//!
//! ## Why these tests exist
//!
//! Per the architecture spec
//! `core/life/docs/superpowers/specs/2026-05-09-bro-1006-authored-agents-architecture.md`
//! §M7 (prompt-fragility hardening): every authored agent in the
//! blessed tier ships with a fixture test. The test runs at every
//! `cargo test` invocation, so:
//!
//! - A typo in frontmatter (missing required field, malformed YAML)
//!   fails CI before it reaches a workflow tick.
//! - A schema that won't compile under `jsonschema` fails at test
//!   time, not at first inference call (where it would silently
//!   bounce every `record_answer`).
//! - Renaming an agent's file without updating its `name` (or vice
//!   versa) fails at load time via `RegistryError::NameMismatch`.
//!
//! These tests are intentionally **deterministic and offline** — no
//! LLM calls, no API keys required, no flaky network. They verify
//! the *contract* of each agent (its declared shape), not its
//! runtime behavior. End-to-end behavior tests live in
//! `tests/anthropic_workflow.rs` and require an Anthropic key.

use std::path::PathBuf;
use std::sync::Arc;

use ergon::{Agent, AgentRegistry, FsAgentRegistry};

/// Resolve the workspace-root `agents/` directory from this crate's
/// `CARGO_MANIFEST_DIR`. The path layout is fixed:
/// `<workspace_root>/crates/arcan/arcan-ergon/Cargo.toml`, so the
/// agents directory is three levels up.
fn agents_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("agents")
        .canonicalize()
        .expect("workspace agents/ dir must exist (BRO-1010 ships it)")
}

/// Names every release of arcan must ship. Adding a new blessed
/// agent? Append its name here AND add a row to `agents/README.md`.
const REQUIRED_BLESSED_AGENTS: &[&str] = &[
    // BRO-1010 — first authored agents
    "general",
    "goal-judge",
    "goal-pursuer",
    // BRO-1011 — meta-agents (no production self-modification)
    "nous-judge",
    "nous-promoter",
    // BRO-1012 — bookkeeping (Nous gate scorers + synthesizer)
    "bookkeeping-novelty",
    "bookkeeping-relevance",
    "bookkeeping-specificity",
    "bookkeeping-synthesizer",
];

#[tokio::test]
async fn fs_registry_loads_blessed_agents_directory() {
    let registry =
        FsAgentRegistry::load(agents_dir()).expect("agents/ directory must load cleanly");
    let names = registry.names().await;
    assert!(
        !names.is_empty(),
        "agents/ must contain at least the blessed-tier agents (got empty list)"
    );
}

#[tokio::test]
async fn all_required_blessed_agents_are_registered() {
    let registry = FsAgentRegistry::load(agents_dir()).expect("agents/ loads");
    let names = registry.names().await;
    for &expected in REQUIRED_BLESSED_AGENTS {
        assert!(
            names.iter().any(|n| n == expected),
            "blessed agent `{expected}` not found in agents/; got {names:?}",
        );
    }
}

#[tokio::test]
async fn each_blessed_agent_has_well_formed_spec() {
    let registry = FsAgentRegistry::load(agents_dir()).expect("agents/ loads");
    for &name in REQUIRED_BLESSED_AGENTS {
        let agent: Arc<dyn Agent> = registry
            .get(name)
            .await
            .unwrap_or_else(|| panic!("agent `{name}` resolves"));
        let spec = agent.spec();

        // Cheap structural checks the registry already runs at load,
        // re-asserted here for clarity at test-failure time.
        spec.validate().unwrap_or_else(|e| {
            panic!("agent `{name}` failed spec validation: {e}");
        });
        assert_eq!(spec.name, name);
        assert!(!spec.model.is_empty(), "agent `{name}`: model is empty");
        assert!(
            !spec.instructions.is_empty(),
            "agent `{name}`: instructions body is empty"
        );
        assert!(
            spec.input_schema.is_object(),
            "agent `{name}`: input_schema must be an object"
        );
        assert!(
            spec.output_schema.is_object(),
            "agent `{name}`: output_schema must be an object"
        );
    }
}

#[tokio::test]
async fn each_blessed_agent_schema_compiles_under_production_validator() {
    // The production `record_answer` path feeds the output_schema
    // through `jsonschema::JSONSchema::compile`. A schema that won't
    // compile would silently bounce every answer the agent emits at
    // runtime. Catch that here instead.
    let registry = FsAgentRegistry::load(agents_dir()).expect("agents/ loads");
    for &name in REQUIRED_BLESSED_AGENTS {
        let spec = registry.get(name).await.expect("registered").spec();

        jsonschema::JSONSchema::options()
            .compile(&spec.input_schema)
            .unwrap_or_else(|e| panic!("agent `{name}`: input_schema does not compile: {e}"));
        jsonschema::JSONSchema::options()
            .compile(&spec.output_schema)
            .unwrap_or_else(|e| panic!("agent `{name}`: output_schema does not compile: {e}"));
    }
}

#[tokio::test]
async fn goal_pursuer_advertises_concrete_outcome_enum() {
    // Pin a behavior that goal-judge depends on — pursuer's `outcome`
    // field is a known enum with three concrete values. If somebody
    // edits goal-pursuer.md and breaks this contract, goal-judge's
    // `claimed_outcome` schema (which mirrors it) would silently
    // diverge.
    let registry = FsAgentRegistry::load(agents_dir()).expect("agents/ loads");
    let pursuer = registry.get("goal-pursuer").await.expect("registered");
    let schema = pursuer.spec().output_schema;

    let outcome = schema
        .get("properties")
        .and_then(|p| p.get("outcome"))
        .expect("goal-pursuer output_schema must declare `outcome` property");
    let enum_values = outcome
        .get("enum")
        .and_then(|v| v.as_array())
        .expect("goal-pursuer.outcome must declare an enum");
    let values: Vec<&str> = enum_values.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        values.contains(&"success"),
        "outcome enum missing `success`"
    );
    assert!(
        values.contains(&"partial"),
        "outcome enum missing `partial`"
    );
    assert!(
        values.contains(&"failure"),
        "outcome enum missing `failure`"
    );
}

#[tokio::test]
async fn goal_judge_outcome_enum_matches_pursuer_outcome_enum() {
    // The judge's `claimed_outcome` (its input) MUST stay in lockstep
    // with the pursuer's `outcome` (its output). If a maintainer edits
    // one without the other, the judge would reject every pursuer
    // answer with a schema-violation error and the workflow would
    // grind to a halt.
    let registry = FsAgentRegistry::load(agents_dir()).expect("agents/ loads");

    let pursuer = registry.get("goal-pursuer").await.expect("registered");
    let pursuer_outcome: Vec<String> = pursuer
        .spec()
        .output_schema
        .get("properties")
        .and_then(|p| p.get("outcome"))
        .and_then(|o| o.get("enum"))
        .and_then(|v| v.as_array())
        .expect("pursuer outcome.enum exists")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();

    let judge = registry.get("goal-judge").await.expect("registered");
    let judge_claimed_outcome: Vec<String> = judge
        .spec()
        .input_schema
        .get("properties")
        .and_then(|p| p.get("claimed_outcome"))
        .and_then(|o| o.get("enum"))
        .and_then(|v| v.as_array())
        .expect("judge claimed_outcome.enum exists")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();

    assert_eq!(
        pursuer_outcome, judge_claimed_outcome,
        "goal-judge.claimed_outcome enum must mirror goal-pursuer.outcome enum exactly. \
         Pursuer: {pursuer_outcome:?}; Judge: {judge_claimed_outcome:?}"
    );
}
