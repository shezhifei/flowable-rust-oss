import type { Draft } from 'immer';

import type {
  ArtifactEnum,
  BpmnEditorDocument,
  BoundaryEvent,
  FlowElementEnum,
  Process,
  SequenceFlow,
  ValuedDataObject,
} from '../generated/editor-protocol';

type NestedContainerElement =
  | Extract<FlowElementEnum, { elementType: 'subProcess' }>
  | Extract<FlowElementEnum, { elementType: 'transaction' }>
  | Extract<FlowElementEnum, { elementType: 'eventSubProcess' }>
  | Extract<FlowElementEnum, { elementType: 'adhocSubProcess' }>;

export type CanonicalOwner = Draft<Process> | Draft<NestedContainerElement>;

export type LocatedCanonicalElement =
  | {
      kind: 'flowElement';
      element: Draft<FlowElementEnum>;
      owner: CanonicalOwner;
      ownerId: string | null;
    }
  | {
      kind: 'dataObject';
      element: Draft<ValuedDataObject>;
      owner: CanonicalOwner;
      ownerId: string | null;
    };

/** Finds the author-owned element and the process/subprocess that directly contains it. */
export function locateCanonicalElement(
  document: Draft<BpmnEditorDocument>,
  elementId: string,
): LocatedCanonicalElement | null {
  for (const process of document.model.processes) {
    const located = locateInOwner(process, elementId);
    if (located) return located;
  }
  return null;
}

/**
 * Rebuilds every lookup/relationship mirror derived from the author-owned
 * process, subprocess, data-object, and artifact collections.
 */
export function normalizeModelInvariants(document: Draft<BpmnEditorDocument>) {
  for (const process of document.model.processes) {
    rebuildFlowRelationships(process);
    rebuildOwnerMaps(process);
    normalizeLaneReferences(process);
  }

  const mainProcess = document.model.processes[0];
  document.model.mainProcess = mainProcess ? cloneCanonical(mainProcess) : null;
}

export function valuedDataObjectFromElement(
  element: Draft<Extract<FlowElementEnum, { elementType: 'valuedDataObject' }>>,
): Draft<ValuedDataObject> {
  const copy = cloneCanonical(element);
  const { elementType: _elementType, ...dataObject } = copy;
  void _elementType;
  return dataObject;
}

function locateInOwner(owner: CanonicalOwner, elementId: string): LocatedCanonicalElement | null {
  for (const element of owner.flowElements ?? []) {
    if (element.id === elementId) {
      return { kind: 'flowElement', element, owner, ownerId: owner.id ?? null };
    }
    const nestedOwner = asNestedOwner(element);
    if (nestedOwner) {
      const located = locateInOwner(nestedOwner, elementId);
      if (located) return located;
    }
  }
  for (const dataObject of owner.dataObjects ?? []) {
    if (dataObject.id === elementId) {
      return { kind: 'dataObject', element: dataObject, owner, ownerId: owner.id ?? null };
    }
  }
  return null;
}

function rebuildFlowRelationships(process: Draft<Process>) {
  const elements = collectFlowElements(process.flowElements ?? []);
  const nodesById = new Map<string, Draft<Exclude<FlowElementEnum, SequenceOrDataElement>>>();
  const sequenceFlows: Draft<Extract<FlowElementEnum, { elementType: 'sequenceFlow' }>>[] = [];
  const boundaryEvents: Draft<Extract<FlowElementEnum, { elementType: 'boundaryEvent' }>>[] = [];

  for (const element of elements) {
    if (element.elementType === 'sequenceFlow') {
      sequenceFlows.push(element);
      continue;
    }
    if (element.elementType === 'valuedDataObject') continue;
    element.incomingFlows = [];
    element.outgoingFlows = [];
    if (isBoundaryHost(element)) element.boundaryEvents = [];
    if (element.id) nodesById.set(element.id, element);
    if (element.elementType === 'boundaryEvent') boundaryEvents.push(element);
  }

  for (const flow of sequenceFlows) {
    if (flow.sourceRef) nodesById.get(flow.sourceRef)?.outgoingFlows.push(sequenceFlowMirror(flow));
    if (flow.targetRef) nodesById.get(flow.targetRef)?.incomingFlows.push(sequenceFlowMirror(flow));
  }

  for (const boundaryEvent of boundaryEvents) {
    if (!boundaryEvent.attachedToRefId) continue;
    const host = nodesById.get(boundaryEvent.attachedToRefId);
    if (host && isBoundaryHost(host)) {
      host.boundaryEvents ??= [];
      host.boundaryEvents.push(boundaryEventMirror(boundaryEvent));
    }
  }
}

type SequenceOrDataElement =
  | Extract<FlowElementEnum, { elementType: 'sequenceFlow' }>
  | Extract<FlowElementEnum, { elementType: 'valuedDataObject' }>;

function rebuildOwnerMaps(owner: CanonicalOwner) {
  for (const element of owner.flowElements ?? []) {
    const nestedOwner = asNestedOwner(element);
    if (nestedOwner) rebuildOwnerMaps(nestedOwner);
  }

  const flowElementMap: Draft<Record<string, FlowElementEnum>> = {};
  for (const dataObject of collectDataObjects(owner)) {
    if (dataObject.id) flowElementMap[dataObject.id] = dataObjectMirror(dataObject);
  }
  for (const element of collectFlowElements(owner.flowElements ?? [])) {
    if (element.id) flowElementMap[element.id] = cloneCanonical(element);
  }
  owner.flowElementMap = flowElementMap;

  const artifactMap: Draft<Record<string, ArtifactEnum>> = {};
  for (const artifact of collectArtifacts(owner)) {
    if (artifact.id) artifactMap[artifact.id] = cloneCanonical(artifact);
  }
  owner.artifactMap = artifactMap;
}

function normalizeLaneReferences(process: Draft<Process>) {
  const validNodeIds = new Set(
    collectFlowElements(process.flowElements ?? []).flatMap((element) =>
      element.id &&
      element.elementType !== 'sequenceFlow' &&
      element.elementType !== 'valuedDataObject'
        ? [element.id]
        : [],
    ),
  );
  const claimed = new Set<string>();
  for (const lane of process.lanes ?? []) {
    lane.flowReferences = lane.flowReferences.filter((elementId) => {
      if (!validNodeIds.has(elementId) || claimed.has(elementId)) return false;
      claimed.add(elementId);
      return true;
    });
  }
}

function collectFlowElements(elements: Draft<FlowElementEnum>[]): Draft<FlowElementEnum>[] {
  return elements.flatMap((element) => {
    const nestedOwner = asNestedOwner(element);
    return [element, ...(nestedOwner ? collectFlowElements(nestedOwner.flowElements ?? []) : [])];
  });
}

function collectDataObjects(owner: CanonicalOwner): Draft<ValuedDataObject>[] {
  return [
    ...(owner.dataObjects ?? []),
    ...(owner.flowElements ?? []).flatMap((element) => {
      const nestedOwner = asNestedOwner(element);
      return nestedOwner ? collectDataObjects(nestedOwner) : [];
    }),
  ];
}

function collectArtifacts(owner: CanonicalOwner): Draft<ArtifactEnum>[] {
  return [
    ...(owner.artifacts ?? []),
    ...(owner.flowElements ?? []).flatMap((element) => {
      const nestedOwner = asNestedOwner(element);
      return nestedOwner ? collectArtifacts(nestedOwner) : [];
    }),
  ];
}

function asNestedOwner(element: Draft<FlowElementEnum>): Draft<NestedContainerElement> | null {
  switch (element.elementType) {
    case 'subProcess':
    case 'transaction':
    case 'eventSubProcess':
    case 'adhocSubProcess':
      return element;
    default:
      return null;
  }
}

type BoundaryHostElement =
  | Extract<FlowElementEnum, { elementType: 'task' }>
  | Extract<FlowElementEnum, { elementType: 'userTask' }>
  | Extract<FlowElementEnum, { elementType: 'serviceTask' }>
  | Extract<FlowElementEnum, { elementType: 'caseServiceTask' }>
  | Extract<FlowElementEnum, { elementType: 'sendTask' }>
  | Extract<FlowElementEnum, { elementType: 'scriptTask' }>
  | Extract<FlowElementEnum, { elementType: 'manualTask' }>
  | Extract<FlowElementEnum, { elementType: 'receiveTask' }>
  | Extract<FlowElementEnum, { elementType: 'businessRuleTask' }>
  | NestedContainerElement
  | Extract<FlowElementEnum, { elementType: 'callActivity' }>;

function isBoundaryHost(
  element: Draft<Exclude<FlowElementEnum, SequenceOrDataElement>>,
): element is Draft<BoundaryHostElement> {
  switch (element.elementType) {
    case 'task':
    case 'userTask':
    case 'serviceTask':
    case 'caseServiceTask':
    case 'sendTask':
    case 'scriptTask':
    case 'manualTask':
    case 'receiveTask':
    case 'businessRuleTask':
    case 'subProcess':
    case 'transaction':
    case 'eventSubProcess':
    case 'adhocSubProcess':
    case 'callActivity':
      return true;
    default:
      return false;
  }
}

function sequenceFlowMirror(
  flow: Draft<Extract<FlowElementEnum, { elementType: 'sequenceFlow' }>>,
): Draft<SequenceFlow> {
  const copy = cloneCanonical(flow);
  const { elementType: _elementType, ...mirror } = copy;
  void _elementType;
  return mirror;
}

function boundaryEventMirror(
  event: Draft<Extract<FlowElementEnum, { elementType: 'boundaryEvent' }>>,
): Draft<BoundaryEvent> {
  const copy = cloneCanonical(event);
  const { elementType: _elementType, ...mirror } = copy;
  void _elementType;
  return mirror;
}

function dataObjectMirror(dataObject: Draft<ValuedDataObject>): Draft<FlowElementEnum> {
  return {
    elementType: 'valuedDataObject',
    ...cloneCanonical(dataObject),
  };
}

function cloneCanonical<T extends object>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}
