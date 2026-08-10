import type { Draft } from 'immer';

import type {
  ArtifactEnum,
  BpmnDiEdge,
  BpmnEditorDocument,
  FlowElementEnum,
  GraphicInfo,
  Process,
  ValuedDataObject,
} from '../generated/editor-protocol';
import type { ModelerCommand } from './commands';
import {
  normalizeModelInvariants,
  type CanonicalOwner as DraftCanonicalOwner,
} from './modelInvariants';

type NestedOwner = Extract<
  FlowElementEnum,
  {
    elementType: 'adhocSubProcess' | 'eventSubProcess' | 'subProcess' | 'transaction';
  }
>;
type CanonicalOwner = Process | NestedOwner;

export interface BpmnClipboardSlice {
  ownerId: string | null;
  elements: FlowElementEnum[];
  dataObjects: ValuedDataObject[];
  artifacts: ArtifactEnum[];
  locationMap: Record<string, GraphicInfo>;
  labelLocationMap: Record<string, GraphicInfo>;
  flowLocationMap: Record<string, GraphicInfo[]>;
  edgeMap: Record<string, BpmnDiEdge>;
}

export function copySelection(
  document: BpmnEditorDocument,
  selectedIds: readonly string[],
): BpmnClipboardSlice | null {
  const selection = new Set(selectedIds);
  const located = [...selection].map((id) => locateOwnedValue(document, id));
  if (!located.length || located.some((entry) => !entry)) return null;
  const owner = located[0]!.owner;
  if (located.some((entry) => entry!.owner !== owner)) return null;

  const roots = (owner.flowElements ?? []).filter(
    (element) =>
      element.elementType !== 'sequenceFlow' && Boolean(element.id && selection.has(element.id)),
  );
  const dataObjects = (owner.dataObjects ?? []).filter((value) =>
    Boolean(value.id && selection.has(value.id)),
  );
  const artifacts = (owner.artifacts ?? []).filter((value) =>
    Boolean(value.id && selection.has(value.id)),
  );
  const copiedEndpointIds = new Set<string>();
  for (const element of roots) collectFlowElementIds(element, copiedEndpointIds);
  for (const value of dataObjects) if (value.id) copiedEndpointIds.add(value.id);
  for (const artifact of artifacts) if (artifact.id) copiedEndpointIds.add(artifact.id);

  const internalFlows = (owner.flowElements ?? []).filter(
    (element): element is Extract<FlowElementEnum, { elementType: 'sequenceFlow' }> =>
      element.elementType === 'sequenceFlow' &&
      Boolean(
        element.sourceRef &&
        element.targetRef &&
        copiedEndpointIds.has(element.sourceRef) &&
        copiedEndpointIds.has(element.targetRef),
      ),
  );
  const internalAssociations = (owner.artifacts ?? []).filter(
    (artifact): artifact is Extract<ArtifactEnum, { artifactType: 'association' }> =>
      artifact.artifactType === 'association' &&
      Boolean(
        artifact.sourceRef &&
        artifact.targetRef &&
        copiedEndpointIds.has(artifact.sourceRef) &&
        copiedEndpointIds.has(artifact.targetRef),
      ),
  );

  const elements = structuredClone([...roots, ...internalFlows]);
  const copiedArtifacts = uniqueById(structuredClone([...artifacts, ...internalAssociations]));
  const copiedDataObjects = structuredClone(dataObjects);
  const copiedIds = new Set<string>();
  collectObjectIds(elements, copiedIds);
  collectObjectIds(copiedArtifacts, copiedIds);
  collectObjectIds(copiedDataObjects, copiedIds);

  return {
    ownerId: owner.id ?? null,
    elements,
    dataObjects: copiedDataObjects,
    artifacts: copiedArtifacts,
    locationMap: copyGraphicMap(document.model.locationMap, copiedIds),
    labelLocationMap: copyGraphicMap(document.model.labelLocationMap, copiedIds),
    flowLocationMap: copyRouteMap(document.model.flowLocationMap, copiedIds),
    edgeMap: copyEdgeMap(document.model.edgeMap, copiedIds),
  };
}

export function pasteClipboardCommand(clipboard: BpmnClipboardSlice, offset = 24): ModelerCommand {
  return {
    label: `Paste ${clipboard.elements.length + clipboard.dataObjects.length} elements`,
    apply(document) {
      const owner = findOwner(document, clipboard.ownerId) ?? document.model.processes[0];
      if (!owner) return;

      const ids = new Set<string>();
      collectObjectIds(document, ids);
      const copiedIds = new Set<string>();
      collectObjectIds(clipboard.elements, copiedIds);
      collectObjectIds(clipboard.dataObjects, copiedIds);
      collectObjectIds(clipboard.artifacts, copiedIds);
      const idMap = allocateIds(copiedIds, ids);

      const elements = structuredClone(clipboard.elements);
      const dataObjects = structuredClone(clipboard.dataObjects);
      const artifacts = structuredClone(clipboard.artifacts);
      remapGraph(elements, idMap);
      remapGraph(dataObjects, idMap);
      remapGraph(artifacts, idMap);
      owner.flowElements ??= [];
      owner.dataObjects ??= [];
      owner.artifacts ??= [];
      owner.flowElements.push(...elements);
      owner.dataObjects.push(...dataObjects);
      owner.artifacts.push(...artifacts);

      pasteGraphicMap(document.model.locationMap, clipboard.locationMap, idMap, offset);
      pasteGraphicMap(document.model.labelLocationMap, clipboard.labelLocationMap, idMap, offset);
      for (const [oldId, waypoints] of Object.entries(clipboard.flowLocationMap)) {
        const newId = idMap.get(oldId);
        if (!newId) continue;
        document.model.flowLocationMap[newId] = waypoints.map((point) => ({
          ...point,
          x: point.x + offset,
          y: point.y + offset,
        }));
      }
      for (const [oldId, edge] of Object.entries(clipboard.edgeMap)) {
        const newId = idMap.get(oldId);
        if (!newId) continue;
        const copy = structuredClone(edge);
        remapGraph(copy, idMap);
        copy.waypoints = (document.model.flowLocationMap[newId] ?? []).map((point) => ({
          ...point,
        }));
        document.model.edgeMap[newId] = copy;
      }
      normalizeModelInvariants(document);
    },
  };
}

function locateOwnedValue(document: BpmnEditorDocument, id: string) {
  for (const process of document.model.processes) {
    const located = locateInOwner(process, id);
    if (located) return located;
  }
  return null;
}

function locateInOwner(owner: CanonicalOwner, id: string): { owner: CanonicalOwner } | null {
  if ((owner.dataObjects ?? []).some((value) => value.id === id)) return { owner };
  if ((owner.artifacts ?? []).some((value) => value.id === id)) return { owner };
  for (const element of owner.flowElements ?? []) {
    if (element.id === id) return { owner };
    if (isNestedOwner(element)) {
      const located = locateInOwner(element, id);
      if (located) return located;
    }
  }
  return null;
}

function findOwner(
  document: Draft<BpmnEditorDocument>,
  ownerId: string | null,
): DraftCanonicalOwner | null {
  for (const process of document.model.processes) {
    if ((process.id ?? null) === ownerId) return process;
    const nested = findNestedOwner(process, ownerId);
    if (nested) return nested;
  }
  return null;
}

function findNestedOwner(
  owner: DraftCanonicalOwner,
  ownerId: string | null,
): DraftCanonicalOwner | null {
  for (const element of owner.flowElements ?? []) {
    if (!isDraftNestedOwner(element)) continue;
    if ((element.id ?? null) === ownerId) return element;
    const nested = findNestedOwner(element, ownerId);
    if (nested) return nested;
  }
  return null;
}

function isNestedOwner(element: FlowElementEnum): element is NestedOwner {
  return (
    element.elementType === 'subProcess' ||
    element.elementType === 'transaction' ||
    element.elementType === 'eventSubProcess' ||
    element.elementType === 'adhocSubProcess'
  );
}

function isDraftNestedOwner(element: Draft<FlowElementEnum>): element is Draft<NestedOwner> {
  return (
    element.elementType === 'subProcess' ||
    element.elementType === 'transaction' ||
    element.elementType === 'eventSubProcess' ||
    element.elementType === 'adhocSubProcess'
  );
}

function collectFlowElementIds(element: FlowElementEnum, ids: Set<string>) {
  if (element.id) ids.add(element.id);
  if (isNestedOwner(element)) {
    for (const child of element.flowElements ?? []) collectFlowElementIds(child, ids);
    for (const value of element.dataObjects ?? []) if (value.id) ids.add(value.id);
    for (const artifact of element.artifacts ?? []) if (artifact.id) ids.add(artifact.id);
  }
}

function collectObjectIds(value: unknown, ids: Set<string>) {
  if (Array.isArray(value)) {
    for (const item of value) collectObjectIds(item, ids);
    return;
  }
  if (!isRecord(value)) return;
  if (typeof value.id === 'string' && value.id) ids.add(value.id);
  for (const child of Object.values(value)) collectObjectIds(child, ids);
}

function allocateIds(copiedIds: Set<string>, existingIds: Set<string>) {
  const result = new Map<string, string>();
  for (const oldId of [...copiedIds].sort()) {
    let suffix = 1;
    let candidate = `${oldId}-copy-${suffix}`;
    while (existingIds.has(candidate)) {
      suffix += 1;
      candidate = `${oldId}-copy-${suffix}`;
    }
    existingIds.add(candidate);
    result.set(oldId, candidate);
  }
  return result;
}

const referenceKeys = new Set([
  'attachedToRefId',
  'dataObjectRef',
  'defaultFlow',
  'sourceRef',
  'targetRef',
]);

function remapGraph(value: unknown, idMap: ReadonlyMap<string, string>) {
  if (Array.isArray(value)) {
    for (const item of value) remapGraph(item, idMap);
    return;
  }
  if (!isRecord(value)) return;
  for (const [key, child] of Object.entries(value)) {
    if (typeof child === 'string' && (key === 'id' || referenceKeys.has(key)) && idMap.has(child)) {
      value[key] = idMap.get(child);
    } else {
      remapGraph(child, idMap);
    }
  }
}

function copyGraphicMap(source: Record<string, GraphicInfo>, ids: ReadonlySet<string>) {
  return Object.fromEntries(
    Object.entries(source)
      .filter(([id]) => ids.has(id))
      .map(([id, bounds]) => [id, structuredClone(bounds)]),
  );
}

function copyRouteMap(source: Record<string, GraphicInfo[]>, ids: ReadonlySet<string>) {
  return Object.fromEntries(
    Object.entries(source)
      .filter(([id]) => ids.has(id))
      .map(([id, waypoints]) => [id, structuredClone(waypoints)]),
  );
}

function copyEdgeMap(source: Record<string, BpmnDiEdge>, ids: ReadonlySet<string>) {
  return Object.fromEntries(
    Object.entries(source)
      .filter(([id]) => ids.has(id))
      .map(([id, edge]) => [id, structuredClone(edge)]),
  );
}

function pasteGraphicMap(
  target: Draft<Record<string, GraphicInfo>>,
  source: Record<string, GraphicInfo>,
  idMap: ReadonlyMap<string, string>,
  offset: number,
) {
  for (const [oldId, bounds] of Object.entries(source)) {
    const newId = idMap.get(oldId);
    if (!newId) continue;
    target[newId] = { ...bounds, x: bounds.x + offset, y: bounds.y + offset };
  }
}

function uniqueById<T extends { id?: string | null }>(values: T[]) {
  const seen = new Set<string>();
  return values.filter((value) => {
    if (!value.id) return true;
    if (seen.has(value.id)) return false;
    seen.add(value.id);
    return true;
  });
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}
