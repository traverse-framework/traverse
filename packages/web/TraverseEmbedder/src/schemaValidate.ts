/**
 * Host-side JSON Schema subset used for capability `inputs.schema` /
 * `outputs.schema` (issue #1658). Mirrors
 * `traverse_runtime::validate_value_against_schema`: `type`, `required`,
 * `properties`, `additionalProperties` (boolean), `items`, `maxLength`.
 * Violation paths are JSON Pointers (`/field`).
 */

import type { JsonValue } from "./types.js";

export interface SchemaViolation {
  readonly path: string;
  readonly message: string;
  readonly keyword?: string;
}

function asRecord(value: JsonValue | undefined): Record<string, JsonValue> | null {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  return value as Record<string, JsonValue>;
}

function pointer(path: string, segment: string): string {
  return `${path}/${segment}`;
}

function displayPointer(path: string): string {
  return path === "" ? "/" : path;
}

function typeError(path: string, expected: string): SchemaViolation {
  return { path: displayPointer(path), message: `expected ${expected}` };
}

export function validateValueAgainstSchema(
  value: JsonValue,
  schema: JsonValue,
  path = "",
  errors: SchemaViolation[] = [],
): SchemaViolation[] {
  const schemaObject = asRecord(schema);
  if (schemaObject === null) {
    errors.push({ path: displayPointer(path), message: "schema must be an object" });
    return errors;
  }

  const schemaType = schemaObject["type"];
  if (typeof schemaType === "string") {
    switch (schemaType) {
      case "object": {
        const instance = asRecord(value);
        if (instance === null) {
          errors.push(typeError(path, "object"));
          break;
        }
        validateRequired(instance, schemaObject, path, errors);
        validateProperties(instance, schemaObject, path, errors);
        break;
      }
      case "array": {
        if (!Array.isArray(value)) {
          errors.push(typeError(path, "array"));
          break;
        }
        const itemSchema = schemaObject["items"];
        if (itemSchema !== undefined) {
          value.forEach((item, index) => {
            validateValueAgainstSchema(item, itemSchema, pointer(path, String(index)), errors);
          });
        }
        break;
      }
      case "string": {
        if (typeof value !== "string") {
          errors.push(typeError(path, "string"));
          break;
        }
        const maxLength = schemaObject["maxLength"];
        if (typeof maxLength === "number" && Number.isInteger(maxLength) && maxLength >= 0) {
          if ([...value].length > maxLength) {
            errors.push({
              path: displayPointer(path),
              keyword: "maxLength",
              message: `string longer than maxLength ${maxLength}`,
            });
          }
        }
        break;
      }
      case "integer":
        if (typeof value !== "number" || !Number.isInteger(value)) {
          errors.push(typeError(path, "integer"));
        }
        break;
      case "number":
        if (typeof value !== "number") {
          errors.push(typeError(path, "number"));
        }
        break;
      case "boolean":
        if (typeof value !== "boolean") {
          errors.push(typeError(path, "boolean"));
        }
        break;
      case "null":
        if (value !== null) {
          errors.push(typeError(path, "null"));
        }
        break;
      default:
        break;
    }
  }
  return errors;
}

function validateRequired(
  instance: Record<string, JsonValue>,
  schemaObject: Record<string, JsonValue>,
  path: string,
  errors: SchemaViolation[],
): void {
  const required = schemaObject["required"];
  if (!Array.isArray(required)) {
    return;
  }
  for (const field of required) {
    if (typeof field !== "string") {
      continue;
    }
    if (!(field in instance)) {
      errors.push({
        path: pointer(path, field),
        keyword: "required",
        message: "required property is missing",
      });
    }
  }
}

function validateProperties(
  instance: Record<string, JsonValue>,
  schemaObject: Record<string, JsonValue>,
  path: string,
  errors: SchemaViolation[],
): void {
  const properties = asRecord(schemaObject["properties"]);
  const additional = schemaObject["additionalProperties"];
  const additionalAllowed = typeof additional === "boolean" ? additional : true;

  for (const [key, value] of Object.entries(instance)) {
    const propertySchema = properties?.[key];
    if (propertySchema !== undefined) {
      validateValueAgainstSchema(value, propertySchema, pointer(path, key), errors);
    } else if (!additionalAllowed) {
      errors.push({
        path: pointer(path, key),
        keyword: "additionalProperties",
        message: "additional property is not allowed",
      });
    }
  }
}

/** Validate `payload` against a contract `inputs.schema` / `outputs.schema`. */
export function validatePayloadAgainstContract(
  payload: JsonValue,
  schema: JsonValue,
): SchemaViolation[] {
  return validateValueAgainstSchema(payload, schema, "");
}
