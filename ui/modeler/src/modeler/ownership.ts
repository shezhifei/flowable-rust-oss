import type { Draft } from 'immer';

import type {
  BpmnEditorDocument,
  FlowElementEnum,
  GraphicInfo,
  Process,
} from '../generated/editor-protocol';
import { normalizeRect, rectContainsPoint, type Point, type Rect } from './geometry';
import type { CanonicalOwner } from './modelInvariants';

export interface DropOwnerResolution {
  laneId: string | null;
  owner: CanonicalOwner;
  ownerId: string | null;
  process: Draft<Process>;
  processId: string | null;
}

export interface BoundaryElementSize {
  height: number;
  width: number;
}

export interface BoundaryAttachmentResolution extends DropOwnerResolution {
  bounds: GraphicInfo;
  hostId: string;
}

interface OwnedActivity {
  element: Draft<FlowElementEnum>;
  owner: CanonicalOwner;
}

export function resolveDropOwner(
  document: Draft<BpmnEditorDocument>,
  point: Point,
): DropOwnerResolution | null {
  const process = resolveProcessAtPoint(document, point);
  if (!process) return null;
  const owner = deepestSubprocessOwner(document, process, point);
  return {
    laneId: laneAtPoint(document, process, point),
    owner,
    ownerId: owner.id ?? null,
    process,
    processId: process.id ?? null,
  };
}

export function resolveBoundaryAttachment(
  document: Draft<BpmnEditorDocument>,
  processId: string,
  point: Point,
  size: BoundaryElementSize,
): BoundaryAttachmentResolution | null {
  const process = document.model.processes.find((candidate) => candidate.id === processId);
  if (!process || size.width <= 0 || size.height <= 0) return null;

  const candidates = collectOwnedActivities(process, process.flowElements ?? [])
    .flatMap((candidate) => {
      if (!candidate.element.id) return [];
      const bounds = document.model.locationMap[candidate.element.id];
      if (!bounds) return [];
      const anchor = nearestBorderPoint(bounds, point);
      return [{ ...candidate, anchor, distance: squaredDistance(anchor, point) }];
    })
    .sort(
      (left, right) =>
        left.distance - right.distance ||
        (left.element.id ?? '').localeCompare(right.element.id ?? ''),
    );
  const nearest = candidates[0];
  if (!nearest?.element.id) return null;

  return {
    bounds: graphicBounds(
      nearest.anchor.x - size.width / 2,
      nearest.anchor.y - size.height / 2,
      size.width,
      size.height,
    ),
    hostId: nearest.element.id,
    laneId: laneForElement(process, nearest.element.id) ?? laneAtPoint(document, process, point),
    owner: nearest.owner,
    ownerId: nearest.owner.id ?? null,
    process,
    processId: process.id ?? null,
  };
}

export function processForOwner(
  document: Draft<BpmnEditorDocument>,
  owner: CanonicalOwner,
): Draft<Process> | null {
  return (
    document.model.processes.find(
      (process) => process === owner || ownerExistsBelow(process.flowElements ?? [], owner),
    ) ?? null
  );
}

export function laneForElement(process: Draft<Process>, elementId: string): string | null {
  return process.lanes?.find((lane) => lane.flowReferences.includes(elementId))?.id ?? null;
}

function resolveProcessAtPoint(
  document: Draft<BpmnEditorDocument>,
  point: Point,
): Draft<Process> | null {
  const processById = new Map(
    document.model.processes.flatMap((process) =>
      process.id ? [[process.id, process] as const] : [],
    ),
  );
  const poolMatches = document.model.pools
    .flatMap((pool) => {
      if (!pool.id || !pool.processRef) return [];
      const bounds = document.model.locationMap[pool.id];
      const process = processById.get(pool.processRef);
      return bounds && process && rectContainsPoint(bounds, point)
        ? [{ area: bounds.width * bounds.height, process }]
        : [];
    })
    .sort((left, right) => left.area - right.area);
  if (poolMatches[0]) return poolMatches[0].process;

  const hasPositionedProcessPool = document.model.pools.some((pool) =>
    Boolean(pool.processRef && pool.id && document.model.locationMap[pool.id]),
  );
  if (hasPositionedProcessPool) return null;

  if (document.model.processes.length === 1) return document.model.processes[0] ?? null;
  const processWithContainedSubprocess = document.model.processes.find(
    (process) => deepestSubprocessOwner(document, process, point) !== process,
  );
  return processWithContainedSubprocess ?? null;
}

function deepestSubprocessOwner(
  document: Draft<BpmnEditorDocument>,
  process: Draft<Process>,
  point: Point,
): CanonicalOwner {
  return deepestNestedOwner(document, process, point) ?? process;
}

function deepestNestedOwner(
  document: Draft<BpmnEditorDocument>,
  owner: CanonicalOwner,
  point: Point,
): CanonicalOwner | null {
  const containing = (owner.flowElements ?? [])
    .flatMap((element) => {
      if (!isSubprocessOwner(element) || !element.id) return [];
      const bounds = document.model.locationMap[element.id];
      return bounds && bounds.expanded !== false && rectContainsPoint(bounds, point)
        ? [{ area: bounds.width * bounds.height, owner: element }]
        : [];
    })
    .sort(
      (left, right) =>
        left.area - right.area || ownerId(left.owner).localeCompare(ownerId(right.owner)),
    );
  const closest = containing[0]?.owner;
  if (!closest) return null;
  return deepestNestedOwner(document, closest, point) ?? closest;
}

function laneAtPoint(
  document: Draft<BpmnEditorDocument>,
  process: Draft<Process>,
  point: Point,
): string | null {
  const matches = (process.lanes ?? [])
    .flatMap((lane) => {
      if (!lane.id) return [];
      const bounds = document.model.locationMap[lane.id];
      return bounds && rectContainsPoint(bounds, point)
        ? [{ area: bounds.width * bounds.height, id: lane.id }]
        : [];
    })
    .sort((left, right) => left.area - right.area || left.id.localeCompare(right.id));
  return matches[0]?.id ?? null;
}

function collectOwnedActivities(
  owner: CanonicalOwner,
  elements: Draft<FlowElementEnum>[],
): OwnedActivity[] {
  return elements.flatMap((element) => {
    const activity = isBoundaryHostActivity(element) ? [{ element, owner }] : [];
    return isSubprocessOwner(element)
      ? [...activity, ...collectOwnedActivities(element, element.flowElements ?? [])]
      : activity;
  });
}

function isBoundaryHostActivity(element: Draft<FlowElementEnum>) {
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
    case 'adhocSubProcess':
    case 'callActivity':
      return true;
    default:
      return false;
  }
}

function isSubprocessOwner(element: Draft<FlowElementEnum>): element is Draft<
  Extract<
    FlowElementEnum,
    {
      elementType: 'subProcess' | 'transaction' | 'eventSubProcess' | 'adhocSubProcess';
    }
  >
> {
  return (
    element.elementType === 'subProcess' ||
    element.elementType === 'transaction' ||
    element.elementType === 'eventSubProcess' ||
    element.elementType === 'adhocSubProcess'
  );
}

function ownerExistsBelow(elements: Draft<FlowElementEnum>[], owner: CanonicalOwner): boolean {
  return elements.some(
    (element) =>
      isSubprocessOwner(element) &&
      (element === owner || ownerExistsBelow(element.flowElements ?? [], owner)),
  );
}

function nearestBorderPoint(bounds: Rect, point: Point): Point {
  const rect = normalizeRect(bounds);
  const maxX = rect.x + rect.width;
  const maxY = rect.y + rect.height;
  const x = Math.min(maxX, Math.max(rect.x, point.x));
  const y = Math.min(maxY, Math.max(rect.y, point.y));
  const outside = point.x < rect.x || point.x > maxX || point.y < rect.y || point.y > maxY;
  if (outside) return { x, y };

  const sides = [
    { distance: Math.abs(point.x - rect.x), point: { x: rect.x, y } },
    { distance: Math.abs(maxX - point.x), point: { x: maxX, y } },
    { distance: Math.abs(point.y - rect.y), point: { x, y: rect.y } },
    { distance: Math.abs(maxY - point.y), point: { x, y: maxY } },
  ];
  sides.sort((left, right) => left.distance - right.distance);
  return sides[0]?.point ?? { x, y };
}

function squaredDistance(left: Point, right: Point) {
  return (left.x - right.x) ** 2 + (left.y - right.y) ** 2;
}

function graphicBounds(x: number, y: number, width: number, height: number): GraphicInfo {
  return {
    x,
    y,
    width,
    height,
    rotation: 0,
    expanded: true,
    xmlRowNumber: 0,
    xmlColumnNumber: 0,
  };
}

function ownerId(owner: CanonicalOwner) {
  return owner.id ?? '';
}
