import { describe, it, expect } from 'vitest';
import { schemas } from './index';

describe('schemas', () => {
  it('should export schemas object', () => {
    expect(schemas).toBeDefined();
    expect(typeof schemas).toBe('object');
  });

  it('should be an empty object initially', () => {
    expect(Object.keys(schemas).length).toBe(0);
  });
}); 