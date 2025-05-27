# @repo/core-schemas

Core JSON Schema definitions for the Arcan AI-Native Meta-Platform.

## Overview

This package contains the foundational JSON Schema definitions that power the Arcan platform. These schemas define the structure and validation rules for the entire Arcan ecosystem, from agents and their capabilities to users, organizations, marketplace, and governance.

## Schema Categories

### Core Entity Schemas
- **ArcanAgentBase.v1.json** - Base schema for Arcan agents with identity, capabilities, state, and communication interfaces
- **ArcanAgentManifest.v1.json** - Agent manifest defining capabilities, protocols, and pricing models
- **ArcanSpell.v1.json** - Agent tools/capabilities aligned with MCP tools, A2A skills, and AG-UI actions
- **ArcanTome.v1.json** - Agent resources/data sources with various access mechanisms
- **ArcanWorkflow.v1.json** - LangGraph-based workflow definitions
- **ArcanWorkflowState.v1.json** - Runtime state of workflow executions
- **ArcanEvent.v1.json** - Base event schema for the platform's event-driven architecture

### Event Schemas
- **events/AgentCreatedEvent.v1.json** - Event emitted when agents are created
- **events/WorkflowCompletedEvent.v1.json** - Event emitted when workflows complete
- **events/AgentCollaborationEvent.v1.json** - Event for agent-to-agent collaboration

### Platform Schemas
- **platform/User.v1.json** - Individual platform users with authentication and blockchain identity
- **platform/UserProfile.v1.json** - Detailed user profile information
- **platform/UserPreferences.v1.json** - User-specific platform settings
- **platform/Organization.v1.json** - Tenant companies/teams with multi-tenancy support
- **platform/OrganizationMember.v1.json** - User membership within organizations
- **platform/SubscriptionPlan.v1.json** - SaaS subscription plans with quotas and features
- **platform/ResourceUsageLog.v1.json** - Resource consumption tracking for billing

### Role Schemas
- **roles/AgentRoleTemplate.v1.json** - Templates for common organizational agent roles
- **roles/FinanceAgent.v1.json** - Specialized finance agent with accounting capabilities
- **roles/SalesAgent.v1.json** - Sales automation, CRM integration, and lead management
- **roles/HRAgent.v1.json** - Human resources, recruitment, and employee lifecycle management
- **roles/MarketingAgent.v1.json** - Marketing automation, content creation, and campaign management
- **roles/EngineeringAgent.v1.json** - Software development, DevOps, and infrastructure management
- **roles/CustomerServiceAgent.v1.json** - Customer support, helpdesk, and service operations
- **roles/DataAnalystAgent.v1.json** - Data analysis, business intelligence, and reporting
- **roles/LegalAgent.v1.json** - Legal operations, compliance, and contract management
- **roles/ExecutiveAgent.v1.json** - Strategic planning, decision-making, and executive operations
- **roles/ProductAgent.v1.json** - Product management, roadmap planning, and feature prioritization

### Marketplace Schemas
- **marketplace/MarketplaceListing.v1.json** - Listings for agents, spells, and tomes in the marketplace

### Governance Schemas
- **governance/DAOProposal.v1.json** - Platform governance proposals for the Arcan DAO

## Usage

### TypeScript/JavaScript

```typescript
import { 
  ArcanAgentBase, 
  ArcanSpell,
  User,
  Organization,
  FinanceAgent,
  SalesAgent,
  validateSchema,
  SchemaRegistry 
} from '@repo/core-schemas';

// Validate data against a schema
const agent = {
  agentId: '123e4567-e89b-12d3-a456-426614174000',
  version: '1.0.0',
  name: 'MyAgent',
  // ... other fields
};

const result = validateSchema(ArcanAgentBase, agent);
if (result.valid) {
  console.log('Agent is valid!');
} else {
  console.error('Validation errors:', result.errors);
}

// Use the schema registry
const registry = new SchemaRegistry();
registry.registerSchema(ArcanAgentBase);
registry.validate('https://arcan.ai/schemas/ArcanAgentBase.v1.json', agent);
```

### Python (via generated models)

The schemas in this package are used to generate Python Pydantic models via the code generation pipeline.

## Schema Features

### Multi-Tenancy Support
All schemas are designed with multi-tenancy in mind, supporting organization-based isolation and resource management.

### Blockchain Integration
User and Organization schemas include optional blockchain identity fields for Web3 integration via Thirdweb.

### Role-Based Access Control
Platform schemas support fine-grained permissions and role-based access control at multiple levels.

### Event-Driven Architecture
Comprehensive event schemas enable real-time updates and audit trails across the platform.

### Marketplace Economy
Marketplace schemas support various pricing models including subscriptions, pay-per-use, and token-based payments.

### DAO Governance
Governance schemas enable decentralized decision-making for platform evolution and resource allocation.

### Full-Stack AI Company Support
Role schemas cover all major departments and functions needed for a complete AI-powered organization:
- **Finance** - Accounting, reporting, compliance, and financial analysis
- **Sales** - CRM integration, lead management, and revenue operations
- **HR** - Recruitment, employee management, and organizational development
- **Marketing** - Campaign management, content creation, and analytics
- **Engineering** - Software development, DevOps, and infrastructure
- **Customer Service** - Support ticketing, knowledge management, and customer satisfaction
- **Data Analytics** - Business intelligence, reporting, and data-driven insights
- **Legal** - Contract management, compliance, and risk assessment
- **Executive** - Strategic planning, decision-making, and organizational leadership
- **Product** - Product management, roadmap planning, and user experience

## Schema Versioning

All schemas follow semantic versioning:
- Schema IDs include version numbers (e.g., `ArcanAgentBase.v1.json`)
- Breaking changes require a new major version
- The schema registry supports multiple versions of the same schema
- DAO governance required for major schema changes

## Protocol Alignment

The schemas are designed to align with industry standards:

- **MCP (Model Context Protocol)** - Spells map to MCP tools, Tomes to MCP resources
- **A2A (Agent-to-Agent)** - Spells map to A2A skills, supports FIPA ACL for agent collaboration
- **AG-UI (Agent-UI)** - Supports real-time UI updates via event streaming
- **Web3 Standards** - EVM wallet addresses, on-chain identities, DAO governance

## Development

### Adding New Schemas

1. Create the schema file in the appropriate subdirectory:
   - `src/definitions/` - Core entity schemas
   - `src/definitions/events/` - Event schemas
   - `src/definitions/platform/` - Platform management schemas
   - `src/definitions/roles/` - Agent role templates
   - `src/definitions/marketplace/` - Marketplace schemas
   - `src/definitions/governance/` - DAO governance schemas

2. Follow the naming convention: `SchemaName.v1.json`

3. Export it from `src/definitions/index.ts`

4. Add tests in `src/schemas/index.test.ts`

5. Run validation tests: `npm test`

### Schema Guidelines

- Use JSON Schema Draft 7
- Include `$schema` and `$id` properties
- Provide clear descriptions for all fields
- Use appropriate format validators (uuid, date-time, uri, email, etc.)
- Define required fields explicitly
- Use semantic versioning in schema IDs
- Consider multi-tenancy implications
- Include proper validation patterns for blockchain addresses
- Support extensibility through `additionalProperties` where appropriate

## Testing

```bash
# Run tests
npm test

# Run tests in watch mode
npm run test:watch

# Run tests with coverage
npm run test:coverage
```

## Building

```bash
# Build the package
npm run build

# Build in watch mode
npm run dev
```

## Schema Documentation

### Agent Schemas
Agents are the core entities in Arcan, representing autonomous AI entities that can execute tasks, manage resources, and collaborate with other agents.

### User & Organization Schemas
Support multi-tenant SaaS operations with user management, organization hierarchies, and subscription billing.

### Role Templates
Pre-defined templates for common organizational roles (Finance, HR, Sales, etc.) that can be instantiated as agents. These templates define:
- Required capabilities (spells)
- Necessary data sources (tomes)
- Communication protocols
- Integration requirements
- Performance metrics
- Compliance needs

### Marketplace Schemas
Enable a thriving ecosystem where agents, spells, and tomes can be shared, sold, and monetized.

### Governance Schemas
Support decentralized governance through the Arcan DAO, enabling community-driven platform evolution.

## License

Part of the Arcan platform - see root LICENSE file.
