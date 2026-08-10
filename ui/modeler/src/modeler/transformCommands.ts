import type { Draft } from 'immer';

import type {
  ArtifactEnum,
  BpmnEditorDocument,
  FlowElementEnum,
  GraphicInfo,
} from '../generated/editor-protocol';
import type { ModelerCommand } from './commands';
import { rectContainsRect, snapPointToGrid, snapToGrid, type Point, type Rect } from './geometry';
import { insertBendpoint, moveBendpoint, removeBendpoint } from './manhattanRouter';
import {
  locateCanonicalElement,
  normalizeModelInvariants,
  type CanonicalOwner,
} from './modelInvariants';

export type TransformCommandErrorCode =
  | 'invalid-bendpoint-index'
  | 'missing-route'
  | 'missing-resize-bounds'
  | 'unsupported-resize-target';

export class TransformCommandError extends Error {
  readonly code: TransformCommandErrorCode;
  readonly targetId: string;

  constructor(code: TransformCommandErrorCode, targetId: string, message: string) {
    super(message);
    this.name = 'TransformCommandError';
    this.code = code;
    this.targetId = targetId;
  }
}

export function moveElementsCommand(
  elementIds: readonly string[],
  deltaX: number,
  deltaY: number,
): ModelerCommand {
  return {
    label: `Move ${elementIds.length} element${elementIds.length === 1 ? '' : 's'}`,
    apply(document) {
      const snappedDelta = {
        x: snapToGrid(deltaX),
        y: snapToGrid(deltaY),
      };
      const explicitlyMovedIds = new Set(elementIds);
      const movedShapeIds = expandMovedShapeIds(document, explicitlyMovedIds);

      for (const id of movedShapeIds) {
        translate(document.model.locationMap[id], snappedDelta);
        translate(document.model.labelLocationMap[id], snappedDelta);
      }

      const changedRoutes = new Set<string>();
      for (const connection of collectConnections(document)) {
        if (!connection.id) continue;
        const waypoints = document.model.flowLocationMap[connection.id];
        if (!waypoints?.length) continue;
        const sourceMoved = Boolean(
          connection.sourceRef && movedShapeIds.has(connection.sourceRef),
        );
        const targetMoved = Boolean(
          connection.targetRef && movedShapeIds.has(connection.targetRef),
        );
        const moveWholeRoute =
          explicitlyMovedIds.has(connection.id) || (sourceMoved && targetMoved);

        if (moveWholeRoute) {
          for (const waypoint of waypoints) translate(waypoint, snappedDelta);
          translate(document.model.labelLocationMap[connection.id], snappedDelta);
          changedRoutes.add(connection.id);
          continue;
        }

        const endpointIndexes = new Set<number>();
        if (sourceMoved) endpointIndexes.add(0);
        if (targetMoved) endpointIndexes.add(waypoints.length - 1);
        for (const index of endpointIndexes) translate(waypoints[index], snappedDelta);
        if (endpointIndexes.size > 0) changedRoutes.add(connection.id);
      }

      for (const id of explicitlyMovedIds) {
        if (!changedRoutes.has(id) && document.model.flowLocationMap[id]?.length) {
          for (const waypoint of document.model.flowLocationMap[id] ?? []) {
            translate(waypoint, snappedDelta);
          }
          translate(document.model.labelLocationMap[id], snappedDelta);
          changedRoutes.add(id);
        }
      }
      for (const routeId of changedRoutes) syncEdgeMap(document, routeId);
      normalizeModelInvariants(document);
    },
  };
}

export function resizeElementCommand(
  elementId: string,
  requestedWidth: number,
  requestedHeight: number,
): ModelerCommand {
  return {
    label: `Resize ${elementId}`,
    apply(document) {
      const kind = resizeTargetKind(document, elementId);
      if (!kind) {
        throw new TransformCommandError(
          'unsupported-resize-target',
          elementId,
          `${elementId} is not a resizable BPMN container`,
        );
      }
      const bounds = document.model.locationMap[elementId];
      if (!bounds) {
        throw new TransformCommandError(
          'missing-resize-bounds',
          elementId,
          `${elementId} does not have DI bounds`,
        );
      }
      const minimum = minimumSize[kind];
      bounds.width = Math.max(minimum.width, snapToGrid(requestedWidth));
      bounds.height = Math.max(minimum.height, snapToGrid(requestedHeight));
      normalizeModelInvariants(document);
    },
  };
}

export function addBendpointCommand(
  flowId: string,
  segmentIndex: number,
  point: Point,
): ModelerCommand {
  return {
    label: `Add bendpoint to ${flowId}`,
    apply(document) {
      const waypoints = requiredRoute(document, flowId);
      let inserted: Point[];
      try {
        inserted = insertBendpoint(waypoints, segmentIndex, snapPointToGrid(point));
      } catch (error) {
        throw invalidBendpointError(flowId, error);
      }
      const next = inserted.map((nextPoint, index) => {
        if (index === segmentIndex + 1) return newWaypoint(nextPoint);
        const originalIndex = index <= segmentIndex ? index : index - 1;
        return copyWaypoint(requiredWaypoint(waypoints, originalIndex), nextPoint);
      });
      replaceRoute(document, flowId, next);
      normalizeModelInvariants(document);
    },
  };
}

export function moveBendpointCommand(
  flowId: string,
  bendpointIndex: number,
  point: Point,
): ModelerCommand {
  return {
    label: `Move bendpoint on ${flowId}`,
    apply(document) {
      const waypoints = requiredRoute(document, flowId);
      let moved: Point[];
      try {
        moved = moveBendpoint(waypoints, bendpointIndex, snapPointToGrid(point));
      } catch (error) {
        throw invalidBendpointError(flowId, error);
      }
      replaceRoute(
        document,
        flowId,
        moved.map((nextPoint, index) =>
          copyWaypoint(requiredWaypoint(waypoints, index), nextPoint),
        ),
      );
      normalizeModelInvariants(document);
    },
  };
}

export function removeBendpointCommand(flowId: string, bendpointIndex: number): ModelerCommand {
  return {
    label: `Remove bendpoint from ${flowId}`,
    apply(document) {
      const waypoints = requiredRoute(document, flowId);
      let removed: Point[];
      try {
        removed = removeBendpoint(waypoints, bendpointIndex);
      } catch (error) {
        throw invalidBendpointError(flowId, error);
      }
      replaceRoute(
        document,
        flowId,
        removed.map((nextPoint, index) => {
          const originalIndex = index < bendpointIndex ? index : index + 1;
          return copyWaypoint(requiredWaypoint(waypoints, originalIndex), nextPoint);
        }),
      );
      normalizeModelInvariants(document);
    },
  };
}

type ResizeTargetKind =
  'adhocSubProcess' | 'eventSubProcess' | 'group' | 'lane' | 'pool' | 'subProcess' | 'transaction';

const minimumSize: Record<ResizeTargetKind, { height: number; width: number }> = {
  subProcess: { width: 160, height: 100 },
  transaction: { width: 160, height: 100 },
  eventSubProcess: { width: 160, height: 100 },
  adhocSubProcess: { width: 160, height: 100 },
  group: { width: 80, height: 60 },
  pool: { width: 300, height: 160 },
  lane: { width: 240, height: 100 },
};

interface ConnectionRef {
  id?: string | null;
  sourceRef?: string | null;
  targetRef?: string | null;
}

function expandMovedShapeIds(
  document: Draft<BpmnEditorDocument>,
  explicitlyMovedIds: ReadonlySet<string>,
): Set<string> {
  const moved = new Set(
    [...explicitlyMovedIds].filter((id) => document.model.locationMap[id] !== undefined),
  );
  const containerIds = collectContainerIds(document);
  let changed = true;
  while (changed) {
    changed = false;
    for (const containerId of [...moved]) {
      if (!containerIds.has(containerId)) continue;
      const container = document.model.locationMap[containerId];
      if (!container) continue;
      const containerArea = Math.abs(container.width * container.height);
      for (const [candidateId, candidate] of Object.entries(document.model.locationMap)) {
        if (moved.has(candidateId)) continue;
        const candidateArea = Math.abs(candidate.width * candidate.height);
        if (
          candidateArea < containerArea &&
          rectContainsRect(rectOf(container), rectOf(candidate))
        ) {
          moved.add(candidateId);
          changed = true;
        }
      }
    }

    for (const boundary of collectFlowElements(document).filter(
      (element): element is Draft<Extract<FlowElementEnum, { elementType: 'boundaryEvent' }>> =>
        element.elementType === 'boundaryEvent',
    )) {
      if (
        boundary.id &&
        boundary.attachedToRefId &&
        moved.has(boundary.attachedToRefId) &&
        !moved.has(boundary.id)
      ) {
        moved.add(boundary.id);
        changed = true;
      }
    }
  }
  return moved;
}

function collectContainerIds(document: Draft<BpmnEditorDocument>): Set<string> {
  const ids = new Set<string>();
  for (const element of collectFlowElements(document)) {
    if (element.id && isNestedContainerType(element.elementType)) ids.add(element.id);
  }
  for (const artifact of collectArtifacts(document)) {
    if (artifact.artifactType === 'group' && artifact.id) ids.add(artifact.id);
  }
  for (const pool of document.model.pools) if (pool.id) ids.add(pool.id);
  for (const process of document.model.processes) {
    for (const lane of process.lanes ?? []) if (lane.id) ids.add(lane.id);
  }
  return ids;
}

function collectConnections(document: Draft<BpmnEditorDocument>): ConnectionRef[] {
  return [
    ...collectFlowElements(document).filter((element) => element.elementType === 'sequenceFlow'),
    ...Object.values(document.model.messageFlows),
    ...collectArtifacts(document).filter(
      (artifact): artifact is Draft<Extract<ArtifactEnum, { artifactType: 'association' }>> =>
        artifact.artifactType === 'association',
    ),
  ];
}

function collectFlowElements(document: Draft<BpmnEditorDocument>): Draft<FlowElementEnum>[] {
  return document.model.processes.flatMap((process) => collectOwnerFlowElements(process));
}

function collectOwnerFlowElements(owner: CanonicalOwner): Draft<FlowElementEnum>[] {
  return (owner.flowElements ?? []).flatMap((element) => [
    element,
    ...(isNestedContainerType(element.elementType) ? collectOwnerFlowElements(element) : []),
  ]);
}

function collectArtifacts(document: Draft<BpmnEditorDocument>): Draft<ArtifactEnum>[] {
  return [
    ...document.model.globalArtifacts,
    ...document.model.processes.flatMap((process) => collectOwnerArtifacts(process)),
  ];
}

function collectOwnerArtifacts(owner: CanonicalOwner): Draft<ArtifactEnum>[] {
  return [
    ...(owner.artifacts ?? []),
    ...(owner.flowElements ?? []).flatMap((element) =>
      isNestedContainerType(element.elementType) ? collectOwnerArtifacts(element) : [],
    ),
  ];
}

function resizeTargetKind(
  document: Draft<BpmnEditorDocument>,
  elementId: string,
): ResizeTargetKind | null {
  const located = locateCanonicalElement(document, elementId);
  if (located?.kind === 'flowElement' && isNestedContainerType(located.element.elementType)) {
    return located.element.elementType;
  }
  if (
    collectArtifacts(document).some(
      (artifact) => artifact.artifactType === 'group' && artifact.id === elementId,
    )
  ) {
    return 'group';
  }
  if (document.model.pools.some((pool) => pool.id === elementId)) return 'pool';
  if (
    document.model.processes.some((process) =>
      (process.lanes ?? []).some((lane) => lane.id === elementId),
    )
  ) {
    return 'lane';
  }
  return null;
}

function isNestedContainerType(elementType: FlowElementEnum['elementType']): elementType is Extract<
  FlowElementEnum,
  {
    elementType: 'adhocSubProcess' | 'eventSubProcess' | 'subProcess' | 'transaction';
  }
>['elementType'] {
  return (
    elementType === 'subProcess' ||
    elementType === 'transaction' ||
    elementType === 'eventSubProcess' ||
    elementType === 'adhocSubProcess'
  );
}

function requiredRoute(document: Draft<BpmnEditorDocument>, flowId: string): Draft<GraphicInfo>[] {
  const waypoints = document.model.flowLocationMap[flowId];
  if (!waypoints) {
    throw new TransformCommandError(
      'missing-route',
      flowId,
      `${flowId} does not have DI waypoints`,
    );
  }
  return waypoints;
}

function replaceRoute(
  document: Draft<BpmnEditorDocument>,
  flowId: string,
  waypoints: GraphicInfo[],
): void {
  document.model.flowLocationMap[flowId] = waypoints;
  syncEdgeMap(document, flowId);
}

function syncEdgeMap(document: Draft<BpmnEditorDocument>, flowId: string): void {
  const waypoints = document.model.flowLocationMap[flowId];
  if (!waypoints) return;
  const edge = document.model.edgeMap[flowId];
  const mirroredWaypoints = waypoints.map((waypoint) => ({ ...waypoint }));
  if (edge) {
    edge.waypoints = mirroredWaypoints;
  } else {
    document.model.edgeMap[flowId] = {
      id: flowId,
      sourceDockerInfo: null,
      targetDockerInfo: null,
      waypoints: mirroredWaypoints,
    };
  }
}

function translate(bounds: Draft<GraphicInfo> | undefined, delta: Point): void {
  if (!bounds) return;
  bounds.x += delta.x;
  bounds.y += delta.y;
}

function rectOf(bounds: Draft<GraphicInfo>): Rect {
  return { x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height };
}

function newWaypoint(point: Point): GraphicInfo {
  return {
    x: point.x,
    y: point.y,
    width: 0,
    height: 0,
    rotation: 0,
    expanded: true,
    xmlColumnNumber: 0,
    xmlRowNumber: 0,
  };
}

function copyWaypoint(waypoint: Draft<GraphicInfo>, point: Point): GraphicInfo {
  return { ...waypoint, x: point.x, y: point.y };
}

function requiredWaypoint(waypoints: Draft<GraphicInfo>[], index: number): Draft<GraphicInfo> {
  const waypoint = waypoints[index];
  if (!waypoint) throw new RangeError(`waypoint ${index} does not exist`);
  return waypoint;
}

function invalidBendpointError(flowId: string, cause: unknown): TransformCommandError {
  const message = cause instanceof Error ? cause.message : 'invalid bendpoint index';
  return new TransformCommandError('invalid-bendpoint-index', flowId, message);
}
