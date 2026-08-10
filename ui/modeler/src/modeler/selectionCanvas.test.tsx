import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { marqueeElementIds } from './geometry';
import { BpmnCanvas, canvasSelectionGeometry } from './BpmnCanvas';
import { sampleDocument } from './sampleDocument';

const {
  clientDeltaToCanvas,
  clientDeltaToModel,
  clientPointToModel,
  isAdditiveSelection,
  isPanGesture,
  selectableElementBounds,
} = canvasSelectionGeometry;

describe('canvas selection coordinates', () => {
  it('maps CSS client coordinates through the SVG viewBox and viewport transform', () => {
    const canvas = { left: 100, top: 50, width: 700, height: 310 };
    expect(clientPointToModel({ x: 450, y: 205 }, canvas, { x: 20, y: 10, zoom: 2 })).toEqual({
      x: 340,
      y: 150,
    });
    expect(clientDeltaToCanvas({ x: 35, y: 31 }, canvas)).toEqual({ x: 70, y: 62 });
    expect(clientDeltaToModel({ x: 35, y: 31 }, canvas, 2)).toEqual({ x: 35, y: 31 });
  });

  it('derives marquee bounds for shapes and waypoint-only connections', () => {
    const bounds = selectableElementBounds(sampleDocument);
    expect(bounds.start).toEqual({ x: 172, y: 164, width: 42, height: 42 });
    expect(bounds.requestFlow).toEqual({ x: 214, y: 185, width: 90, height: 0 });
    expect(
      marqueeElementIds({ x: 165, y: 155, width: 55, height: 60 }, bounds, 'contains'),
    ).toContain('start');
  });

  it('recognizes platform additive modifiers and pan gestures', () => {
    expect(isAdditiveSelection({ ctrlKey: true, metaKey: false })).toBe(true);
    expect(isAdditiveSelection({ ctrlKey: false, metaKey: true })).toBe(true);
    expect(isAdditiveSelection({ ctrlKey: false, metaKey: false })).toBe(false);
    expect(isPanGesture('hand', 0)).toBe(true);
    expect(isPanGesture('pointer', 1)).toBe(true);
    expect(isPanGesture('pointer', 0)).toBe(false);
    expect(isPanGesture('connect', 0)).toBe(false);
  });
});

describe('canvas selectable SVG surface', () => {
  let document = structuredClone(sampleDocument);

  beforeEach(() => {
    document = structuredClone(sampleDocument);
    document.model.dataStores.auditStore = {
      id: 'auditStore',
      name: 'Audit store',
      attributes: {},
      extensionElements: {},
      dataState: null,
      itemSubjectRef: null,
      xmlColumnNumber: 0,
      xmlRowNumber: 0,
    };
    document.model.locationMap.auditStore = {
      x: 940,
      y: 100,
      width: 56,
      height: 62,
      rotation: 0,
      expanded: true,
      xmlColumnNumber: 0,
      xmlRowNumber: 0,
    };
  });

  it('marks every selectable renderer family with ids and selected classes', () => {
    const selectedElementIds = [
      'review',
      'requestFlow',
      'approvalLink',
      'approvalNote',
      'approvalGroup',
      'auditStore',
      'leavePool',
      'managerLane',
    ];
    const html = renderToStaticMarkup(
      <BpmnCanvas
        renderState={{
          document,
          selectedElementIds,
          tool: 'pointer',
          viewport: { x: 16, y: 18, zoom: 0.82 },
        }}
      />,
    );
    for (const id of selectedElementIds) {
      expect(html).toContain(`data-element-id="${id}"`);
    }
    expect(html).toContain('sequence-flow is-selected');
    expect(html).toContain('association-flow is-selected');
    expect(html).toContain('text-annotation is-selected');
    expect(html).toContain('group-shape is-selected');
    expect(html).toContain('data-store-shape is-selected');
    expect(html).toContain('pool-shape is-selected');
    expect(html).toContain('lane-shape is-selected');
    expect(html).toContain('diagram-element element-userTask is-selected');
  });
});
