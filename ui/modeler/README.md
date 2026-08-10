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

## BPMN renderer foundation

The current M1 renderer is a first-party React/SVG implementation:

- `src/modeler/modelerStore.ts` owns the versioned document, selection, pan, and zoom state through
  Zustand + Immer.
- `src/modeler/diagramModel.ts` traverses every process and nested subprocess without inventing a
  second frontend model.
- `src/modeler/BpmnCanvas.tsx` renders pools, lanes, sequence/message/association flows, data
  stores, and BPMN DI transforms in separate SVG layers.
- `src/modeler/BpmnElement.tsx` renders the task, event, gateway, subprocess, call-activity, and
  data-object families from their generated discriminated unions.

The renderer covers the canonical task families, event definitions, all five gateway kinds,
embedded/event/adhoc subprocesses, transactions, call activities, pools and lanes, data objects and
stores, text annotations, groups, sequence/message flows, and directed associations. A complex
gateway remains a first-class protocol and rendering type; deployment validation rejects it with
`flowable-complex-gateway-not-supported` until the Rust engine implements its execution semantics.

`POST /modeler-app/rest/editor/layout` preserves existing BPMN DI and deterministically fills only
missing shape bounds and edge waypoints, including nested subprocess elements and artifacts. The
canvas Fit action derives its viewport from every rendered shape and waypoint without changing
canonical model coordinates.

The C1 screenshot gate is generated from the same 20 representative XML round-trip fixtures used by
the Rust converter tests. `npm run generate:render-fixtures` refreshes the ignored browser JSON
inputs; `npm run test:e2e` regenerates them, builds the E2E-only fixture harness, and compares all 20
Windows Chromium images strictly. To intentionally accept a reviewed renderer change, run:

```powershell
npx playwright test e2e/render-fixtures.spec.ts --update-snapshots
npm run test:e2e
```

Production builds do not expose the fixture harness.

The M2 command boundary is now active for node movement. Document mutations are captured as Immer
forward/inverse patches in `src/modeler/commands.ts`; one drag creates one history entry, adjusts
attached boundary-event geometry and connected DI endpoints, and supports button or keyboard
undo/redo. The property panel remains read-only until M3. Node selection, wheel/button zoom,
drag-to-pan, node dragging, and the typed property summary are covered by Chromium acceptance
tests.

Palette clicks now create canonical start-event, user-task, exclusive-gateway, subprocess, and data
object variants with synchronized list/map/DI state. Delete/Backspace and the toolbar delete action
remove the selected node together with attached boundary events, connected flows, lane references,
and DI metadata in one reversible command.

## Architecture boundaries

- The frontend never parses or emits BPMN or DMN XML.
- Generated protocol types live in `src/generated/` and must not be hand-edited.
- Editor state is the single source of truth. Future mutations enter the store as commands so undo
  and redo remain deterministic.
- Server validation is authoritative; client validation exists for immediate editing feedback.
- BPMN rendering and interaction use React and native SVG. Third-party graph-editing kernels are
  prohibited.

## Dependency allowlist

Production dependencies are deliberately narrow. Adding a package requires documenting its purpose
here before changing `package.json`.

| Package family                         | Purpose                        | Status           |
| -------------------------------------- | ------------------------------ | ---------------- |
| `react`, `react-dom`                   | UI runtime                     | Allowed          |
| `react-router-dom`                     | `/modeler-app` route ownership | Allowed          |
| `zustand`                              | Editor state store             | Allowed          |
| `immer`                                | Immutable command application  | Allowed          |
| `dayjs`                                | Date display and editing       | Allowed          |
| `vite`, `typescript`                   | Build and strict type checking | Development only |
| `vitest`                               | Unit and protocol tests        | Development only |
| `@playwright/test`                     | Browser acceptance tests       | Development only |
| `json-schema-to-typescript`            | Rust schema to TypeScript      | Development only |
| `eslint` and official/plugin ecosystem | Static analysis                | Development only |
| `prettier`                             | Deterministic formatting       | Development only |

Explicitly prohibited editor kernels include Oryx, bpmn-js, dmn-js, mxGraph, JointJS, GoJS, and
similar graph/canvas frameworks. Utility packages are not implicitly allowed by this list.
