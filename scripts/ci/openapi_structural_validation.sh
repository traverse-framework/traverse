#!/usr/bin/env bash

set -euo pipefail

openapi_file="specs/033-http-json-api/openapi.yaml"

test -s "$openapi_file"

grep -q '^openapi: 3\.1\.0$' "$openapi_file"
grep -q '^paths:$' "$openapi_file"
grep -q '^components:$' "$openapi_file"
grep -q 'application/problem+json' "$openapi_file"
grep -q 'traverse_code' "$openapi_file"

required_paths=(
  '/healthz:'
  '/v1/workspaces/{workspace_id}/execute:'
  '/v1/workspaces/{workspace_id}/executions/{execution_id}:'
  '/v1/workspaces/{workspace_id}/traces/{execution_id}:'
  '/v1/workspaces/{workspace_id}/apps/{app_id}/events:'
  '/v1/workspaces/{workspace_id}/capabilities:'
  '/v1/workspaces/{workspace_id}/event-contracts:'
  '/v1/workspaces/{workspace_id}/workflows:'
  '/v1/workspaces/{workspace_id}/bundles:'
)

for path in "${required_paths[@]}"; do
  grep -q "  ${path}" "$openapi_file"
done

required_schemas=(
  'HealthEnvelope:'
  'ExecuteRequest:'
  'ExecutionEnvelope:'
  'AsyncAcceptedEnvelope:'
  'ExecutionStatusEnvelope:'
  'PublicTraceEnvelope:'
  'RegistrationRequest:'
  'BundleRegistrationRequest:'
  'RegistrationOutcome:'
  'ProblemDetails:'
)

for schema in "${required_schemas[@]}"; do
  grep -q "    ${schema}" "$openapi_file"
done

echo "OpenAPI structural validation passed."
