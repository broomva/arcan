import Ajv from 'ajv';
import addFormats from 'ajv-formats';

// Initialize AJV with JSON Schema draft-07 support
const ajv = new Ajv({
  strict: true,
  allErrors: true,
  verbose: true,
  validateFormats: true,
});

// Add format validators (uuid, date-time, uri, etc.)
addFormats(ajv);

/**
 * Validates data against a JSON Schema
 * @param schema - The JSON Schema to validate against
 * @param data - The data to validate
 * @returns Validation result with errors if any
 */
export function validateSchema(
  schema: any, // Using any to handle imported JSON schemas
  data: unknown
): {
  valid: boolean;
  errors?: Array<{
    message: string;
    path: string;
    keyword: string;
    params: Record<string, unknown>;
  }>;
} {
  const validate = ajv.compile(schema);
  const valid = validate(data);

  if (!valid && validate.errors) {
    return {
      valid: false,
      errors: validate.errors.map((error) => ({
        message: error.message || 'Validation error',
        path: error.instancePath,
        keyword: error.keyword,
        params: error.params,
      })),
    };
  }

  return { valid: true };
}

/**
 * Creates a validator function for a specific schema
 * @param schema - The JSON Schema to create a validator for
 * @returns A validator function
 */
export function createValidator(schema: any) {
  const validate = ajv.compile(schema);
  
  return (data: unknown): boolean => {
    return validate(data);
  };
}

/**
 * Gets validation errors from the last validation
 * @returns Array of validation errors or null
 */
export function getValidationErrors() {
  return ajv.errors;
} 