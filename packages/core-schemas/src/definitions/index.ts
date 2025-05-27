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
export { default as AgentCollaborationEvent } from './events/AgentCollaborationEvent.v1.json';

// Platform Schemas
export { default as User } from './platform/User.v1.json';
export { default as UserProfile } from './platform/UserProfile.v1.json';
export { default as UserPreferences } from './platform/UserPreferences.v1.json';
export { default as Organization } from './platform/Organization.v1.json';
export { default as OrganizationMember } from './platform/OrganizationMember.v1.json';
export { default as SubscriptionPlan } from './platform/SubscriptionPlan.v1.json';
export { default as ResourceUsageLog } from './platform/ResourceUsageLog.v1.json';

// Role Schemas
export { default as AgentRoleTemplate } from './roles/AgentRoleTemplate.v1.json';
export { default as FinanceAgent } from './roles/FinanceAgent.v1.json';
export { default as SalesAgent } from './roles/SalesAgent.v1.json';
export { default as HRAgent } from './roles/HRAgent.v1.json';
export { default as MarketingAgent } from './roles/MarketingAgent.v1.json';
export { default as EngineeringAgent } from './roles/EngineeringAgent.v1.json';
export { default as CustomerServiceAgent } from './roles/CustomerServiceAgent.v1.json';
export { default as DataAnalystAgent } from './roles/DataAnalystAgent.v1.json';
export { default as LegalAgent } from './roles/LegalAgent.v1.json';
export { default as ExecutiveAgent } from './roles/ExecutiveAgent.v1.json';
export { default as ProductAgent } from './roles/ProductAgent.v1.json';

// Marketplace Schemas
export { default as MarketplaceListing } from './marketplace/MarketplaceListing.v1.json';

// Governance Schemas
export { default as DAOProposal } from './governance/DAOProposal.v1.json';

// Schema Metadata
export const SCHEMA_VERSION = '1.0.0';
export const SCHEMA_BASE_URL = 'https://arcan.ai/schemas/'; 