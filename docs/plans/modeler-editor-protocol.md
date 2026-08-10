# Modeler editor JSON protocol

The modeler frontend never parses or produces BPMN or DMN XML. It exchanges versioned JSON
documents with Rust, and the nested model is the same canonical model used by the converters and
runtime services.

The generated JSON Schema is
[`modeler-editor-protocol.schema.json`](./modeler-editor-protocol.schema.json). Browser types are
generated from it at `ui/modeler/src/generated/editor-protocol.ts`.

## Document envelopes

Every editor endpoint returns and accepts one concrete envelope:

```json
{
  "schemaVersion": "1.0",
  "model": {}
}
```

The schema defines `BpmnEditorDocument`, `DmnEditorDocument`, and `FormEditorDocument`. Version
`1.0` is intentionally an enum value rather than a free-form string: a server must reject an editor
document it cannot interpret instead of silently dropping fields.

## BPMN projection

`BpmnEditorDocument.model` is `flowable_bpmn_model::BpmnModel`. Polymorphic values use explicit,
stable discriminators so JSON round-trips preserve the exact Rust enum variant:

- `FlowElementEnum`: `elementType` (`userTask`, `serviceTask`, `startEvent`, and so on)
- `EventDefinitionEnum`: `eventDefinitionType`
- `SubProcessEnum`: `subProcessType`
- `ArtifactEnum`: `artifactType`

The JSON remains a faithful projection of the canonical model, including its `mainProcess`,
`flowElementMap`, and `artifactMap` lookup views. The editor treats `processes[].flowElements` as the
author-owned source of truth; the backend rebuilds derived views before persistence. A lane's
recursive `parentProcess` link remains omitted.

DI data remains first-class: shape bounds are in `locationMap`, label bounds in
`labelLocationMap`, connection waypoints in `flowLocationMap`, and edge/docker metadata in
`edgeMap`.

## DMN projection

`DmnEditorDocument.model` is `flowable_dmn_model::DmnDefinition`. Hit policies and collect
operators use their canonical uppercase wire values. The editor must only produce FEEL syntax
supported by `flowable-dmn-engine`; the schema describes structure, not expression semantics.

## Form projection

`FormEditorDocument.model` is `flowable_form_service::FormModel`, the author-owned form definition
without deployment/runtime metadata. Fields use `fieldType` with the values `Container`,
`OptionFormField`, `ExpressionFormField`, and `BaseField`. The ordinary field control kind (for
example `text`, `date`, or `dropdown`) remains the nested field's `type` property.

## HTTP boundary

`flowable-ui-rest::modeler` exposes the protocol through the cookie-authenticated UI surface. Every
handler also extracts `UiAuth`, so mounting the route module without A's middleware fails closed.

| Method | Path | Request / response |
| --- | --- | --- |
| `GET`, `PUT` | `/modeler-app/rest/models/:id/editor/bpmn-json` | BPMN editor document; PUT persists validated BPMN XML |
| `GET`, `PUT` | `/modeler-app/rest/models/:id/editor/dmn-json` | DMN editor document; PUT persists validated DMN XML |
| `GET`, `PUT` | `/modeler-app/rest/form-models/:id/editor/form-json` | Form editor document; PUT rejects invalid form semantics before persistence |
| `POST` | `/modeler-app/rest/models/:id/validate` | Validates the stored BPMN, DMN, or Form source and returns `ValidationResult` |
| `GET` | `/modeler-app/rest/models/:id/thumbnail` | Returns a generated `image/png`, laying out BPMN without DI first |
| `POST` | `/modeler-app/rest/editor/layout` | Accepts and returns a BPMN editor document with generated DI |

Repository not-found/conflict/permission errors are translated to A's UI error envelope. Invalid
client documents are `400`; corrupt persisted source is logged and returned as a non-leaking `500`.
When `ui/modeler/dist` exists—or `FLOWABLE_MODELER_STATIC_DIR` points to a distribution—the same
module serves `/modeler-app/` and returns `index.html` for SPA deep links. Exact REST routes take
precedence over the static fallback.

## Generation and change discipline

Run from `ui/modeler`:

```powershell
npm run generate:types
```

This command runs the Rust schema exporter and then `json-schema-to-typescript`. Do not hand-edit
the schema or generated TypeScript. A protocol change requires, in the same commit:

1. changing the canonical Rust model or versioned envelope;
2. updating Rust JSON round-trip tests;
3. regenerating the schema and TypeScript;
4. updating endpoint contract tests and this document when wire semantics change.
