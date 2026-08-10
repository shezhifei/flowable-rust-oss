# Modeler C2 interaction design

Date: 2026-08-10. Scope: complete the BPMN interaction editor on top of the canonical C0/C1
protocol and renderer.

## Decision

All persistent edits continue to target `BpmnEditorDocument` directly. A command owns one completed
user gesture and produces Immer forward/inverse patches; undo and redo apply those patches without
replaying UI events. Selection, active tool, marquee rectangle, connection preview, and drag preview
are transient editor state and do not enter document history.

Three approaches were considered. Direct component mutations would be small initially but would
split invariants across pointer handlers and make undo unreliable. A normalized browser graph would
simplify lookups but would introduce the second semantic model prohibited by C0. Canonical commands
plus shared container/location helpers keep the protocol authoritative while centralizing the
duplicated BPMN list/map and DI maintenance, so this is the selected approach.

## Canonical mutation invariants

Every create, delete, paste, replace, connect, move, resize, and bendpoint operation updates all
representations that the Rust writer and renderer consume:

- the owning process or subprocess `flowElements` list and `flowElementMap`;
- source `outgoingFlows` and target `incomingFlows` for sequence flows;
- `locationMap`, `labelLocationMap`, `flowLocationMap`, and `edgeMap`;
- lane `flowReferences` when an element enters or leaves a lane;
- boundary-event attachment and host `boundaryEvents` mirrors;
- connected flow endpoints and descendant geometry when a container moves.

A recursive container locator resolves the semantic owner of every flow element. Moving across a
subprocess boundary transfers the canonical element and any internal connected flows only when the
new ownership is legal. Pool/lane ownership is expressed through process and lane references rather
than a renderer-only parent field.

## Selection and transformation

The store exposes ordered multi-selection with one primary element. Pointer click replaces the
selection; Ctrl/Cmd-click toggles membership; dragging empty canvas in pointer mode performs a
marquee selection; hand mode and middle-button/space dragging pan. Moving any selected node moves
the whole selected set on the 10-unit grid in one command. Alignment guides show candidate left,
center, right, top, middle, and bottom matches during the preview.

Resize handles are available only for pools, lanes, subprocess variants, and groups. Resize changes
DI bounds with minimum sizes and grid snapping; it does not scale child coordinates. Moving a
resizable container translates its descendants so their relative positions remain stable. Boundary
events snap to the nearest legal activity border and store `attachedToRefId`; ordinary nodes cannot
be placed outside their resolved process/subprocess or lane owner.

## Creation, connection, and structure

Palette items support click creation and native drag-to-place. The palette covers the initial C2
families needed for acceptance, including start/end events, user tasks, exclusive gateways,
subprocesses, boundary timer events, and data objects. Defaults come from typed factories, not object
casts.

Connect mode and visible node anchors create sequence flows only for legal pairs in the same
semantic container. Starts reject incoming connections; ends and terminate ends reject outgoing
connections; boundary events reject incoming connections; data/artifact/pool/lane shapes are not
sequence-flow endpoints. The first router is deterministic Manhattan routing and may cross unrelated
nodes as permitted by the plan. Interior bendpoints can be added, moved, and removed through
commands.

Delete cascades to descendants, attached boundary events, connected sequence/message flows, lane
references, and DI. Copy/paste duplicates selected nodes plus flows wholly inside the selection,
allocates collision-free ids, remaps references, and offsets DI by 24 units. Task replacement uses a
typed factory and preserves base flow-node/activity properties, connected flow mirrors, listeners,
loop characteristics, documentation, and extension data while replacing only subtype-specific
fields. Flow insertion and subprocess collapse remain the two explicitly optional C2 items and are
not required for the first complete acceptance path.

## Persistence and verification

When the route supplies a model id, the workspace loads and saves through
`/modeler-app/rest/models/{id}/editor/bpmn-json`; the same client is mockable by Playwright. Save
uses the canonical document body and replaces local state with the validated server response, so a
reload proves transport fidelity instead of merely preserving component state.

Unit tests cover each command's forward state, mirrored invariants, undo, and redo. Browser tests
draw the leave-request process (start, user task, exclusive gateway, two conditional branches, and
ends), exercise multi-selection, snap, resize, boundary attachment, bendpoints, copy/paste, task
replacement, and at least 50 undo/redo operations, then save and reload through a mocked HTTP
boundary. Existing 20-model strict renderer screenshots remain unchanged and must pass alongside
build, lint, formatting, and unit tests.
