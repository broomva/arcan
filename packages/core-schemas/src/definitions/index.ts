// Core Entity Schemas
export { default as ArcanAgentBase } from './ArcanAgentBase.v1.json';
export { default as ArcanAgentManifest } from './ArcanAgentManifest.v1.json';
export { default as ArcanSpell } from './ArcanSpell.v1.json';
export { default as ArcanTome } from './ArcanTome.v1.json';
export { default as ArcanWorkflow } from './ArcanWorkflow.v1.json';
export { default as ArcanWorkflowState } from './ArcanWorkflowState.v1.json';
export { default as ArcanEvent } from './ArcanEvent.v1.json';

// Event Schemas
export { default as AgentCreatedEvent } from './events/AgentCreatedEvent.v1.json';
export { default as WorkflowCompletedEvent } from './events/WorkflowCompletedEvent.v1.json';

// Schema Metadata
export const SCHEMA_VERSION = '1.0.0';
export const SCHEMA_BASE_URL = 'https://arcan.ai/schemas/'; 