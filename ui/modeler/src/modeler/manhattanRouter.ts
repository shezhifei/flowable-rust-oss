import type { Point, Rect } from './geometry';

export type AnchorDirection = 'bottom' | 'left' | 'right' | 'top';

export interface Anchor extends Point {
  direction: AnchorDirection;
}

export interface ConnectionAnchors {
  source: Anchor;
  target: Anchor;
}

export function anchorPoint(bounds: Rect, direction: AnchorDirection): Anchor {
  const centerX = bounds.x + bounds.width / 2;
  const centerY = bounds.y + bounds.height / 2;
  switch (direction) {
    case 'top':
      return { x: centerX, y: bounds.y, direction };
    case 'right':
      return { x: bounds.x + bounds.width, y: centerY, direction };
    case 'bottom':
      return { x: centerX, y: bounds.y + bounds.height, direction };
    case 'left':
      return { x: bounds.x, y: centerY, direction };
  }
}

/** Uses the dominant center-to-center axis; exact ties resolve horizontally. */
export function chooseConnectionAnchors(source: Rect, target: Rect): ConnectionAnchors {
  const sourceCenter = center(source);
  const targetCenter = center(target);
  const deltaX = targetCenter.x - sourceCenter.x;
  const deltaY = targetCenter.y - sourceCenter.y;

  if (Math.abs(deltaX) >= Math.abs(deltaY)) {
    return deltaX >= 0
      ? { source: anchorPoint(source, 'right'), target: anchorPoint(target, 'left') }
      : { source: anchorPoint(source, 'left'), target: anchorPoint(target, 'right') };
  }
  return deltaY >= 0
    ? { source: anchorPoint(source, 'bottom'), target: anchorPoint(target, 'top') }
    : { source: anchorPoint(source, 'top'), target: anchorPoint(target, 'bottom') };
}

/**
 * Produces an obstacle-agnostic deterministic orthogonal route. The C2 first
 * iteration intentionally permits crossing unrelated nodes.
 */
export function routeManhattan(source: Rect, target: Rect): Point[] {
  const anchors = chooseConnectionAnchors(source, target);
  const points: Point[] = [pointOf(anchors.source)];

  if (isHorizontal(anchors.source.direction)) {
    const middleX = (anchors.source.x + anchors.target.x) / 2;
    points.push({ x: middleX, y: anchors.source.y }, { x: middleX, y: anchors.target.y });
  } else {
    const middleY = (anchors.source.y + anchors.target.y) / 2;
    points.push({ x: anchors.source.x, y: middleY }, { x: anchors.target.x, y: middleY });
  }
  points.push(pointOf(anchors.target));
  return simplifyOrthogonalPoints(points);
}

/** Inserts a bendpoint after the addressed segment without mutating the input. */
export function insertBendpoint(
  waypoints: readonly Point[],
  segmentIndex: number,
  point: Point,
): Point[] {
  if (!Number.isInteger(segmentIndex) || segmentIndex < 0 || segmentIndex >= waypoints.length - 1) {
    throw new RangeError('segmentIndex must address an existing waypoint segment');
  }
  return [
    ...waypoints.slice(0, segmentIndex + 1).map(copyPoint),
    copyPoint(point),
    ...waypoints.slice(segmentIndex + 1).map(copyPoint),
  ];
}

/** Moves an internal bendpoint; source and target anchors remain route-owned. */
export function moveBendpoint(
  waypoints: readonly Point[],
  bendpointIndex: number,
  point: Point,
): Point[] {
  requireBendpointIndex(waypoints, bendpointIndex);
  return waypoints.map((waypoint, index) =>
    index === bendpointIndex ? copyPoint(point) : copyPoint(waypoint),
  );
}

/** Removes an internal bendpoint; source and target anchors remain route-owned. */
export function removeBendpoint(waypoints: readonly Point[], bendpointIndex: number): Point[] {
  requireBendpointIndex(waypoints, bendpointIndex);
  return waypoints.filter((_, index) => index !== bendpointIndex).map(copyPoint);
}

function simplifyOrthogonalPoints(points: readonly Point[]): Point[] {
  const unique = points.filter(
    (point, index) => index === 0 || !samePoint(point, points[index - 1] as Point),
  );
  return unique.filter((point, index) => {
    if (index === 0 || index === unique.length - 1) return true;
    const previous = unique[index - 1] as Point;
    const next = unique[index + 1] as Point;
    return !(
      (previous.x === point.x && point.x === next.x) ||
      (previous.y === point.y && point.y === next.y)
    );
  });
}

function center(bounds: Rect): Point {
  return { x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2 };
}

function isHorizontal(direction: AnchorDirection): boolean {
  return direction === 'left' || direction === 'right';
}

function pointOf(anchor: Anchor): Point {
  return { x: anchor.x, y: anchor.y };
}

function samePoint(left: Point, right: Point): boolean {
  return left.x === right.x && left.y === right.y;
}

function copyPoint(point: Point): Point {
  return { x: point.x, y: point.y };
}

function requireBendpointIndex(waypoints: readonly Point[], index: number): void {
  if (!Number.isInteger(index) || index <= 0 || index >= waypoints.length - 1) {
    throw new RangeError('bendpointIndex must address an internal waypoint');
  }
}
