# Modeler C1 completion design

Date: 2026-08-10. Scope: finish the C1 BPMN rendering kernel before adding more C2 interactions.

## Decision

Keep the existing canonical pipeline: BPMN XML is parsed into `flowable-bpmn-model`, serialized as
the editor JSON projection, and rendered directly by the React/SVG workspace. No renderer-only
semantic model is introduced. The renderer continues to dispatch on the generated
`FlowElementEnum` and `ArtifactEnum` discriminants, while BPMN DI maps remain the sole geometry
source.

Three approaches were considered. A renderer-side compatibility object would be quick but would
create a second semantic model and break round-trip guarantees. Treating unsupported XML as generic
SVG annotations would preserve pixels but lose editor semantics. Extending the Rust canonical model
and converter makes the missing shapes typed, writable, testable, and available to every later C2/C3
operation, so it is the selected approach.

## Protocol and conversion

Add canonical support for BPMN's fifth gateway (`complexGateway`) and the two missing artifact
families (`textAnnotation` and `group`). Preserve `activationCondition`, annotation text/text format,
group category reference, and association direction. Parser and writer support both process and
nested subprocess artifacts. `artifacts` and `artifactMap` become the serialized source of truth;
the legacy skipped `associations` collection remains only as a compatibility mirror and must not
cause duplicate XML output.

Schema export regenerates the TypeScript discriminated unions. Exhaustive renderer predicates then
make unsupported protocol additions visible at compile time instead of silently falling through.

## SVG rendering

Extend the existing gateway family with the complex-gateway asterisk marker. Render text
annotations as open brackets with wrapped text, and groups as non-interactive dashed rounded
rectangles behind ordinary nodes. Associations retain their direction metadata in the protocol and
use the appropriate end marker when directed. Existing tasks, events, pools, lanes, data objects,
data stores, sequence flows, message flows, labels, pan, zoom, and selection remain unchanged.

The renderer reads only canonical JSON plus DI. Missing DI continues to be handled by the existing
layout endpoint, not by inventing persistent browser geometry.

## Verification

Add a focused XML → model → JSON → model → XML contract containing complex gateway, text
annotation, group, directed association, and nested subprocess artifacts. Reparse the emitted XML
and assert semantic fields and artifact maps. Regenerate schema/types and add Vitest coverage for
each new SVG family.

For the C1 hard gate, generate browser-consumable documents from all 20 representative round-trip
fixtures. Playwright renders each document and captures a stable screenshot; three representative
models (events/boundary, pools/lanes, subprocess/call activity) receive explicit visual review.
Build, lint, formatting, unit tests, converter regressions, and Playwright must all pass before C1 is
declared complete.
