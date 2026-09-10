# Bundled OpenAPI JSON Schemas

Vendored so key/enum completion and validation work offline. All three are
published by the OpenAPI Initiative under Apache-2.0.

| File | Source | Fetched |
| --- | --- | --- |
| `openapi-2.0.json` | https://raw.githubusercontent.com/OAI/OpenAPI-Specification/main/_archive_/schemas/v2.0/schema.json | 2026-09-10 |
| `openapi-3.0.json` | https://spec.openapis.org/oas/3.0/schema/2021-09-28 | 2026-09-10 |
| `openapi-3.1.json` | https://spec.openapis.org/oas/3.1/schema/2022-10-07 | 2026-09-10 |

`json-schema-2020-12/` holds the JSON Schema 2020-12 dialect and its seven
vocabularies (from https://json-schema.org/draft/2020-12/, fetched 2026-09-10).
`openapi-3.1.json` hands Schema Objects to the dialect with
`$dynamicRef: "#meta"`, which `openapi_core::jsonschema` resolves to the
bundled dialect; without them a 3.1 Schema Object would describe no keys.

`unevaluatedProperties`, `if`/`then` and `dependentSchemas` are still ignored,
so 3.1 documents get fewer "property not allowed" diagnostics than 3.0 ones.
