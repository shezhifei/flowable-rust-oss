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
