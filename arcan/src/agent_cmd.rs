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
//!
//! Live-LLM execution (`arcan agent test` without `--dry-run`) is
//! deliberately out of scope for this PR — it requires wiring an
//! arcan provider stack and is a fast-follow.
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

use anyhow::{Context, Result, anyhow, bail};
use ergon::{AgentRegistry, AgentSpec, FsAgentRegistry, parse_agent_md};
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
        "note: live execution (without --dry-run) is not yet supported in this build; \
         see BRO-1008 follow-ups for the live-LLM path."
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
    let compiled = jsonschema::JSONSchema::options()
        .compile(&spec.input_schema)
        .map_err(|e| anyhow!("agent `{}` input_schema is malformed: {e}", spec.name))?;
    if let Err(errors) = compiled.validate(input) {
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
            "input is INVALID for agent `{}`:\n  - {}",
            spec.name,
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
}
