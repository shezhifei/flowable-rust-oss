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
npm run build
npm run test:e2e
```

The production base path is `/modeler-app/`. Vite development runs on port `5174`; preview and
Playwright use port `4174`.

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

The sample workbench is deliberately read-only until the M2 command stack is introduced. Node
selection, wheel/button zoom, drag-to-pan, and the typed property summary are live and covered by
Chromium acceptance tests.

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
