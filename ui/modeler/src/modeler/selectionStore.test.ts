import { beforeEach, describe, expect, it } from 'vitest';

import { sampleDocument } from './sampleDocument';
import { useModelerStore } from './modelerStore';

describe('modeler transient selection state', () => {
  beforeEach(() => {
    useModelerStore.getState().setDocument(structuredClone(sampleDocument));
    useModelerStore.getState().setTool('pointer');
  });

  it('supports ordered primary and additive selection without document history', () => {
    const store = useModelerStore.getState();
    store.selectElement('review');
    store.selectElement('approved', true);

    expect(useModelerStore.getState()).toMatchObject({
      selectedElementIds: ['review', 'approved'],
      selectedElementId: 'approved',
      undoStack: [],
    });

    useModelerStore.getState().selectElement('approved', true);
    expect(useModelerStore.getState()).toMatchObject({
      selectedElementIds: ['review'],
      selectedElementId: 'review',
    });
  });

  it('deduplicates marquee selection and resets it when a document loads', () => {
    useModelerStore.getState().selectElements(['notify', 'review', 'notify']);
    expect(useModelerStore.getState().selectedElementIds).toEqual(['notify', 'review']);

    useModelerStore.getState().setDocument(structuredClone(sampleDocument));
    expect(useModelerStore.getState()).toMatchObject({
      selectedElementIds: [],
      selectedElementId: null,
    });
  });

  it('switches editor tools outside undo history', () => {
    useModelerStore.getState().setTool('connect');
    expect(useModelerStore.getState()).toMatchObject({ tool: 'connect', undoStack: [] });
  });
});
