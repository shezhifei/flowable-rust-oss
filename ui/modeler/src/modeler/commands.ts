import type { Draft } from 'immer';

import type {
  ArtifactEnum,
  BpmnEditorDocument,
  FlowElementEnum,
  GraphicInfo,
} from '../generated/editor-protocol';
import { normalizeModelInvariants, valuedDataObjectFromElement } from './modelInvariants';

export interface ModelerCommand {
  label: string;
  apply: (document: Draft<BpmnEditorDocument>) => void;
}

export function createElementCommand(
  element: FlowElementEnum,
  bounds: GraphicInfo,
): ModelerCommand {
  return {
    label: `Create ${element.id ?? element.elementType}`,
    apply(document) {
      const process = document.model.processes[0];
      if (!process || !element.id) return;
      if (element.elementType === 'valuedDataObject') {
        process.dataObjects ??= [];
        process.dataObjects.push(valuedDataObjectFromElement(element));
      } else {
        process.flowElements ??= [];
        process.flowElements.push(element);
      }
      document.model.locationMap[element.id] = bounds;
      normalizeModelInvariants(document);
    },
  };
}

export function deleteElementsCommand(elementIds: string[]): ModelerCommand {
  return {
    label: `Delete ${elementIds.length} element${elementIds.length === 1 ? '' : 's'}`,
    apply(document) {
      const ids = new Set(elementIds);
      const allElements = document.model.processes.flatMap((process) =>
        collectElements(process.flowElements ?? []),
      );

      for (const element of allElements) {
        if (element.id && ids.has(element.id)) collectDescendantIds(element, ids);
      }
      for (const element of allElements) {
        if (
          element.elementType === 'boundaryEvent' &&
          element.id &&
          element.attachedToRefId &&
          ids.has(element.attachedToRefId)
        ) {
          ids.add(element.id);
        }
      }
      for (const element of allElements) {
        if (
          element.elementType === 'sequenceFlow' &&
          element.id &&
          ((element.sourceRef && ids.has(element.sourceRef)) ||
            (element.targetRef && ids.has(element.targetRef)))
        ) {
          ids.add(element.id);
        }
      }

      for (const process of document.model.processes) {
        removeElements(process.flowElements ?? [], ids);
        for (const id of ids) delete process.flowElementMap?.[id];
        process.dataObjects = (process.dataObjects ?? []).filter(
          (dataObject) => !dataObject.id || !ids.has(dataObject.id),
        );
        removeNestedDataObjects(process.flowElements ?? [], ids);
        for (const lane of process.lanes ?? []) {
          lane.flowReferences = lane.flowReferences.filter((id) => !ids.has(id));
        }
      }

      for (const [id, flow] of Object.entries(document.model.messageFlows)) {
        if (
          ids.has(id) ||
          (flow.sourceRef && ids.has(flow.sourceRef)) ||
          (flow.targetRef && ids.has(flow.targetRef))
        ) {
          ids.add(id);
          delete document.model.messageFlows[id];
        }
      }
      const artifacts = allArtifacts(document);
      let foundConnectedAssociation = true;
      while (foundConnectedAssociation) {
        foundConnectedAssociation = false;
        for (const artifact of artifacts) {
          if (
            artifact.artifactType === 'association' &&
            artifact.id &&
            !ids.has(artifact.id) &&
            ((artifact.sourceRef && ids.has(artifact.sourceRef)) ||
              (artifact.targetRef && ids.has(artifact.targetRef)))
          ) {
            ids.add(artifact.id);
            foundConnectedAssociation = true;
          }
        }
      }
      document.model.globalArtifacts = document.model.globalArtifacts.filter(
        (artifact) => !shouldDeleteArtifact(artifact, ids),
      );
      for (const process of document.model.processes) {
        removeArtifacts(process.artifacts ?? [], process.flowElements ?? [], ids);
      }
      for (const id of ids) {
        delete document.model.locationMap[id];
        delete document.model.labelLocationMap[id];
        delete document.model.flowLocationMap[id];
        delete document.model.edgeMap[id];
      }
      normalizeModelInvariants(document);
    },
  };
}

function allArtifacts(document: Draft<BpmnEditorDocument>): Draft<ArtifactEnum>[] {
  return [
    ...document.model.globalArtifacts,
    ...document.model.processes.flatMap((process) => [
      ...(process.artifacts ?? []),
      ...nestedArtifacts(process.flowElements ?? []),
    ]),
  ];
}

function nestedArtifacts(elements: Draft<FlowElementEnum>[]): Draft<ArtifactEnum>[] {
  return elements.flatMap((element) => {
    const nested = nestedElements(element);
    const artifacts = nestedOwnerArtifacts(element);
    return artifacts ? [...artifacts, ...nestedArtifacts(nested)] : [];
  });
}

function nestedOwnerArtifacts(element: Draft<FlowElementEnum>): Draft<ArtifactEnum>[] | undefined {
  switch (element.elementType) {
    case 'subProcess':
    case 'transaction':
    case 'eventSubProcess':
    case 'adhocSubProcess':
      return element.artifacts;
    default:
      return undefined;
  }
}

function removeArtifacts(
  artifacts: Draft<ArtifactEnum>[],
  elements: Draft<FlowElementEnum>[],
  ids: Set<string>,
) {
  for (let index = artifacts.length - 1; index >= 0; index -= 1) {
    const artifact = artifacts[index];
    if (artifact && shouldDeleteArtifact(artifact, ids)) artifacts.splice(index, 1);
  }
  for (const element of elements) {
    const nested = nestedElements(element);
    const nestedArtifacts = nestedOwnerArtifacts(element);
    if (nestedArtifacts) removeArtifacts(nestedArtifacts, nested, ids);
  }
}

function shouldDeleteArtifact(artifact: Draft<ArtifactEnum>, ids: Set<string>) {
  return (
    (artifact.id !== undefined && artifact.id !== null && ids.has(artifact.id)) ||
    (artifact.artifactType === 'association' &&
      ((artifact.sourceRef !== undefined &&
        artifact.sourceRef !== null &&
        ids.has(artifact.sourceRef)) ||
        (artifact.targetRef !== undefined &&
          artifact.targetRef !== null &&
          ids.has(artifact.targetRef))))
  );
}

function collectElements(elements: Draft<FlowElementEnum>[]): Draft<FlowElementEnum>[] {
  return elements.flatMap((element) => [element, ...collectElements(nestedElements(element))]);
}

function collectDescendantIds(element: Draft<FlowElementEnum>, ids: Set<string>) {
  const nestedDataObjects = nestedOwnerDataObjects(element);
  for (const dataObject of nestedDataObjects ?? []) {
    if (dataObject.id) ids.add(dataObject.id);
  }
  for (const artifact of nestedOwnerArtifacts(element) ?? []) {
    if (artifact.id) ids.add(artifact.id);
  }
  for (const child of nestedElements(element)) {
    if (child.id) ids.add(child.id);
    collectDescendantIds(child, ids);
  }
}

function removeNestedDataObjects(elements: Draft<FlowElementEnum>[], ids: Set<string>) {
  for (const element of elements) {
    const dataObjects = nestedOwnerDataObjects(element);
    if (dataObjects) {
      const retained = dataObjects.filter(
        (dataObject) => !dataObject.id || !ids.has(dataObject.id),
      );
      switch (element.elementType) {
        case 'subProcess':
        case 'transaction':
        case 'eventSubProcess':
        case 'adhocSubProcess':
          element.dataObjects = retained;
          break;
        default:
          break;
      }
    }
    removeNestedDataObjects(nestedElements(element), ids);
  }
}

function nestedOwnerDataObjects(element: Draft<FlowElementEnum>) {
  switch (element.elementType) {
    case 'subProcess':
    case 'transaction':
    case 'eventSubProcess':
    case 'adhocSubProcess':
      return element.dataObjects;
    default:
      return undefined;
  }
}

function removeElements(elements: Draft<FlowElementEnum>[], ids: Set<string>) {
  for (let index = elements.length - 1; index >= 0; index -= 1) {
    const element = elements[index];
    if (!element) continue;
    if (element.id && ids.has(element.id)) {
      elements.splice(index, 1);
    } else {
      removeElements(nestedElements(element), ids);
    }
  }
}

function nestedElements(element: Draft<FlowElementEnum>): Draft<FlowElementEnum>[] {
  switch (element.elementType) {
    case 'subProcess':
    case 'transaction':
    case 'eventSubProcess':
    case 'adhocSubProcess':
      return element.flowElements ?? [];
    default:
      return [];
  }
}

export function moveElementCommand(
  elementId: string,
  deltaX: number,
  deltaY: number,
): ModelerCommand {
  return {
    label: `Move ${elementId}`,
    apply(document) {
      const { model } = document;
      translate(model.locationMap[elementId], deltaX, deltaY);
      translate(model.labelLocationMap[elementId], deltaX, deltaY);

      for (const process of model.processes) {
        moveAttachedBoundaryEvents(
          process.flowElements ?? [],
          elementId,
          model.locationMap,
          model.labelLocationMap,
          deltaX,
          deltaY,
        );
        updateSequenceEndpoints(
          process.flowElements ?? [],
          elementId,
          model.flowLocationMap,
          deltaX,
          deltaY,
        );
      }
      for (const flow of Object.values(model.messageFlows)) {
        updateFlowEndpoints(flow, elementId, model.flowLocationMap[flow.id ?? ''], deltaX, deltaY);
      }
      normalizeModelInvariants(document);
    },
  };
}

function updateSequenceEndpoints(
  elements: Draft<FlowElementEnum>[],
  elementId: string,
  locations: Draft<Record<string, GraphicInfo[]>>,
  deltaX: number,
  deltaY: number,
) {
  for (const element of elements) {
    if (element.elementType === 'sequenceFlow') {
      updateFlowEndpoints(element, elementId, locations[element.id ?? ''], deltaX, deltaY);
      continue;
    }
    switch (element.elementType) {
      case 'subProcess':
      case 'transaction':
      case 'eventSubProcess':
      case 'adhocSubProcess':
        updateSequenceEndpoints(element.flowElements ?? [], elementId, locations, deltaX, deltaY);
        break;
      default:
        break;
    }
  }
}

function updateFlowEndpoints(
  flow: { sourceRef?: string | null; targetRef?: string | null },
  elementId: string,
  waypoints: Draft<GraphicInfo[]> | undefined,
  deltaX: number,
  deltaY: number,
) {
  if (!waypoints?.length) return;
  if (flow.sourceRef === elementId) translate(waypoints[0], deltaX, deltaY);
  if (flow.targetRef === elementId) translate(waypoints[waypoints.length - 1], deltaX, deltaY);
}

function moveAttachedBoundaryEvents(
  elements: Draft<FlowElementEnum>[],
  hostId: string,
  locations: Draft<Record<string, GraphicInfo>>,
  labelLocations: Draft<Record<string, GraphicInfo>>,
  deltaX: number,
  deltaY: number,
) {
  for (const element of elements) {
    if (
      element.elementType === 'boundaryEvent' &&
      element.attachedToRefId === hostId &&
      element.id
    ) {
      translate(locations[element.id], deltaX, deltaY);
      translate(labelLocations[element.id], deltaX, deltaY);
    }
    switch (element.elementType) {
      case 'subProcess':
      case 'transaction':
      case 'eventSubProcess':
      case 'adhocSubProcess':
        moveAttachedBoundaryEvents(
          element.flowElements ?? [],
          hostId,
          locations,
          labelLocations,
          deltaX,
          deltaY,
        );
        break;
      default:
        break;
    }
  }
}

function translate(bounds: Draft<GraphicInfo> | undefined, deltaX: number, deltaY: number) {
  if (!bounds) return;
  bounds.x += deltaX;
  bounds.y += deltaY;
}
