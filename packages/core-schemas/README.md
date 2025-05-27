# @repo/core-schemas

This package contains JSON Schema definitions for the Arcan platform's core entities.

## Structure

- `/schemas` - JSON Schema definition files
- `/src` - TypeScript source for schema validation and utilities
- `/dist` - Compiled JavaScript output

## Usage

```typescript
import { schemas } from '@repo/core-schemas';
```

## Development

```bash
# Build the package
bun run build

# Watch mode
bun run dev

# Type checking
bun run check-types
```
