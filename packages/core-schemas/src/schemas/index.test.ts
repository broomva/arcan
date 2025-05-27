import { describe, it, expect } from 'vitest';
import { validateSchema } from './validator';
import { SchemaRegistry } from './registry';
import * as schemas from '../definitions';

describe('Schema Validation', () => {
  describe('ArcanAgentBase Schema', () => {
    it('should validate a valid agent', () => {
      const validAgent = {
        agentId: '123e4567-e89b-12d3-a456-426614174000',
        version: '1.0.0',
        name: 'TestAgent',
        owner: 'did:example:123',
        createdAt: '2024-01-01T00:00:00Z',
        updatedAt: '2024-01-01T00:00:00Z',
        status: 'active',
      };

      const result = validateSchema(schemas.ArcanAgentBase, validAgent);
      expect(result.valid).toBe(true);
    });

    it('should reject invalid agent', () => {
      const invalidAgent = {
        // Missing required fields
        name: 'TestAgent',
      };

      const result = validateSchema(schemas.ArcanAgentBase, invalidAgent);
      expect(result.valid).toBe(false);
      expect(result.errors).toBeDefined();
      expect(result.errors?.length).toBeGreaterThan(0);
    });
  });

  describe('ArcanSpell Schema', () => {
    it('should validate a valid spell', () => {
      const validSpell = {
        spellId: '123e4567-e89b-12d3-a456-426614174000',
        name: 'calculateSum',
        description: 'Calculates the sum of two numbers provided as input',
        inputSchema: {
          type: 'object',
          properties: {
            a: { type: 'number' },
            b: { type: 'number' },
          },
          required: ['a', 'b'],
        },
        outputSchema: {
          type: 'object',
          properties: {
            result: { type: 'number' },
          },
          required: ['result'],
        },
        invocationMechanism: {
          type: 'internalFunction',
        },
        version: '1.0.0',
      };

      const result = validateSchema(schemas.ArcanSpell, validSpell);
      expect(result.valid).toBe(true);
    });
  });

  describe('ArcanEvent Schema', () => {
    it('should validate a valid event', () => {
      const validEvent = {
        eventId: '123e4567-e89b-12d3-a456-426614174000',
        eventType: 'agent.created',
        timestamp: '2024-01-01T00:00:00Z',
        source: {
          service: 'agent-service',
          instanceId: 'instance-1',
        },
        schemaVersion: '1.0.0',
        payload: {
          agentId: '123e4567-e89b-12d3-a456-426614174000',
        },
      };

      const result = validateSchema(schemas.ArcanEvent, validEvent);
      expect(result.valid).toBe(true);
    });
  });
});

describe('Schema Registry', () => {
  it('should register and retrieve schemas', () => {
    const registry = new SchemaRegistry();
    
    registry.registerSchema(schemas.ArcanAgentBase);
    
    const retrieved = registry.getSchema('https://arcan.ai/schemas/ArcanAgentBase.v1.json');
    expect(retrieved).toBeDefined();
    expect(retrieved?.metadata.title).toBe('ArcanAgentBase');
  });

  it('should validate data using registered schemas', () => {
    const registry = new SchemaRegistry();
    registry.registerSchema(schemas.ArcanAgentBase);

    const validAgent = {
      agentId: '123e4567-e89b-12d3-a456-426614174000',
      version: '1.0.0',
      name: 'TestAgent',
      owner: 'did:example:123',
      createdAt: '2024-01-01T00:00:00Z',
      updatedAt: '2024-01-01T00:00:00Z',
      status: 'active',
    };

    const result = registry.validate('https://arcan.ai/schemas/ArcanAgentBase.v1.json', validAgent);
    expect(result.valid).toBe(true);
  });
}); 