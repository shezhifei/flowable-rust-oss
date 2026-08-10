export const DEFAULT_GRID_SIZE = 10;

export interface Point {
  x: number;
  y: number;
}

export interface Rect extends Point {
  width: number;
  height: number;
}

export type MarqueeSelectionMode = 'contains' | 'intersects';
export type AlignmentAxis = 'x' | 'y';
export type AlignmentEdge = 'start' | 'center' | 'end';

export interface AlignmentGuideCandidate {
  axis: AlignmentAxis;
  candidateId: string;
  candidateEdge: AlignmentEdge;
  delta: number;
  movingEdge: AlignmentEdge;
  value: number;
}

export function snapToGrid(value: number, gridSize = DEFAULT_GRID_SIZE): number {
  requireFinite(value, 'value');
  requirePositive(gridSize, 'gridSize');
  return normalizeZero(Math.round(value / gridSize) * gridSize);
}

export function snapPointToGrid(point: Point, gridSize = DEFAULT_GRID_SIZE): Point {
  return {
    x: snapToGrid(point.x, gridSize),
    y: snapToGrid(point.y, gridSize),
  };
}

/** Normalizes drag rectangles so containment works in every drag direction. */
export function normalizeRect(rect: Rect): Rect {
  const x = rect.width < 0 ? rect.x + rect.width : rect.x;
  const y = rect.height < 0 ? rect.y + rect.height : rect.y;
  return {
    x,
    y,
    width: Math.abs(rect.width),
    height: Math.abs(rect.height),
  };
}

export function rectContainsPoint(rect: Rect, point: Point): boolean {
  const normalized = normalizeRect(rect);
  return (
    point.x >= normalized.x &&
    point.x <= normalized.x + normalized.width &&
    point.y >= normalized.y &&
    point.y <= normalized.y + normalized.height
  );
}

export function rectContainsRect(container: Rect, candidate: Rect): boolean {
  const normalizedCandidate = normalizeRect(candidate);
  return (
    rectContainsPoint(container, normalizedCandidate) &&
    rectContainsPoint(container, {
      x: normalizedCandidate.x + normalizedCandidate.width,
      y: normalizedCandidate.y + normalizedCandidate.height,
    })
  );
}

/** Edge contact counts as intersection, which matches visible SVG selection behavior. */
export function rectsIntersect(left: Rect, right: Rect): boolean {
  const a = normalizeRect(left);
  const b = normalizeRect(right);
  return !(
    a.x + a.width < b.x ||
    b.x + b.width < a.x ||
    a.y + a.height < b.y ||
    b.y + b.height < a.y
  );
}

export function marqueeElementIds(
  marquee: Rect,
  boundsById: Readonly<Record<string, Rect | undefined>>,
  mode: MarqueeSelectionMode = 'intersects',
): string[] {
  const matches = mode === 'contains' ? rectContainsRect : rectsIntersect;
  return Object.entries(boundsById)
    .filter((entry): entry is [string, Rect] => entry[1] !== undefined)
    .filter(([, bounds]) => matches(marquee, bounds))
    .map(([id]) => id)
    .sort((left, right) => left.localeCompare(right));
}

/**
 * Finds every nearby edge/center alignment. Consumers can display all returned
 * guide lines or take the first x/y entry to apply the smallest correction.
 */
export function alignmentGuideCandidates(
  moving: Rect,
  boundsById: Readonly<Record<string, Rect | undefined>>,
  tolerance = 5,
): AlignmentGuideCandidate[] {
  requireNonNegative(tolerance, 'tolerance');
  const movingAnchors = rectAnchors(moving);
  const guides: AlignmentGuideCandidate[] = [];

  for (const [candidateId, bounds] of Object.entries(boundsById)) {
    if (bounds === undefined) continue;
    const candidateAnchors = rectAnchors(bounds);
    collectAlignmentGuides(
      'x',
      candidateId,
      movingAnchors.x,
      candidateAnchors.x,
      tolerance,
      guides,
    );
    collectAlignmentGuides(
      'y',
      candidateId,
      movingAnchors.y,
      candidateAnchors.y,
      tolerance,
      guides,
    );
  }

  return guides.sort(
    (left, right) =>
      Math.abs(left.delta) - Math.abs(right.delta) ||
      left.axis.localeCompare(right.axis) ||
      left.candidateId.localeCompare(right.candidateId) ||
      edgeRank(left.movingEdge) - edgeRank(right.movingEdge) ||
      edgeRank(left.candidateEdge) - edgeRank(right.candidateEdge),
  );
}

type RectAnchors = Record<AlignmentAxis, Record<AlignmentEdge, number>>;

function rectAnchors(rect: Rect): RectAnchors {
  const normalized = normalizeRect(rect);
  return {
    x: {
      start: normalized.x,
      center: normalized.x + normalized.width / 2,
      end: normalized.x + normalized.width,
    },
    y: {
      start: normalized.y,
      center: normalized.y + normalized.height / 2,
      end: normalized.y + normalized.height,
    },
  };
}

function collectAlignmentGuides(
  axis: AlignmentAxis,
  candidateId: string,
  moving: Record<AlignmentEdge, number>,
  candidate: Record<AlignmentEdge, number>,
  tolerance: number,
  output: AlignmentGuideCandidate[],
): void {
  for (const movingEdge of alignmentEdges) {
    for (const candidateEdge of alignmentEdges) {
      const value = candidate[candidateEdge];
      const delta = value - moving[movingEdge];
      if (Math.abs(delta) <= tolerance) {
        output.push({ axis, candidateId, candidateEdge, delta, movingEdge, value });
      }
    }
  }
}

const alignmentEdges: readonly AlignmentEdge[] = ['start', 'center', 'end'];

function edgeRank(edge: AlignmentEdge): number {
  return alignmentEdges.indexOf(edge);
}

function requireFinite(value: number, name: string): void {
  if (!Number.isFinite(value)) throw new RangeError(`${name} must be finite`);
}

function requirePositive(value: number, name: string): void {
  requireFinite(value, name);
  if (value <= 0) throw new RangeError(`${name} must be greater than zero`);
}

function requireNonNegative(value: number, name: string): void {
  requireFinite(value, name);
  if (value < 0) throw new RangeError(`${name} must not be negative`);
}

function normalizeZero(value: number): number {
  return Object.is(value, -0) ? 0 : value;
}
