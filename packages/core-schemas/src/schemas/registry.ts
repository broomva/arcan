import { validateSchema } from './validator';

interface SchemaMetadata {
  id: string;
  version: string;
  title: string;
  description?: string;
  createdAt: Date;
  updatedAt: Date;
}

interface RegisteredSchema {
  schema: any; // Using any to handle imported JSON schemas
  metadata: SchemaMetadata;
}

/**
 * Schema Registry for managing and versioning JSON Schemas
 */
export class SchemaRegistry {
  private schemas: Map<string, RegisteredSchema> = new Map();
  private schemasByVersion: Map<string, RegisteredSchema> = new Map();

  /**
   * Registers a new schema in the registry
   * @param schema - The JSON Schema to register
   * @param metadata - Additional metadata for the schema
   */
  registerSchema(schema: any, metadata?: Partial<SchemaMetadata>): void {
    if (!schema.$id) {
      throw new Error('Schema must have an $id property');
    }

    const id = schema.$id;
    const version = this.extractVersion(id) || '1.0.0';
    const title = schema.title || 'Untitled Schema';

    const fullMetadata: SchemaMetadata = {
      id,
      version,
      title,
      description: schema.description,
      createdAt: new Date(),
      updatedAt: new Date(),
      ...metadata,
    };

    const registeredSchema: RegisteredSchema = {
      schema,
      metadata: fullMetadata,
    };

    this.schemas.set(id, registeredSchema);
    this.schemasByVersion.set(`${title}:${version}`, registeredSchema);
  }

  /**
   * Gets a schema by its ID
   * @param id - The schema ID
   * @returns The registered schema or undefined
   */
  getSchema(id: string): RegisteredSchema | undefined {
    return this.schemas.get(id);
  }

  /**
   * Gets a schema by title and version
   * @param title - The schema title
   * @param version - The schema version
   * @returns The registered schema or undefined
   */
  getSchemaByVersion(title: string, version: string): RegisteredSchema | undefined {
    return this.schemasByVersion.get(`${title}:${version}`);
  }

  /**
   * Lists all registered schemas
   * @returns Array of registered schemas
   */
  listSchemas(): RegisteredSchema[] {
    return Array.from(this.schemas.values());
  }

  /**
   * Validates data against a registered schema
   * @param schemaId - The ID of the schema to validate against
   * @param data - The data to validate
   * @returns Validation result
   */
  validate(schemaId: string, data: unknown) {
    const registeredSchema = this.schemas.get(schemaId);
    if (!registeredSchema) {
      throw new Error(`Schema with ID ${schemaId} not found`);
    }

    return validateSchema(registeredSchema.schema, data);
  }

  /**
   * Checks if a schema is registered
   * @param id - The schema ID
   * @returns True if the schema is registered
   */
  hasSchema(id: string): boolean {
    return this.schemas.has(id);
  }

  /**
   * Removes a schema from the registry
   * @param id - The schema ID to remove
   */
  removeSchema(id: string): void {
    const schema = this.schemas.get(id);
    if (schema) {
      const key = `${schema.metadata.title}:${schema.metadata.version}`;
      this.schemasByVersion.delete(key);
      this.schemas.delete(id);
    }
  }

  /**
   * Extracts version from schema ID
   * @param id - The schema ID
   * @returns The version string or null
   */
  private extractVersion(id: string): string | null {
    const match = id.match(/\.v(\d+)\.json$/);
    if (match) {
      return `${match[1]}.0.0`;
    }
    return null;
  }

  /**
   * Gets all versions of a schema by title
   * @param title - The schema title
   * @returns Array of versions
   */
  getSchemaVersions(title: string): string[] {
    const versions: string[] = [];
    for (const [key, schema] of this.schemasByVersion) {
      if (key.startsWith(`${title}:`)) {
        versions.push(schema.metadata.version);
      }
    }
    return versions.sort();
  }
}

// Create a singleton instance
export const defaultRegistry = new SchemaRegistry(); 