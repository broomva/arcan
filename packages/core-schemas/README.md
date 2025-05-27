# @repo/core-schemas

Core JSON Schema definitions for the Arcan AI-Native Meta-Platform.

## Overview

This package contains the foundational JSON Schema definitions that power the Arcan platform. These schemas define the structure and validation rules for:

- **Agents** - Intelligent, autonomous entities with capabilities (Spells) and resources (Tomes)
- **Spells** - Tools, actions, or capabilities that agents can execute
- **Tomes** - Knowledge bases, data sources, or resources that agents can access
- **Workflows** - LangGraph-based workflow definitions and execution states
- **Events** - Platform events for the event-driven architecture
- **Manifests** - Agent manifests defining capabilities, protocols, and pricing

## Schema Structure

### Core Entity Schemas

- `ArcanAgentBase.v1.json` - Base schema for Arcan agents
- `ArcanAgentManifest.v1.json` - Agent manifest with capabilities and protocols
- `ArcanSpell.v1.json` - Schema for agent tools/capabilities
- `ArcanTome.v1.json` - Schema for agent resources/data sources
- `ArcanWorkflow.v1.json` - LangGraph workflow definitions
- `ArcanWorkflowState.v1.json` - Runtime state of workflow executions
- `ArcanEvent.v1.json` - Base event schema for the platform

### Event Schemas

- `events/AgentCreatedEvent.v1.json` - Event emitted when agents are created
- `events/WorkflowCompletedEvent.v1.json` - Event emitted when workflows complete

## Usage

### TypeScript/JavaScript

```typescript
import { 
  ArcanAgentBase, 
  ArcanSpell, 
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

## Schema Versioning

All schemas follow semantic versioning:
- Schema IDs include version numbers (e.g., `ArcanAgentBase.v1.json`)
- Breaking changes require a new major version
- The schema registry supports multiple versions of the same schema

## Protocol Alignment

The schemas are designed to align with industry standards:

- **MCP (Model Context Protocol)** - Spells map to MCP tools, Tomes to MCP resources
- **A2A (Agent-to-Agent)** - Spells map to A2A skills, supports agent collaboration
- **AG-UI (Agent-UI)** - Supports real-time UI updates via event streaming

## Development

### Adding New Schemas

1. Create the schema file in `src/definitions/` following the naming convention
2. Export it from `src/definitions/index.ts`
3. Add tests in `src/schemas/index.test.ts`
4. Run validation tests: `npm test`

### Schema Guidelines

- Use JSON Schema Draft 7
- Include `$schema` and `$id` properties
- Provide clear descriptions for all fields
- Use appropriate format validators (uuid, date-time, uri, etc.)
- Define required fields explicitly
- Use semantic versioning in schema IDs

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

## License

Part of the Arcan platform - see root LICENSE file.
