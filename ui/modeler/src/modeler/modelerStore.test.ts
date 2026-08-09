import { beforeEach, describe, expect, it } from 'vitest';

import { useModelerStore } from './modelerStore';

describe('modeler store viewport', () => {
  beforeEach(() => {
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
});
