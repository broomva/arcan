pub mod aisdk;
pub mod context;
pub mod context_compiler;
pub mod error;
pub mod hooks;
pub mod lifecycle;
pub mod prompt;
pub mod protocol;
pub mod protocol_bridge;
pub mod queue;
pub mod runtime;
pub mod state;
pub mod summarization;

pub use context::{CompactionResult, ContextConfig, compact_messages, estimate_tokens};
pub use context_compiler::{
    CompiledContext, ContextBlock, ContextBlockKind, ContextCompilerConfig, compile_context,
};
pub use error::CoreError;
pub use hooks::{HookConfig, HookContext, HookDenied, HookEvent, HookRegistry, HookResult};
pub use lifecycle::LifecycleHook;
pub use protocol::*;
pub use runtime::*;
pub use state::{AppState, StateError};
pub use summarization::{
    COMPRESSED_SUMMARY_HEADER, CompressionOutcome, HeuristicSummarizer, SummarizationConfig,
    SummarizationMiddleware, Summarizer, compressed_block,
};
