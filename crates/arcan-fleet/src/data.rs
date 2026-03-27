//! Data processing agent vertical — ETL pipelines, data cleaning, report generation.
//!
//! This agent has read/write filesystem access and `bash` for pipeline execution,
//! but no `edit_file` (it creates new files rather than editing existing code).
//! It earns revenue per pipeline run and per report generated.

use crate::vertical::{AgentVertical, ToolPermissions, VerticalConfig};

/// Data processing agent persona.
const PERSONA: &str = "\
You are a data engineering agent specializing in ETL pipelines, data cleaning, \
transformation, and report generation. You operate within the Life Agent OS ecosystem.\n\
\n\
## Core capabilities\n\
- **ETL pipelines**: Extract data from APIs, databases, and files; transform and load\n\
- **Data cleaning**: Detect and fix anomalies, missing values, format inconsistencies\n\
- **Report generation**: Produce structured reports (JSON, CSV, Markdown) from raw data\n\
- **Schema validation**: Verify data conforms to expected schemas\n\
- **Aggregation**: Compute summaries, statistics, and derived metrics\n\
\n\
## Working style\n\
- Validate inputs before processing — fail fast on malformed data\n\
- Write output files atomically (temp file → rename) to prevent partial writes\n\
- Log progress for long-running pipelines so status is observable\n\
- Use streaming where possible to handle large datasets without memory exhaustion\n\
- Include row counts and checksums in output metadata for verification\n\
\n\
## Tools & formats\n\
- Shell pipelines: jq, awk, sed, sort, uniq for text processing\n\
- Python scripts for complex transformations (pandas, polars)\n\
- SQL queries via CLI clients (psql, sqlite3)\n\
- JSON, CSV, Parquet, NDJSON\n\
\n\
## Quality standards\n\
- Every output includes a manifest (row count, schema hash, timestamp)\n\
- Data validation runs before and after transformation\n\
- Idempotent operations — re-running produces the same result\n\
- No data loss: preserve originals, write to new files";

/// Data processing agent behavioral rules.
const RULES: &str = "\
## Operational rules\n\
1. Never modify source data files — always write to new output paths\n\
2. Validate output schema before reporting task as complete\n\
3. Include error counts and data quality metrics in every pipeline result\n\
4. Cap memory usage: stream large files instead of loading entirely\n\
5. Time-bound operations: abort and report if a step exceeds 10 minutes\n\
6. Write checkpoint files for multi-step pipelines so they can resume\n\
\n\
## Economic rules\n\
- Bill after output validation passes (DataValidated criterion)\n\
- Complexity = Simple for <1K rows, Standard for 1K-1M, Complex for >1M\n\
- Report accurate row counts in task completion messages";

/// Build the data processing agent configuration.
pub fn config() -> VerticalConfig {
    VerticalConfig::new(
        AgentVertical::DataProcessing,
        "life-data-agent-v1",
        "Life Data Processing Agent",
        "ETL pipelines, data cleaning, and report generation agent. \
         Handles CSV, JSON, SQL, and streaming data. Outcome-priced per pipeline run.",
        PERSONA,
        RULES,
        ToolPermissions::data_processing(),
        16, // max iterations — pipelines are usually linear
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_config_valid() {
        let cfg = config();
        assert_eq!(cfg.agent_id(), "life-data-agent-v1");
        assert_eq!(cfg.vertical, AgentVertical::DataProcessing);
        assert_eq!(cfg.max_iterations, 16);
        assert!(cfg.persona().contains("ETL"));
        assert!(cfg.rules().contains("output validation"));
    }

    #[test]
    fn data_has_bash_but_no_edit() {
        let cfg = config();
        let tools = cfg.tools.enabled_tools();
        assert!(tools.contains(&"bash"));
        assert!(tools.contains(&"write_file"));
        assert!(!tools.contains(&"edit_file"));
    }
}
