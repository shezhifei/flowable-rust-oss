import { describe, expect, it } from 'vitest';

import { createElementCommand, deleteElementsCommand } from './commands';
import { createPaletteElement, type PaletteElementKind } from './elementFactory';
import { useModelerStore } from './modelerStore';
import { sampleDocument } from './sampleDocument';

describe('palette element factory', () => {
  it('creates canonical discriminated elements for every initial palette family', () => {
    const cases: Array<[PaletteElementKind, string]> = [
      ['event', 'startEvent'],
      ['task', 'userTask'],
      ['gateway', 'exclusiveGateway'],
      ['subprocess', 'subProcess'],
      ['data', 'valuedDataObject'],
    ];

    for (const [kind, expectedType] of cases) {
      const element = createPaletteElement(kind, `created-${kind}`);
      expect(element.elementType).toBe(expectedType);
      expect(element.id).toBe(`created-${kind}`);
    }
  });

  it('adds flow/list/map/DI state atomically and removes it on undo', () => {
    useModelerStore.getState().setDocument(structuredClone(sampleDocument));
    const element = createPaletteElement('task', 'created-task');
    useModelerStore.getState().execute(
      createElementCommand(element, {
        x: 500,
        y: 260,
        width: 156,
        height: 100,
        rotation: 0,
        expanded: true,
        xmlRowNumber: 0,
        xmlColumnNumber: 0,
      }),
    );

    let state = useModelerStore.getState();
    expect(state.undoStack).toHaveLength(1);
    expect(state.document.model.processes[0]?.flowElementMap?.['created-task']).toMatchObject({
      elementType: 'userTask',
    });
    expect(state.document.model.locationMap['created-task']).toMatchObject({ x: 500, y: 260 });

    state.undo();
    state = useModelerStore.getState();
    expect(state.document.model.processes[0]?.flowElementMap?.['created-task']).toBeUndefined();
    expect(state.document.model.locationMap['created-task']).toBeUndefined();
  });

  it('deletes connected flows and attached boundary geometry in one reversible command', () => {
    useModelerStore.getState().setDocument(structuredClone(sampleDocument));
    useModelerStore.getState().execute(deleteElementsCommand(['review']));

    let state = useModelerStore.getState();
    const ids = state.document.model.processes[0]?.flowElements?.map((element) => element.id);
    expect(ids).not.toContain('review');
    expect(ids).not.toContain('reviewTimer');
    expect(ids).not.toContain('requestFlow');
    expect(ids).not.toContain('decisionFlow');
    expect(state.document.model.locationMap.review).toBeUndefined();
    expect(state.document.model.flowLocationMap.requestFlow).toBeUndefined();

    state.undo();
    state = useModelerStore.getState();
    const restoredIds = state.document.model.processes[0]?.flowElements?.map(
      (element) => element.id,
    );
    expect(restoredIds).toContain('review');
    expect(restoredIds).toContain('reviewTimer');
    expect(restoredIds).toContain('requestFlow');
    expect(restoredIds).toContain('decisionFlow');
  });
});
