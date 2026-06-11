//! Implementation of the `arcan agent` subcommand family.
//!
//! Per architecture spec
//! `core/life/docs/superpowers/specs/2026-05-09-bro-1006-authored-agents-architecture.md`
//! §M5 / §8 (BRO-1008 row), the agent CLI is the operator surface for
//! the authored-agent substrate shipped in BRO-1007 / BRO-1010. It
//! exposes four read-mostly actions over a directory of
//! `agents/<name>.md` files:
//!
//! | Sub-action | Purpose |
//! |---|---|
//! | `list` | One-line-per-agent table of every agent under `<dir>`. |
//! | `show <name>` | Pretty-printed full `AgentSpec` for one agent. |
//! | `new <name>` | Scaffold a new `agents/<name>.md` from a template. |
//! | `test <name> --input <…> --dry-run` | Validate input JSON against the agent's `input_schema`. |
//! | `test <name> --input <…> --live` | Execute the agent against the configured provider (costs money). |
//!
//! ## Live-LLM execution (`--live`, BRO-1008 follow-up)
//!
//! [`test_live`] drives the same `ergon::Agent::run` interpreter the
//! runtime uses, over the same provider adapter chain `arcan serve`
//! builds at boot (`arcan_core::Provider` → [`ArcanProviderAdapter`] →
//! [`ModelProviderAdapter`]). The chain is assembled by
//! [`build_ergon_provider`], which the binary calls in **sync**
//! context (the Anthropic provider's `reqwest::blocking` client owns
//! an inner tokio runtime — constructing or dropping it inside an
//! async context panics; see
//! `crates/arcan/arcan-ergon/tests/anthropic_agents_smoke.rs`).
//!
//! Spend is bounded by [`AGENT_TEST_MAX_TOKENS`] via a
//! [`TokenBudgetHook`] registered on the run — a runaway agent is
//! denied its next inference call once the cumulative token count
//! crosses the cap. Bare `arcan agent test` (neither `--dry-run` nor
//! `--live`) still errors so no invocation spends money by accident.
//!
//! ## Why this module exists
//!
//! `main.rs` is already 1700 LOC. The CLI handlers for `agent` are
//! small but each has a distinct shape (filesystem walk vs single-spec
//! lookup vs scaffold-write vs schema-validate). Keeping them in their
//! own module makes them independently testable from
//! `tests/agent_cmd.rs` (no subprocess invocation needed) and keeps
//! `main.rs` focused on wiring.
//!
//! ## Error contract
//!
//! All public functions return `anyhow::Result<()>` and write
//! human-readable output to stdout. Errors are propagated up to the
//! Clap dispatch in `main.rs`, which converts them to a non-zero exit
//! status. The integration tests assert that `Err(_)` is returned for
//! the "missing agent" / "schema violation" / "refusing to overwrite"
//! cases — that's the contract the surrounding shell relies on.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use aios_protocol::mode::OperatingMode;
use aios_protocol::{BranchId, ModelProviderPort, RunId};
use anyhow::{Context, Result, anyhow, bail};
use arcan_aios_adapters::ArcanProviderAdapter;
use arcan_core::runtime::Provider as ArcanProvider;
use arcan_ergon::{ModeRuntimeHandle, ModelProviderAdapter};
use async_trait::async_trait;
use ergon::{
    AgentRegistry, AgentSpec, BufferSink, ErgonError, FsAgentRegistry, Hook, HookCtx, HookOutcome,
    HookRegistry, InferenceHookOutcome, ModelRequest, ModelResponse, Provider as ErgonProvider,
    SessionId, StepCtx, StreamSink, ToolCall, ToolDefinition, ToolRegistry, ToolResult,
    parse_agent_md,
};
use serde_json::Value;

/// Default model for `arcan agent new` scaffolds. Mirrors the model
/// already used by every blessed agent (see `agents/general.md` etc.)
/// so a freshly scaffolded agent's frontmatter is consistent with what
/// ships in the repo.
const DEFAULT_NEW_AGENT_MODEL: &str = "claude-sonnet-4-5-20250929";

/// Placeholder body inserted into a freshly scaffolded agent. The body
/// is intentionally minimal — the operator is expected to replace it
/// with the agent's actual instructions before checking the file in.
/// We pick a single-line comment so the file is structurally valid
/// (non-empty body — `parse_agent_md` rejects empty bodies) while
/// being obviously a stub.
const DEFAULT_NEW_AGENT_BODY: &str = "<!-- TODO: write agent instructions here -->";

// ─── list ───────────────────────────────────────────────────────────────

/// Implementation of `arcan agent list`.
///
/// Loads every `*.md` file under `dir` via [`FsAgentRegistry::load`]
/// and prints a one-line summary per agent: `name` / `model` /
/// `max_turns` / first non-empty line of `instructions`.
///
/// Returns `Err` if `dir` is not a directory or if any agent file
/// fails to parse — fail-fast matches the runtime's load semantics
/// (the same registry construction is what `arcan serve` runs at boot).
#[allow(clippy::print_stdout)]
pub async fn list(dir: &Path) -> Result<()> {
    let registry = load_registry(dir)?;
    let names = registry.names().await;

    if names.is_empty() {
        println!(
            "no agents found in {}\n\n\
             tip: scaffold one with `arcan agent new <name>` or \
             see <https://github.com/broomva/life/tree/main/agents> for examples.",
            dir.display(),
        );
        return Ok(());
    }

    println!(
        "{:<32} {:<32} {:>10}  INSTRUCTIONS",
        "NAME", "MODEL", "MAX_TURNS",
    );
    let divider = "-".repeat(110);
    println!("{divider}");

    for name in &names {
        let agent = registry
            .get(name)
            .await
            .ok_or_else(|| anyhow!("registry returned name `{name}` but get() missed"))?;
        let spec = agent.spec();
        let first_line = first_nonempty_line(&spec.instructions);
        println!(
            "{:<32} {:<32} {:>10}  {}",
            truncate(&spec.name, 32),
            truncate(&spec.model, 32),
            spec.max_turns,
            truncate(&first_line, 60),
        );
    }

    Ok(())
}

// ─── show ───────────────────────────────────────────────────────────────

/// Implementation of `arcan agent show <name>`.
///
/// Looks up the named agent in the directory-backed registry and
/// pretty-prints every field of its `AgentSpec`: name, model,
/// max_turns, max_retries, allowed_tools, formatted JSON for the
/// input/output schemas, and the full `instructions` body.
///
/// Returns `Err` if the agent isn't registered. The error message
/// includes the directory and the names that *are* registered, so
/// the operator can correct the name without a second command.
#[allow(clippy::print_stdout)]
pub async fn show(dir: &Path, name: &str) -> Result<()> {
    let registry = load_registry(dir)?;
    let Some(agent) = registry.get(name).await else {
        let available = registry.names().await;
        let mut msg = format!("agent `{name}` not found in {}", dir.display(),);
        if available.is_empty() {
            msg.push_str(" (no agents loaded)");
        } else {
            msg.push_str("\nregistered agents:");
            for n in available {
                msg.push_str("\n  - ");
                msg.push_str(&n);
            }
        }
        bail!(msg);
    };
    let spec = agent.spec();
    print_spec(&spec);
    Ok(())
}

/// Pretty-print a single [`AgentSpec`] to stdout. Extracted so the
/// integration tests can assert on the rendered shape without going
/// through the registry-load path.
#[allow(clippy::print_stdout)]
pub fn print_spec(spec: &AgentSpec) {
    println!("=== Agent: {} ===", spec.name);
    println!("model:        {}", spec.model);
    println!("max_turns:    {}", spec.max_turns);
    println!("max_retries:  {}", spec.max_retries);
    match &spec.allowed_tools {
        None => println!("allowed_tools: (inherits workflow registry)"),
        Some(tools) if tools.is_empty() => {
            println!("allowed_tools: [] (no workflow tools — record_answer / spawn_agent only)");
        }
        Some(tools) => {
            println!("allowed_tools:");
            for t in tools {
                println!("  - {t}");
            }
        }
    }
    if !spec.extensions.is_empty() {
        println!("extensions:");
        for (k, v) in &spec.extensions {
            let rendered = serde_json::to_string(v).unwrap_or_else(|_| "<unrenderable>".to_owned());
            println!("  {k}: {rendered}");
        }
    }
    println!();
    println!("=== input_schema ===");
    println!("{}", pretty_json(&spec.input_schema));
    println!();
    println!("=== output_schema ===");
    println!("{}", pretty_json(&spec.output_schema));
    println!();
    println!("=== instructions ===");
    println!("{}", spec.instructions);
}

// ─── new ────────────────────────────────────────────────────────────────

/// Implementation of `arcan agent new <name>`.
///
/// Scaffolds a fresh `<dir>/<name>.md` file with a minimal but
/// well-formed frontmatter template plus a placeholder body. Refuses
/// to overwrite an existing file — the operator must explicitly
/// remove the old one first, matching the spirit of the
/// `InMemoryAgentRegistry::insert` "no silent overwrites" rule.
///
/// The scaffolded shape mirrors `agents/general.md` (the simplest
/// blessed agent) so the result parses immediately under
/// `parse_agent_md`. Operators are expected to replace the placeholder
/// body and tighten the schemas before committing.
#[allow(clippy::print_stdout)]
pub fn new_agent(
    dir: &Path,
    name: &str,
    model: Option<&str>,
    instructions: Option<&str>,
) -> Result<()> {
    if name.is_empty() {
        bail!("agent name must not be empty");
    }
    // Filenames are derived directly from the name; reject anything
    // that would produce a multi-component path or a hidden file.
    if name.contains('/') || name.contains('\\') || name.starts_with('.') {
        bail!(
            "agent name `{name}` is not a valid filename stem \
             (no slashes, no leading dot)"
        );
    }

    std::fs::create_dir_all(dir)
        .with_context(|| format!("failed to ensure agents dir at {}", dir.display()))?;

    let path = dir.join(format!("{name}.md"));
    if path.exists() {
        bail!(
            "refusing to overwrite existing agent file {} \
             (delete it first or pick a different name)",
            path.display()
        );
    }

    let model = model.unwrap_or(DEFAULT_NEW_AGENT_MODEL);
    let body = instructions.unwrap_or(DEFAULT_NEW_AGENT_BODY);
    let content = render_new_agent_template(name, model, body);

    std::fs::write(&path, &content)
        .with_context(|| format!("failed to write {}", path.display()))?;

    // Self-check: the file we just wrote should round-trip cleanly
    // through the same parser the runtime uses. If it doesn't, that's a
    // bug in the template — better to surface it here than at first
    // `arcan serve` boot.
    parse_agent_md(&content).with_context(|| {
        format!(
            "scaffolded agent at {} did not parse — this is a bug in `arcan agent new`",
            path.display()
        )
    })?;

    println!("scaffolded agent `{name}` at {}", path.display());
    println!(
        "next: edit the file (replace the TODO body, refine the schemas), \
         then `arcan agent show {name}` and `arcan agent test {name} --input '{{}}' --dry-run`."
    );
    Ok(())
}

/// Build the markdown content for a fresh authored-agent file.
///
/// Layout matches the simplest blessed agent (`agents/general.md`):
///
/// - `name` mirrors the filename stem (the registry enforces this
///   match at load time).
/// - `model` defaults to the same `claude-sonnet-4-5-…` id every
///   blessed agent uses.
/// - `max_turns` and `max_retries` use the `AgentSpec::new` defaults
///   so an unmodified scaffold behaves like the substrate's defaults.
/// - `input_schema` and `output_schema` are deliberately *minimal but
///   non-empty* (a single string field each). Empty `properties: {}`
///   would parse but be useless; a tiny example invites the operator
///   to extend it.
/// - `instructions` is the supplied body or the default TODO marker.
///
/// Kept as a free function so the integration tests can assert on the
/// rendered shape without spinning up the CLI dispatch.
fn render_new_agent_template(name: &str, model: &str, instructions: &str) -> String {
    format!(
        r#"---
name: {name}
model: {model}
max_turns: 16
max_retries: 3
input_schema:
  type: object
  properties:
    request:
      type: string
      description: TODO — describe the input the agent accepts.
  required: [request]
  additionalProperties: false
output_schema:
  type: object
  properties:
    response:
      type: string
      description: TODO — describe the typed answer the agent must produce.
  required: [response]
  additionalProperties: false
---

# {name}

{instructions}
"#
    )
}

// ─── test --dry-run ─────────────────────────────────────────────────────

/// Implementation of `arcan agent test <name> --input <…> --dry-run`.
///
/// Loads the agent, parses `raw_input` (which may be a literal JSON
/// document or `@<path>` to read from a file), and validates the
/// parsed value against the agent's `input_schema` using the same
/// `jsonschema` crate the runtime uses for `record_answer`
/// validation.
///
/// Returns `Ok(())` on a clean validation, `Err(_)` (carrying the
/// joined schema-violation messages) otherwise. Live LLM execution
/// is out of scope for this PR — see the module-level docs.
#[allow(clippy::print_stdout)]
pub async fn test_dry_run(dir: &Path, name: &str, raw_input: &str) -> Result<()> {
    let registry = load_registry(dir)?;
    let agent = registry
        .get(name)
        .await
        .ok_or_else(|| anyhow!("agent `{name}` not found in {}", dir.display()))?;
    let spec = agent.spec();

    let input = parse_input_arg(raw_input)?;
    validate_input(&spec, &input)?;

    println!(
        "input is valid for agent `{}` (input_schema accepted the payload).",
        spec.name,
    );
    println!(
        "note: to execute this agent against the configured LLM provider, re-run \
         with --live instead of --dry-run (this costs money; capped at \
         {AGENT_TEST_MAX_TOKENS} tokens)."
    );
    Ok(())
}

/// Validate `input` against `spec.input_schema` using the same
/// `jsonschema` crate the runtime uses for `record_answer`. Joins
/// every violation into a single human-readable message so the caller
/// gets all problems at once instead of fixing them one by one.
///
/// Public so the integration tests can drive validation directly
/// without going through the registry load.
pub fn validate_input(spec: &AgentSpec, input: &Value) -> Result<()> {
    validate_against_schema(&spec.input_schema, input, "input", &spec.name)
}

/// Validate `value` against an arbitrary JSON schema, reporting every
/// violation in one joined error message. `what` names the payload in
/// the error text (`"input"` / `"output"`); `agent_name` identifies
/// which agent's schema rejected it.
///
/// This is the shared engine behind [`validate_input`] (dry-run path)
/// and the output check in [`test_live`] — both use the same
/// `jsonschema` crate the ergon runtime uses for `record_answer`
/// validation, so the CLI's accept/reject judgement matches the
/// runtime's.
pub fn validate_against_schema(
    schema: &Value,
    value: &Value,
    what: &str,
    agent_name: &str,
) -> Result<()> {
    let compiled = jsonschema::JSONSchema::options()
        .compile(schema)
        .map_err(|e| anyhow!("agent `{agent_name}` {what}_schema is malformed: {e}"))?;
    if let Err(errors) = compiled.validate(value) {
        let messages: Vec<String> = errors
            .map(|e| {
                let path = e.instance_path.to_string();
                if path.is_empty() {
                    format!("{e}")
                } else {
                    format!("{path}: {e}")
                }
            })
            .collect();
        bail!(
            "{what} is INVALID for agent `{agent_name}`:\n  - {}",
            messages.join("\n  - "),
        );
    }
    Ok(())
}

/// Parse the `--input` argument. Two forms supported:
///
/// - `@<path>` — read the file at `<path>` and parse its contents
///   as JSON. Convenient for non-trivial inputs that don't shell-quote
///   well.
/// - `<json-literal>` — parse the argument directly as JSON.
///
/// Errors carry enough context (file path, JSON-parser message) to
/// fix without re-reading the source.
fn parse_input_arg(raw: &str) -> Result<Value> {
    let json = if let Some(path) = raw.strip_prefix('@') {
        std::fs::read_to_string(path)
            .with_context(|| format!("failed to read --input file at {path}"))?
    } else {
        raw.to_owned()
    };
    serde_json::from_str(&json)
        .with_context(|| format!("failed to parse --input as JSON ({} bytes)", json.len()))
}

// ─── test --live ────────────────────────────────────────────────────────

/// Hard cap on cumulative tokens (input + output) a single
/// `arcan agent test --live` run may consume. Once the cap is crossed
/// the [`TokenBudgetHook`] denies the next inference call, aborting
/// the run with a budget error instead of letting a runaway agent
/// spend unboundedly. At Sonnet pricing 50K tokens is well under a
/// dollar even in the worst (all-output) case.
pub const AGENT_TEST_MAX_TOKENS: u64 = 50_000;

/// Empty tool registry for CLI test runs.
///
/// Mirrors `EmptyTools` in
/// `crates/arcan/arcan-ergon/tests/anthropic_agents_smoke.rs`: the
/// CLI test surface deliberately advertises **no** workflow tools —
/// the framework's own `record_answer` tool is synthesized by
/// `ergon::run_spec` regardless, which is all an authored agent needs
/// to produce its typed answer.
struct CliTestTools;

#[async_trait]
impl ToolRegistry for CliTestTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        Vec::new()
    }
    async fn invoke(&self, call: ToolCall) -> std::result::Result<ToolResult, ErgonError> {
        Err(ErgonError::Tool(format!(
            "CliTestTools cannot invoke `{}` — `arcan agent test --live` runs with no \
             workflow tools",
            call.name
        )))
    }
}

/// Cost guard for live CLI test runs.
///
/// Tracks cumulative token usage across every inference turn
/// (`on_post_inference`) and denies the next provider call
/// (`on_pre_inference`) once the total crosses `max_tokens`. Denial
/// surfaces as `ErgonError::Hook` from the autonomous loop, aborting
/// the run — the answer captured so far (if any) still wins, because
/// `ergon::run_spec` checks the answer slot before inspecting the
/// loop result.
///
/// Also serves as the usage ledger for the post-run summary line:
/// [`Self::input_tokens`] / [`Self::output_tokens`] expose what the
/// run actually consumed.
pub struct TokenBudgetHook {
    max_tokens: u64,
    input_used: AtomicU64,
    output_used: AtomicU64,
}

impl TokenBudgetHook {
    /// Construct a guard with the given cumulative token cap.
    pub fn new(max_tokens: u64) -> Self {
        Self {
            max_tokens,
            input_used: AtomicU64::new(0),
            output_used: AtomicU64::new(0),
        }
    }

    /// Input tokens consumed so far.
    pub fn input_tokens(&self) -> u64 {
        self.input_used.load(Ordering::Relaxed)
    }

    /// Output tokens consumed so far.
    pub fn output_tokens(&self) -> u64 {
        self.output_used.load(Ordering::Relaxed)
    }

    /// Total tokens consumed so far (input + output).
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens() + self.output_tokens()
    }
}

#[async_trait]
impl Hook for TokenBudgetHook {
    fn name(&self) -> &str {
        "agent-test-token-budget"
    }

    async fn on_pre_inference(
        &self,
        _ctx: &HookCtx<'_>,
        _req: &mut ModelRequest,
    ) -> std::result::Result<InferenceHookOutcome, ErgonError> {
        let used = self.total_tokens();
        if used >= self.max_tokens {
            return Ok(InferenceHookOutcome::Deny(format!(
                "agent test token budget exhausted: used {used} tokens >= cap {} \
                 (AGENT_TEST_MAX_TOKENS)",
                self.max_tokens
            )));
        }
        Ok(InferenceHookOutcome::Continue)
    }

    async fn on_post_inference(
        &self,
        _ctx: &HookCtx<'_>,
        resp: &ModelResponse,
    ) -> std::result::Result<HookOutcome, ErgonError> {
        self.input_used
            .fetch_add(u64::from(resp.usage.input_tokens), Ordering::Relaxed);
        self.output_used
            .fetch_add(u64::from(resp.usage.output_tokens), Ordering::Relaxed);
        Ok(HookOutcome::Continue)
    }
}

/// Adapt an `arcan_core::Provider` into the `ergon::Provider` the
/// agent interpreter consumes.
///
/// This is the **same chain the runtime uses** (see
/// `crates/arcan/arcan-ergon/tests/anthropic_agents_smoke.rs` and the
/// `arcan serve` boot path):
///
/// ```text
/// arcan_core::Provider
///   → ArcanProviderAdapter   (aios_protocol::ModelProviderPort)
///   → ModelProviderAdapter   (ergon::Provider)
/// ```
///
/// `provider_name` is what `ergon::Provider::name` reports (embedded
/// in `StreamEvent::SessionStart`); pass the resolved provider label
/// (`"anthropic"`, `"ollama"`, …).
///
/// ## Sync-context requirement
///
/// MUST be called from a **sync** context (outside any tokio
/// runtime), and the returned `Arc` must outlive the `block_on` that
/// drives [`test_live`] so the final drop also happens in sync
/// context. The Anthropic provider holds a `reqwest::blocking::Client`
/// whose inner tokio runtime panics if constructed or dropped from
/// async context.
pub fn build_ergon_provider(
    provider: Arc<dyn ArcanProvider>,
    provider_name: &str,
) -> Arc<dyn ErgonProvider> {
    // ArcanProviderAdapter wants a tools list (empty — authored agents
    // get their `record_answer` tool from the chained registry inside
    // `ergon::run_spec`) and a streaming-sender handle (empty mutex —
    // the CLI does not subscribe to the runtime broadcast).
    let streaming_sender = Arc::new(std::sync::Mutex::new(None));
    let port: Arc<dyn ModelProviderPort> = Arc::new(ArcanProviderAdapter::new(
        provider,
        Vec::new(),
        streaming_sender,
    ));

    Arc::new(ModelProviderAdapter::new(
        port,
        aios_protocol::SessionId::from_string("agent-test-live".to_owned()),
        BranchId::main(),
        RunId::new_uuid(),
        provider_name,
    ))
}

/// Implementation of `arcan agent test <name> --input <…> --live`.
///
/// Loads the agent from `dir`, validates the parsed input against the
/// agent's `input_schema` (same gate as `--dry-run`), then drives the
/// production `ergon::Agent::run` interpreter against `provider`:
/// empty tools ([`CliTestTools`]), a [`TokenBudgetHook`] capped at
/// [`AGENT_TEST_MAX_TOKENS`], a [`BufferSink`], and an Execute-mode
/// [`ModeRuntimeHandle`]. On success the typed answer is re-validated
/// against the agent's `output_schema` (defense in depth — the
/// interpreter already validated it), pretty-printed to stdout, and
/// followed by a one-line usage/cost summary.
///
/// `provider` must come from [`build_ergon_provider`] called in sync
/// context — see its docs for the `reqwest::blocking` drop-order
/// constraint.
#[allow(clippy::print_stdout)]
pub async fn test_live(
    dir: &Path,
    name: &str,
    raw_input: &str,
    provider: Arc<dyn ErgonProvider>,
) -> Result<()> {
    let registry = load_registry(dir)?;
    let agent = registry
        .get(name)
        .await
        .ok_or_else(|| anyhow!("agent `{name}` not found in {}", dir.display()))?;
    let spec = agent.spec();

    // Same input gate as --dry-run: never spend tokens on a payload
    // the agent's own schema would reject.
    let input = parse_input_arg(raw_input)?;
    validate_input(&spec, &input)?;

    let budget = Arc::new(TokenBudgetHook::new(AGENT_TEST_MAX_TOKENS));
    let hooks = HookRegistry::new().with_arc(Arc::clone(&budget) as Arc<dyn Hook>);

    let mut ctx = StepCtx::new(
        SessionId::from_string("agent-test-live".to_owned()),
        "agent-test",
        provider,
        Arc::new(CliTestTools) as Arc<dyn ToolRegistry>,
        Arc::new(hooks),
        Arc::new(BufferSink::new()) as Arc<dyn StreamSink>,
        Arc::new(ModeRuntimeHandle::new(OperatingMode::Execute)) as Arc<dyn ergon::RuntimeHandle>,
        tracing::Span::current(),
    );

    println!(
        "running agent `{}` live (model {}, max_turns {}, token cap {AGENT_TEST_MAX_TOKENS})…",
        spec.name, spec.model, spec.max_turns,
    );

    let answer = agent
        .run(&mut ctx, input)
        .await
        .map_err(|e| anyhow!("agent `{}` live run failed: {e}", spec.name))?;

    // Defense in depth: `ergon::run_spec` already validated the
    // captured answer, but re-checking here surfaces a clear CLI error
    // if that invariant is ever bypassed.
    validate_against_schema(&spec.output_schema, &answer, "output", &spec.name)?;

    println!("{}", pretty_json(&answer));
    let input_tokens = budget.input_tokens();
    let output_tokens = budget.output_tokens();
    let cost = crate::cost::estimate_cost(input_tokens, output_tokens, &spec.model);
    println!(
        "agent `{}` OK — model {} | tokens: {input_tokens} in / {output_tokens} out \
         (cap {AGENT_TEST_MAX_TOKENS}) | est. cost ${cost:.4}",
        spec.name, spec.model,
    );
    Ok(())
}

// ─── helpers ────────────────────────────────────────────────────────────

/// Load the agents directory through [`FsAgentRegistry::load`] with a
/// helpful error if the directory is missing. Centralizes the path-
/// validation message so each subcommand surfaces the same failure
/// mode consistently.
fn load_registry(dir: &Path) -> Result<FsAgentRegistry> {
    if !dir.exists() {
        bail!(
            "agents directory {} does not exist \
             (pass --agents-dir <path> or create one — see agents/README.md)",
            dir.display()
        );
    }
    if !dir.is_dir() {
        bail!("agents path {} is not a directory", dir.display());
    }
    FsAgentRegistry::load(dir)
        .with_context(|| format!("failed to load agents from {}", dir.display()))
}

/// First non-empty (whitespace-stripped) line of `s`. Used by `list`
/// to surface a one-liner from the agent's full instructions block.
/// Returns `""` if `s` has no non-empty lines.
fn first_nonempty_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_owned()
}

/// Truncate `s` to at most `max_chars` characters (NOT bytes — counts
/// `char`s so multi-byte UTF-8 characters don't get split). Appends
/// `…` if truncation actually happened. Width-agnostic — fine for the
/// table layout because terminal width budgets cope with single
/// trailing ellipses.
fn truncate(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_owned()
    } else if max_chars == 0 {
        String::new()
    } else {
        // `max_chars - 1` to leave room for the ellipsis.
        let take = max_chars.saturating_sub(1);
        let mut out: String = s.chars().take(take).collect();
        out.push('…');
        out
    }
}

/// Render a JSON value as a pretty-printed string. Used for schema
/// display. Falls back to a marker if serialization unexpectedly
/// fails (it shouldn't — `Value` always round-trips through serde).
fn pretty_json(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| "<unrenderable JSON>".to_owned())
}

/// Resolve the agents directory CLI flag to a concrete path, applying
/// the same default (`./agents/`) the `serve` subcommand uses. Kept
/// here so the agent CLI handlers don't have to repeat the
/// `unwrap_or_else` dance.
pub fn resolve_agents_dir(arg: Option<PathBuf>) -> PathBuf {
    arg.unwrap_or_else(|| PathBuf::from("agents"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_nonempty_line_skips_blank_prefix() {
        assert_eq!(first_nonempty_line("\n\n  hello world\n"), "hello world");
        assert_eq!(first_nonempty_line(""), "");
        assert_eq!(first_nonempty_line("   "), "");
    }

    #[test]
    fn truncate_handles_short_and_long() {
        assert_eq!(truncate("abc", 10), "abc");
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("", 5), "");
        // Multi-byte UTF-8: every "é" is one char, so a 3-char string
        // truncated to 2 chars yields one char + ellipsis.
        assert_eq!(truncate("éèà", 2), "é…");
    }

    #[test]
    fn render_new_agent_template_round_trips() {
        let rendered = render_new_agent_template("test-agent", DEFAULT_NEW_AGENT_MODEL, "body");
        let spec = parse_agent_md(&rendered).expect("template must parse");
        assert_eq!(spec.name, "test-agent");
        assert_eq!(spec.model, DEFAULT_NEW_AGENT_MODEL);
        assert!(spec.instructions.contains("body"));
    }

    // ─── test --live plumbing (offline — no network, no real provider) ──

    use ergon::{ContentBlock, StopReason, Usage};

    fn hook_ctx<'a>(span: &'a tracing::Span) -> HookCtx<'a> {
        HookCtx::new(
            SessionId::from_string("agent-test-unit".to_owned()),
            "agent-test",
            span,
        )
    }

    fn response_with_usage(input: u32, output: u32) -> ModelResponse {
        let mut usage = Usage::default();
        usage.input_tokens = input;
        usage.output_tokens = output;
        ModelResponse::new(vec![ContentBlock::text("ok")], StopReason::EndTurn).with_usage(usage)
    }

    #[tokio::test]
    async fn token_budget_hook_continues_under_cap() {
        let span = tracing::Span::current();
        let ctx = hook_ctx(&span);
        let hook = TokenBudgetHook::new(1_000);

        let mut req = ModelRequest::new("m", vec![]);
        assert!(matches!(
            hook.on_pre_inference(&ctx, &mut req).await.unwrap(),
            InferenceHookOutcome::Continue
        ));
    }

    #[tokio::test]
    async fn token_budget_hook_accumulates_usage_and_denies_over_cap() {
        let span = tracing::Span::current();
        let ctx = hook_ctx(&span);
        let hook = TokenBudgetHook::new(100);

        // Record 60 in + 50 out = 110 total — over the 100 cap.
        hook.on_post_inference(&ctx, &response_with_usage(60, 50))
            .await
            .unwrap();
        assert_eq!(hook.input_tokens(), 60);
        assert_eq!(hook.output_tokens(), 50);
        assert_eq!(hook.total_tokens(), 110);

        let mut req = ModelRequest::new("m", vec![]);
        match hook.on_pre_inference(&ctx, &mut req).await.unwrap() {
            InferenceHookOutcome::Deny(reason) => {
                assert!(
                    reason.contains("token budget exhausted"),
                    "deny reason must explain the cap: {reason}"
                );
                assert!(
                    reason.contains("110"),
                    "deny reason carries usage: {reason}"
                );
            }
            other => panic!("expected Deny over cap, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn token_budget_hook_denies_exactly_at_cap() {
        let span = tracing::Span::current();
        let ctx = hook_ctx(&span);
        let hook = TokenBudgetHook::new(50);
        hook.on_post_inference(&ctx, &response_with_usage(25, 25))
            .await
            .unwrap();
        let mut req = ModelRequest::new("m", vec![]);
        assert!(matches!(
            hook.on_pre_inference(&ctx, &mut req).await.unwrap(),
            InferenceHookOutcome::Deny(_)
        ));
    }

    #[test]
    fn validate_against_schema_accepts_and_rejects_output() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "response": { "type": "string" } },
            "required": ["response"],
            "additionalProperties": false,
        });
        validate_against_schema(
            &schema,
            &serde_json::json!({"response": "hi"}),
            "output",
            "a",
        )
        .expect("conformant output must pass");

        let err = validate_against_schema(&schema, &serde_json::json!({"bogus": 1}), "output", "a")
            .expect_err("non-conformant output must fail");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("output is INVALID for agent `a`"),
            "error names the payload and agent: {msg}"
        );
    }

    /// Scripted `ergon::Provider` for offline `test_live` runs.
    ///
    /// Turn 1: emit a `record_answer` tool call carrying `answer`.
    /// Turn 2+: plain EndTurn text (the loop's natural exit).
    /// Every turn reports `usage_per_turn` tokens so the budget hook
    /// ledger is exercised.
    struct ScriptedProvider {
        answer: Value,
        usage_per_turn: u32,
        calls: AtomicU64,
    }

    #[async_trait]
    impl ErgonProvider for ScriptedProvider {
        fn name(&self) -> &str {
            "scripted"
        }

        async fn stream(
            &self,
            _req: ModelRequest,
            _sink: Arc<dyn StreamSink>,
        ) -> std::result::Result<ModelResponse, ErgonError> {
            let call_n = self.calls.fetch_add(1, Ordering::Relaxed);
            let mut usage = Usage::default();
            usage.input_tokens = self.usage_per_turn;
            usage.output_tokens = self.usage_per_turn;
            let response = if call_n == 0 {
                // `record_answer` wraps the typed answer as
                // `{"answer": <payload>}` — mirror what a real model
                // following the Output Contract emits.
                ModelResponse::new(
                    vec![ContentBlock::ToolUse {
                        id: "call-1".to_owned(),
                        name: ergon::RECORD_ANSWER_TOOL.to_owned(),
                        input: serde_json::json!({ "answer": self.answer.clone() }),
                    }],
                    StopReason::ToolUse,
                )
            } else {
                ModelResponse::new(vec![ContentBlock::text("done")], StopReason::EndTurn)
            };
            Ok(response.with_usage(usage))
        }
    }

    /// Write a minimal agent file usable by `test_live` into a fresh
    /// temp dir; returns the dir.
    fn scripted_agents_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "arcan-agent-test-live-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(&dir).expect("create temp agents dir");
        std::fs::write(
            dir.join("echo.md"),
            render_new_agent_template("echo", DEFAULT_NEW_AGENT_MODEL, "Echo the request."),
        )
        .expect("write echo agent");
        dir
    }

    #[tokio::test]
    async fn test_live_round_trips_with_scripted_provider() {
        let dir = scripted_agents_dir();
        let provider: Arc<dyn ErgonProvider> = Arc::new(ScriptedProvider {
            answer: serde_json::json!({"response": "echoed"}),
            usage_per_turn: 10,
            calls: AtomicU64::new(0),
        });

        test_live(&dir, "echo", r#"{"request": "hello"}"#, provider)
            .await
            .expect("scripted live run must succeed");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_live_rejects_input_before_calling_provider() {
        let dir = scripted_agents_dir();
        let scripted = Arc::new(ScriptedProvider {
            answer: serde_json::json!({"response": "never"}),
            usage_per_turn: 10,
            calls: AtomicU64::new(0),
        });
        let provider: Arc<dyn ErgonProvider> = Arc::clone(&scripted) as Arc<dyn ErgonProvider>;

        let err = test_live(&dir, "echo", r#"{"wrong": 1}"#, provider)
            .await
            .expect_err("schema-invalid input must fail before any spend");
        assert!(format!("{err:#}").contains("INVALID"));

        // The provider must never have been called — input gating
        // happens before the loop starts.
        assert_eq!(
            scripted.calls.load(Ordering::Relaxed),
            0,
            "provider must not be invoked on invalid input"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_live_surfaces_schema_violation_from_agent_answer() {
        let dir = scripted_agents_dir();
        // The scripted answer violates `echo`'s output_schema (which
        // requires `response: string`); run_spec retries then fails.
        let provider: Arc<dyn ErgonProvider> = Arc::new(ScriptedProvider {
            answer: serde_json::json!({"not_response": 42}),
            usage_per_turn: 10,
            calls: AtomicU64::new(0),
        });

        let err = test_live(&dir, "echo", r#"{"request": "hello"}"#, provider)
            .await
            .expect_err("schema-violating answer must surface as an error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("live run failed"),
            "error wraps the interpreter failure: {msg}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
