import type { Draft } from 'immer';

import type {
  BpmnEditorDocument,
  FlowElementEnum,
  GraphicInfo,
} from '../generated/editor-protocol';

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
      process.flowElements ??= [];
      process.flowElementMap ??= {};
      process.flowElements.push(element);
      process.flowElementMap[element.id] = element;
      document.model.locationMap[element.id] = bounds;
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
      for (const id of ids) {
        delete document.model.locationMap[id];
        delete document.model.labelLocationMap[id];
        delete document.model.flowLocationMap[id];
        delete document.model.edgeMap[id];
      }
    },
  };
}

function collectElements(elements: Draft<FlowElementEnum>[]): Draft<FlowElementEnum>[] {
  return elements.flatMap((element) => [element, ...collectElements(nestedElements(element))]);
}

function collectDescendantIds(element: Draft<FlowElementEnum>, ids: Set<string>) {
  for (const child of nestedElements(element)) {
    if (child.id) ids.add(child.id);
    collectDescendantIds(child, ids);
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
