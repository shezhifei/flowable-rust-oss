import type { Draft } from 'immer';

import type {
  BpmnEditorDocument,
  FlowElementEnum,
  GraphicInfo,
  Process,
  ValuedDataObject,
} from '../generated/editor-protocol';
import type { ModelerCommand } from './commands';
import {
  createPaletteElement,
  defaultElementSize,
  type CanonicalPaletteElementKind,
} from './elementFactory';
import { snapToGrid, type Point } from './geometry';
import {
  locateCanonicalElement,
  normalizeModelInvariants,
  valuedDataObjectFromElement,
  type CanonicalOwner,
} from './modelInvariants';
import {
  laneForElement,
  processForOwner,
  resolveBoundaryAttachment,
  resolveDropOwner,
  type DropOwnerResolution,
} from './ownership';
import { moveElementsCommand } from './transformCommands';

type NestedOwnerElement =
  | Extract<FlowElementEnum, { elementType: 'subProcess' }>
  | Extract<FlowElementEnum, { elementType: 'transaction' }>
  | Extract<FlowElementEnum, { elementType: 'eventSubProcess' }>
  | Extract<FlowElementEnum, { elementType: 'adhocSubProcess' }>;

export const BPMN_PALETTE_MIME = 'application/x-flowable-modeler-palette';

export function createAtPointCommand(
  kind: CanonicalPaletteElementKind,
  elementId: string,
  point: Point,
): ModelerCommand {
  return {
    label: `Create ${elementId}`,
    apply(document) {
      if (!elementId || hasCanonicalId(document, elementId)) return;
      const dropOwner = resolveDropOwner(document, point);
      if (!dropOwner) return;
      const element = createPaletteElement(kind, elementId);

      if (kind === 'boundaryTimer') {
        const size = defaultElementSize(kind);
        if (!dropOwner.processId) return;
        const attachment = resolveBoundaryAttachment(document, dropOwner.processId, point, size);
        if (!attachment || element.elementType !== 'boundaryEvent') return;
        element.attachedToRefId = attachment.hostId;
        addFlowElement(attachment.owner, element);
        document.model.locationMap[elementId] = attachment.bounds;
        assignLane(document, attachment.process, elementId, attachment.laneId);
        normalizeModelInvariants(document);
        return;
      }

      const size = defaultElementSize(kind);
      document.model.locationMap[elementId] = centeredBounds(point, size.width, size.height);
      if (element.elementType === 'valuedDataObject') {
        addDataObject(dropOwner.owner, valuedDataObjectFromElement(element));
      } else {
        addFlowElement(dropOwner.owner, element);
        if (isLaneEligible(element)) {
          assignLane(document, dropOwner.process, elementId, dropOwner.laneId);
        }
      }
      normalizeModelInvariants(document);
    },
  };
}

export function nextPaletteElementId(
  document: BpmnEditorDocument,
  kind: CanonicalPaletteElementKind,
  prefix = 'modeler',
): string {
  let suffix = 1;
  while (hasCanonicalId(document, `${prefix}-${kind}-${suffix}`)) suffix += 1;
  return `${prefix}-${kind}-${suffix}`;
}

/**
 * Completes a pointer move as one history entry. Ownership and lane changes are
 * validated at the final snapped center before any DI is translated, so an
 * illegal cross-container gesture cannot leave geometry outside its owner.
 */
export function moveAndReparentElementsCommand(
  elementIds: readonly string[],
  deltaX: number,
  deltaY: number,
): ModelerCommand {
  return {
    label: `Move ${elementIds.length} element${elementIds.length === 1 ? '' : 's'}`,
    apply(document) {
      const snappedDelta = { x: snapToGrid(deltaX), y: snapToGrid(deltaY) };
      if (snappedDelta.x === 0 && snappedDelta.y === 0) return;

      const ownershipMoves: Array<{ elementId: string; point: Point }> = [];
      for (const elementId of [...new Set(elementIds)]) {
        const located = locateCanonicalElement(document, elementId);
        const bounds = document.model.locationMap[elementId];
        if (!located || !bounds) continue;
        const point = {
          x: bounds.x + bounds.width / 2 + snappedDelta.x,
          y: bounds.y + bounds.height / 2 + snappedDelta.y,
        };
        if (!canReparentElementAtPoint(document, elementId, point)) return;
        ownershipMoves.push({ elementId, point });
      }

      for (const move of ownershipMoves) {
        reparentElementCommand(move.elementId, move.point).apply(document);
      }
      moveElementsCommand(elementIds, snappedDelta.x, snappedDelta.y).apply(document);
    },
  };
}

export function canReparentElementAtPoint(
  document: Draft<BpmnEditorDocument>,
  elementId: string,
  dropPoint: Point,
): boolean {
  const located = locateCanonicalElement(document, elementId);
  const target = resolveDropOwner(document, dropPoint);
  if (!located || !target) return false;
  if (located.owner === target.owner) return true;
  if (located.kind === 'dataObject') return true;
  const sourceProcess = processForOwner(document, located.owner);
  return Boolean(
    sourceProcess && canChangeCanonicalOwner(located.element, located.owner, target, sourceProcess),
  );
}

export function reparentElementCommand(elementId: string, dropPoint: Point): ModelerCommand {
  return {
    label: `Reparent ${elementId}`,
    apply(document) {
      const located = locateCanonicalElement(document, elementId);
      const target = resolveDropOwner(document, dropPoint);
      if (!located || !target) return;
      const sourceProcess = processForOwner(document, located.owner);
      if (!sourceProcess) return;

      if (located.owner === target.owner) {
        if (located.kind === 'flowElement' && isLaneEligible(located.element)) {
          if (laneForElement(sourceProcess, elementId) === target.laneId) return;
          assignLane(document, target.process, elementId, target.laneId);
          normalizeModelInvariants(document);
        }
        return;
      }

      if (located.kind === 'flowElement') {
        if (!canChangeCanonicalOwner(located.element, located.owner, target, sourceProcess)) return;
        if (located.element.elementType === 'valuedDataObject') {
          const dataObject = removeMirroredDataObject(located.owner, elementId);
          if (!dataObject) return;
          addDataObject(target.owner, dataObject);
        } else {
          const element = removeFlowElement(located.owner, elementId);
          if (!element) return;
          addFlowElement(target.owner, element);
          if (isLaneEligible(element)) {
            assignLane(document, target.process, elementId, target.laneId);
          }
        }
      } else {
        const dataObject = removeDataObject(located.owner, elementId);
        if (!dataObject) return;
        addDataObject(target.owner, dataObject);
        clearLaneMembership(document, elementId);
      }

      normalizeModelInvariants(document);
    },
  };
}

function canChangeCanonicalOwner(
  element: Draft<FlowElementEnum>,
  sourceOwner: CanonicalOwner,
  target: DropOwnerResolution,
  sourceProcess: Draft<Process>,
) {
  if (element.elementType === 'sequenceFlow' || element.elementType === 'boundaryEvent') {
    return false;
  }
  if (target.ownerId === element.id) return false;
  if (target.ownerId && descendantOwnerIds(element).has(target.ownerId)) return false;
  if (hasIncidentSequenceFlow(sourceProcess, element.id ?? '')) return false;
  if (hasAttachedBoundaryEvent(sourceOwner, element.id ?? '')) return false;
  return true;
}

function addFlowElement(owner: CanonicalOwner, element: Draft<FlowElementEnum>) {
  owner.flowElements ??= [];
  owner.flowElements.push(element);
}

function addDataObject(owner: CanonicalOwner, dataObject: Draft<ValuedDataObject>) {
  owner.dataObjects ??= [];
  owner.dataObjects.push(dataObject);
}

function removeFlowElement(owner: CanonicalOwner, elementId: string) {
  const index = owner.flowElements?.findIndex((element) => element.id === elementId) ?? -1;
  if (index < 0) return null;
  return owner.flowElements?.splice(index, 1)[0] ?? null;
}

function removeDataObject(owner: CanonicalOwner, elementId: string) {
  const index = owner.dataObjects?.findIndex((dataObject) => dataObject.id === elementId) ?? -1;
  if (index < 0) return null;
  return owner.dataObjects?.splice(index, 1)[0] ?? null;
}

function removeMirroredDataObject(owner: CanonicalOwner, elementId: string) {
  const dataObjectIndex =
    owner.dataObjects?.findIndex((dataObject) => dataObject.id === elementId) ?? -1;
  if (dataObjectIndex < 0) return null;
  const dataObject = owner.dataObjects?.splice(dataObjectIndex, 1)[0] ?? null;
  const mirrorIndex = owner.flowElements?.findIndex(
    (element) => element.id === elementId && element.elementType === 'valuedDataObject',
  );
  if (mirrorIndex !== undefined && mirrorIndex >= 0) owner.flowElements?.splice(mirrorIndex, 1);
  return dataObject;
}

function assignLane(
  document: Draft<BpmnEditorDocument>,
  process: Draft<Process>,
  elementId: string,
  laneId: string | null,
) {
  clearLaneMembership(document, elementId);
  if (!laneId) return;
  const lane = process.lanes?.find((candidate) => candidate.id === laneId);
  if (lane && !lane.flowReferences.includes(elementId)) lane.flowReferences.push(elementId);
}

function clearLaneMembership(document: Draft<BpmnEditorDocument>, elementId: string) {
  for (const process of document.model.processes) {
    for (const lane of process.lanes ?? []) {
      lane.flowReferences = lane.flowReferences.filter((reference) => reference !== elementId);
    }
  }
}

function hasIncidentSequenceFlow(process: Draft<Process>, elementId: string) {
  return collectFlowElements(process.flowElements ?? []).some(
    (element) =>
      element.elementType === 'sequenceFlow' &&
      (element.sourceRef === elementId || element.targetRef === elementId),
  );
}

function hasAttachedBoundaryEvent(owner: CanonicalOwner, elementId: string) {
  return (owner.flowElements ?? []).some(
    (element) => element.elementType === 'boundaryEvent' && element.attachedToRefId === elementId,
  );
}

function descendantOwnerIds(element: Draft<FlowElementEnum>): Set<string> {
  const ids = new Set<string>();
  for (const child of nestedFlowElements(element)) {
    if (isSubprocess(child) && child.id) ids.add(child.id);
    for (const id of descendantOwnerIds(child)) ids.add(id);
  }
  return ids;
}

function collectFlowElements(elements: Draft<FlowElementEnum>[]): Draft<FlowElementEnum>[] {
  return elements.flatMap((element) => [
    element,
    ...collectFlowElements(nestedFlowElements(element)),
  ]);
}

function nestedFlowElements(element: Draft<FlowElementEnum>): Draft<FlowElementEnum>[] {
  return isSubprocess(element) ? (element.flowElements ?? []) : [];
}

function isSubprocess(element: Draft<FlowElementEnum>): element is Draft<NestedOwnerElement> {
  return (
    element.elementType === 'subProcess' ||
    element.elementType === 'transaction' ||
    element.elementType === 'eventSubProcess' ||
    element.elementType === 'adhocSubProcess'
  );
}

function isLaneEligible(element: Draft<FlowElementEnum>) {
  return element.elementType !== 'sequenceFlow' && element.elementType !== 'valuedDataObject';
}

function centeredBounds(point: Point, width: number, height: number): GraphicInfo {
  return {
    x: point.x - width / 2,
    y: point.y - height / 2,
    width,
    height,
    rotation: 0,
    expanded: true,
    xmlRowNumber: 0,
    xmlColumnNumber: 0,
  };
}

function hasCanonicalId(document: Draft<BpmnEditorDocument>, elementId: string) {
  if (locateCanonicalElement(document, elementId)) return true;
  if (
    document.model.locationMap[elementId] ||
    document.model.flowLocationMap[elementId] ||
    document.model.labelLocationMap[elementId] ||
    document.model.edgeMap[elementId] ||
    document.model.dataStores[elementId] ||
    document.model.messageFlows[elementId]
  ) {
    return true;
  }
  if (document.model.globalArtifacts.some((artifact) => artifact.id === elementId)) return true;
  return document.model.processes.some(
    (process) =>
      process.id === elementId ||
      process.lanes?.some((lane) => lane.id === elementId) ||
      collectArtifacts(process.flowElements ?? [], process.artifacts ?? []).some(
        (artifactId) => artifactId === elementId,
      ),
  );
}

function collectArtifacts(
  elements: Draft<FlowElementEnum>[],
  ownArtifacts: Draft<{ id?: string | null }>[],
): string[] {
  return [
    ...ownArtifacts.flatMap((artifact) => (artifact.id ? [artifact.id] : [])),
    ...elements.flatMap((element) =>
      isSubprocess(element)
        ? collectArtifacts(element.flowElements ?? [], element.artifacts ?? [])
        : [],
    ),
  ];
}
