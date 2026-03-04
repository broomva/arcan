mod error;
mod middleware;
mod mock;
mod port;
#[cfg(feature = "spacetimedb")]
mod spacetimedb;
mod tools;

pub use error::SpacesBridgeError;
pub use middleware::SpacesActivityMiddleware;
pub use mock::MockSpacesClient;
pub use port::{
    SpacesChannel, SpacesChannelType, SpacesDirectMessage, SpacesMessage, SpacesMessageType,
    SpacesPort,
};
#[cfg(feature = "spacetimedb")]
pub use spacetimedb::{SpacetimeDbClient, SpacetimeDbConfig};
pub use tools::{
    SpacesListChannelsTool, SpacesReadMessagesTool, SpacesSendDmTool, SpacesSendMessageTool,
};

use arcan_core::runtime::ToolRegistry;
use std::sync::Arc;

/// Register all Spaces tools into a tool registry.
pub fn register_spaces_tools(registry: &mut ToolRegistry, port: Arc<dyn SpacesPort>) {
    registry.register(SpacesSendMessageTool::new(port.clone()));
    registry.register(SpacesListChannelsTool::new(port.clone()));
    registry.register(SpacesReadMessagesTool::new(port.clone()));
    registry.register(SpacesSendDmTool::new(port));
}
