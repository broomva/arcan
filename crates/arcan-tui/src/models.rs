pub mod scroll;
pub mod state;
pub mod ui_block;

// Re-export commonly used types for backward compatibility
pub use scroll::ScrollState;
pub use state::{AppState, ConnectionStatus, ErrorFlash};
pub use ui_block::{ApprovalRequest, ToolStatus, UiBlock};
