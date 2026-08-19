# Flowable Modeler UI

First-party modeling UI for BPMN 2.0, DMN decision tables, and Flowable forms. The browser edits
typed JSON projections; Rust owns XML conversion, validation, layout, thumbnails, and persistence.

## Commands

```powershell
npm install
npm run dev
npm run lint
npm test
npm run generate:types
npm run generate:render-fixtures
npm run build
npm run test:e2e
```

The production base path is `/modeler-app/`. Vite development runs on port `5174`; preview and
Playwright use port `4174`.

The Rust UI server discovers the production bundle at `ui/modeler/dist` relative to the workspace.
Set `FLOWABLE_MODELER_STATIC_DIR` to override that location in packaged deployments. If the
directory is absent, REST routes remain mounted and static Modeler routes are omitted rather than
falling back to source files.

## Surface map

| Route | Surface |
| ----- | ------- |
| `/modeler-app/` | Model repository list (create / delete / import / publish) |
| `/modeler-app/models/:id/bpmn` | BPMN process editor |
| `/modeler-app/models/:id/dmn` | DMN decision table editor |
| `/modeler-app/models/:id/form` | Form designer |

The reserved BPMN model id `sample` keeps the offline demo document and does not hit the REST
persistence endpoints.

## BPMN editor

- `src/modeler/modelerStore.ts` owns the versioned document, selection, pan, and zoom state through
  Zustand + Immer.
- `src/modeler/diagramModel.ts` traverses every process and nested subprocess without inventing a
  second frontend model.
- `src/modeler/BpmnCanvas.tsx` / `BpmnElement.tsx` render pools, lanes, flows, data, and the full
  element family from the generated protocol unions.
- Interaction commands cover move, create, delete, connect, clipboard, transform, ownership, and
  replacement.
- Properties panel (C3): General / Execution / Assignment / Form & scheduling / Implementation /
  Condition, plus phase-two groups for multi-instance, task/execution listeners, signal & message
  definitions and refs, field injection, call-activity parameters, timer definitions, error and
  escalation references, and user-task / start-event form properties. Business rule tasks edit their
  DMN decision key beside the implementation fields.

The C1 screenshot gate is generated from the same 20 representative XML round-trip fixtures used by
the Rust converter tests. `npm run generate:render-fixtures` refreshes the ignored browser JSON
inputs; `npm run test:e2e` regenerates them, builds the E2E-only fixture harness, and compares all 20
Windows Chromium images strictly. To intentionally accept a reviewed renderer change, run:

```powershell
npx playwright test e2e/render-fixtures.spec.ts --update-snapshots
npm run test:e2e
```

Production builds do not expose the fixture harness.

## DMN decision table editor

`src/dmn/` owns the decision-table document store, FEEL subset validation, hit-policy editing, and
the undoable grid UI. Persistence uses `/modeler-app/rest/models/:id/editor/dmn-json`.

## Form designer

`src/form/` owns the form document store, the Flowable 6.8 wire-type palette (19 types; boolean is
the checkbox), recursive containers, outcomes, preview mode, and client-side validation that mirrors
the Rust form boundary codes. Persistence uses
`/modeler-app/rest/form-models/:id/editor/form-json` (PUT then GET).

## Model repository

`src/models/` lists `/repository/models`, creates stubs for BPMN/DMN/form, deletes, imports source
files, and publishes through `/repository/deployments` (BPMN multipart) or the DMN/form repository
deployment endpoints. BPMN rows request `/modeler-app/rest/models/:id/thumbnail`.

## Architecture boundaries

- The frontend never parses or emits BPMN or DMN XML.
- Generated protocol types live in `src/generated/` and must not be hand-edited.
- Editor state is the single source of truth. Mutations enter the store as commands so undo and redo
  remain deterministic.
- Server validation is authoritative; client validation exists for immediate editing feedback.
- BPMN rendering and interaction use React and native SVG. Third-party graph-editing kernels are
  prohibited.

## Dependency allowlist

Production dependencies are deliberately narrow. Adding a package requires documenting its purpose
here before changing `package.json`. This table is reconciled with `package.json` dependencies and
devDependencies (versions are pinned there).

| Package family                         | Purpose                        | Status           | package.json |
| -------------------------------------- | ------------------------------ | ---------------- | ------------ |
| `react`, `react-dom`                   | UI runtime                     | Allowed          | 18.3.x       |
| `react-router-dom`                     | `/modeler-app` route ownership | Allowed          | 7.x          |
| `zustand`                              | Editor state store             | Allowed          | 5.x          |
| `immer`                                | Immutable command application  | Allowed          | 11.x         |
| `dayjs`                                | Date display and editing       | Allowed          | 1.11.x       |
| `vite`, `@vitejs/plugin-react`         | Build                          | Development only | 8.x / 6.x    |
| `typescript`                           | Strict type checking           | Development only | 5.9.x        |
| `vitest`                               | Unit and protocol tests        | Development only | 4.x          |
| `@playwright/test`                     | Browser acceptance tests       | Development only | 1.62.x       |
| `json-schema-to-typescript`            | Rust schema to TypeScript      | Development only | 15.x         |
| `eslint` and official/plugin ecosystem | Static analysis                | Development only | 9.x          |
| `prettier`                             | Deterministic formatting       | Development only | 3.x          |
| `@types/*`                             | Type definitions               | Development only | pinned       |

Explicitly prohibited editor kernels include Oryx, bpmn-js, dmn-js, mxGraph, JointJS, GoJS, and
similar graph/canvas frameworks. Utility packages are not implicitly allowed by this list.
