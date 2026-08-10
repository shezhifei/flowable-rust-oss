import { beforeEach, describe, expect, it } from 'vitest';

import { moveElementCommand } from './commands';
import { useModelerStore } from './modelerStore';
import { sampleDocument } from './sampleDocument';

describe('modeler store', () => {
  beforeEach(() => {
    useModelerStore.getState().setDocument(structuredClone(sampleDocument));
    useModelerStore.getState().resetViewport();
    useModelerStore.getState().selectElement(null);
  });

  it('pans, selects, and clamps zoom at both boundaries', () => {
    useModelerStore.getState().panBy(12, -8);
    useModelerStore.getState().selectElement('notify');
    useModelerStore.getState().zoomBy(100);

    expect(useModelerStore.getState().viewport).toEqual({ x: 28, y: 10, zoom: 2.5 });
    expect(useModelerStore.getState().selectedElementId).toBe('notify');

    useModelerStore.getState().zoomBy(0.001);
    expect(useModelerStore.getState().viewport.zoom).toBe(0.35);
  });

  it('undoes and redoes fifty moves without losing node, boundary, or edge geometry', () => {
    const store = useModelerStore.getState();
    for (let index = 0; index < 50; index += 1) {
      store.execute(moveElementCommand('review', 1, 2));
    }

    let state = useModelerStore.getState();
    expect(state.undoStack).toHaveLength(50);
    expect(state.document.model.locationMap.review).toMatchObject({ x: 354, y: 235 });
    expect(state.document.model.locationMap.reviewTimer).toMatchObject({ x: 460, y: 314 });
    expect(required(state.document.model.flowLocationMap.requestFlow).at(-1)).toMatchObject({
      x: 354,
      y: 285,
    });
    expect(required(state.document.model.flowLocationMap.decisionFlow)[0]).toMatchObject({
      x: 510,
      y: 285,
    });

    for (let index = 0; index < 50; index += 1) useModelerStore.getState().undo();
    state = useModelerStore.getState();
    expect(state.undoStack).toHaveLength(0);
    expect(state.redoStack).toHaveLength(50);
    expect(state.document.model.locationMap.review).toMatchObject({ x: 304, y: 135 });
    expect(state.document.model.locationMap.reviewTimer).toMatchObject({ x: 410, y: 214 });
    expect(required(state.document.model.flowLocationMap.requestFlow).at(-1)).toMatchObject({
      x: 304,
      y: 185,
    });

    for (let index = 0; index < 50; index += 1) useModelerStore.getState().redo();
    state = useModelerStore.getState();
    expect(state.undoStack).toHaveLength(50);
    expect(state.redoStack).toHaveLength(0);
    expect(state.document.model.locationMap.review).toMatchObject({ x: 354, y: 235 });
  });

  it('fits every DI bound into the canvas while preserving model coordinates', () => {
    const document = structuredClone(sampleDocument);
    document.model.locationMap.farDataObject = {
      x: 32,
      y: 1068,
      width: 50,
      height: 50,
      rotation: 0,
      expanded: true,
      xmlRowNumber: 0,
      xmlColumnNumber: 0,
    };
    useModelerStore.getState().setDocument(document);
    useModelerStore.getState().fitToModel();

    const { viewport } = useModelerStore.getState();
    expect(viewport.zoom).toBeLessThan(0.6);
    expect(viewport.y + 72 * viewport.zoom).toBeGreaterThanOrEqual(41.9);
    expect(viewport.y + 1118 * viewport.zoom).toBeLessThanOrEqual(578.1);
    expect(required(useModelerStore.getState().document.model.locationMap.farDataObject).y).toBe(
      1068,
    );
  });

  it('copies and pastes the current selection as one reversible history entry', () => {
    useModelerStore.getState().selectElements(['review', 'decision']);
    useModelerStore.getState().copySelection();
    expect(useModelerStore.getState().clipboard?.elements.map((element) => element.id)).toEqual([
      'review',
      'decision',
      'decisionFlow',
    ]);

    useModelerStore.getState().pasteClipboard();
    let state = useModelerStore.getState();
    expect(state.undoStack.at(-1)?.label).toBe('Paste 3 elements');
    expect(state.document.model.processes[0]?.flowElementMap?.['review-copy-1']).toBeDefined();

    state.undo();
    state = useModelerStore.getState();
    expect(state.document.model.processes[0]?.flowElementMap?.['review-copy-1']).toBeUndefined();
  });
});

function required<T>(value: T | undefined): T {
  if (value === undefined) throw new Error('expected fixture value to exist');
  return value;
}
