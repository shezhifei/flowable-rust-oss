import { describe, expect, it } from 'vitest';

import {
  anchorPoint,
  chooseConnectionAnchors,
  insertBendpoint,
  moveBendpoint,
  removeBendpoint,
  routeManhattan,
} from './manhattanRouter';

describe('Manhattan anchors and routing', () => {
  it('provides all four centered rectangle anchors', () => {
    const bounds = { x: 20, y: 40, width: 100, height: 60 };
    expect(anchorPoint(bounds, 'top')).toEqual({ x: 70, y: 40, direction: 'top' });
    expect(anchorPoint(bounds, 'right')).toEqual({ x: 120, y: 70, direction: 'right' });
    expect(anchorPoint(bounds, 'bottom')).toEqual({ x: 70, y: 100, direction: 'bottom' });
    expect(anchorPoint(bounds, 'left')).toEqual({ x: 20, y: 70, direction: 'left' });
  });

  it('chooses opposing horizontal anchors and a deterministic orthogonal route', () => {
    const source = { x: 0, y: 0, width: 100, height: 80 };
    const target = { x: 300, y: 100, width: 100, height: 80 };

    expect(chooseConnectionAnchors(source, target)).toEqual({
      source: { x: 100, y: 40, direction: 'right' },
      target: { x: 300, y: 140, direction: 'left' },
    });
    expect(routeManhattan(source, target)).toEqual([
      { x: 100, y: 40 },
      { x: 200, y: 40 },
      { x: 200, y: 140 },
      { x: 300, y: 140 },
    ]);
  });

  it('chooses opposing vertical anchors and removes redundant collinear points', () => {
    const source = { x: 100, y: 240, width: 80, height: 60 };
    const target = { x: 100, y: 20, width: 80, height: 60 };

    expect(chooseConnectionAnchors(source, target)).toEqual({
      source: { x: 140, y: 240, direction: 'top' },
      target: { x: 140, y: 80, direction: 'bottom' },
    });
    expect(routeManhattan(source, target)).toEqual([
      { x: 140, y: 240 },
      { x: 140, y: 80 },
    ]);
  });

  it('resolves an exact diagonal tie horizontally', () => {
    const anchors = chooseConnectionAnchors(
      { x: 0, y: 0, width: 40, height: 40 },
      { x: 100, y: 100, width: 40, height: 40 },
    );
    expect(anchors.source.direction).toBe('right');
    expect(anchors.target.direction).toBe('left');
  });
});

describe('bendpoint operations', () => {
  const original = [
    { x: 0, y: 0 },
    { x: 50, y: 0 },
    { x: 50, y: 100 },
    { x: 100, y: 100 },
  ];

  it('inserts, moves, and removes bendpoints immutably', () => {
    const inserted = insertBendpoint(original, 0, { x: 20, y: 0 });
    expect(inserted).toEqual([
      { x: 0, y: 0 },
      { x: 20, y: 0 },
      { x: 50, y: 0 },
      { x: 50, y: 100 },
      { x: 100, y: 100 },
    ]);

    const moved = moveBendpoint(original, 1, { x: 60, y: 10 });
    expect(moved[1]).toEqual({ x: 60, y: 10 });
    expect(removeBendpoint(moved, 1)).toEqual([
      { x: 0, y: 0 },
      { x: 50, y: 100 },
      { x: 100, y: 100 },
    ]);
    expect(original[1]).toEqual({ x: 50, y: 0 });
    expect(moved[0]).not.toBe(original[0]);
  });

  it('protects route anchors and rejects missing segments', () => {
    expect(() => moveBendpoint(original, 0, { x: 1, y: 1 })).toThrow(RangeError);
    expect(() => removeBendpoint(original, original.length - 1)).toThrow(RangeError);
    expect(() => insertBendpoint(original, original.length - 1, { x: 1, y: 1 })).toThrow(
      RangeError,
    );
  });
});
