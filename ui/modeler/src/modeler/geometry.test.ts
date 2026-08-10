import { describe, expect, it } from 'vitest';

import {
  alignmentGuideCandidates,
  marqueeElementIds,
  normalizeRect,
  rectContainsPoint,
  rectContainsRect,
  rectsIntersect,
  snapPointToGrid,
  snapToGrid,
} from './geometry';

describe('grid geometry', () => {
  it('snaps numbers and points to the ten-unit grid', () => {
    expect(snapToGrid(14)).toBe(10);
    expect(snapToGrid(15)).toBe(20);
    expect(snapPointToGrid({ x: -16, y: 24 })).toEqual({ x: -20, y: 20 });
    expect(() => snapToGrid(10, 0)).toThrow(RangeError);
  });
});

describe('rectangle geometry', () => {
  it('normalizes reverse drags and includes points on the edge', () => {
    const normalized = normalizeRect({ x: 100, y: 80, width: -60, height: -30 });
    expect(normalized).toEqual({ x: 40, y: 50, width: 60, height: 30 });
    expect(rectContainsPoint(normalized, { x: 100, y: 80 })).toBe(true);
    expect(rectContainsPoint(normalized, { x: 101, y: 80 })).toBe(false);
  });

  it('checks rectangle containment and intersection independently', () => {
    const outer = { x: 0, y: 0, width: 100, height: 100 };
    expect(rectContainsRect(outer, { x: 20, y: 20, width: 40, height: 40 })).toBe(true);
    expect(rectContainsRect(outer, { x: 80, y: 80, width: 40, height: 40 })).toBe(false);
    expect(rectsIntersect(outer, { x: 100, y: 35, width: 20, height: 20 })).toBe(true);
    expect(rectsIntersect(outer, { x: 101, y: 35, width: 20, height: 20 })).toBe(false);
  });

  it('returns deterministic marquee ids for containment or intersection', () => {
    const bounds = {
      zeta: { x: 95, y: 20, width: 20, height: 20 },
      alpha: { x: 10, y: 10, width: 20, height: 20 },
      missing: undefined,
      beta: { x: 50, y: 50, width: 20, height: 20 },
    };
    const marquee = { x: 0, y: 0, width: 100, height: 100 };

    expect(marqueeElementIds(marquee, bounds)).toEqual(['alpha', 'beta', 'zeta']);
    expect(marqueeElementIds(marquee, bounds, 'contains')).toEqual(['alpha', 'beta']);
  });
});

describe('alignment guides', () => {
  it('finds exact and near edge/center candidates ordered by correction distance', () => {
    const guides = alignmentGuideCandidates(
      { x: 98, y: 20, width: 40, height: 40 },
      {
        taskB: { x: 100, y: 100, width: 80, height: 40 },
        taskA: { x: 250, y: 22, width: 40, height: 36 },
      },
      3,
    );

    expect(guides[0]).toMatchObject({
      axis: 'y',
      candidateId: 'taskA',
      candidateEdge: 'center',
      movingEdge: 'center',
      delta: 0,
      value: 40,
    });
    expect(guides).toContainEqual({
      axis: 'x',
      candidateId: 'taskB',
      candidateEdge: 'start',
      movingEdge: 'start',
      delta: 2,
      value: 100,
    });
    expect(guides.every((guide) => Math.abs(guide.delta) <= 3)).toBe(true);
  });
});
