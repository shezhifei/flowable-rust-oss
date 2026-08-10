import { describe, expect, it } from 'vitest';

import { createElementCommand, deleteElementsCommand } from './commands';
import { createPaletteElement, type PaletteElementKind } from './elementFactory';
import { useModelerStore } from './modelerStore';
import { sampleDocument } from './sampleDocument';

describe('palette element factory', () => {
  it('creates canonical discriminated elements for every initial palette family', () => {
    const cases: Array<[PaletteElementKind, string]> = [
      ['start', 'startEvent'],
      ['end', 'endEvent'],
      ['userTask', 'userTask'],
      ['exclusiveGateway', 'exclusiveGateway'],
      ['subprocess', 'subProcess'],
      ['data', 'valuedDataObject'],
      ['boundaryTimer', 'boundaryEvent'],
    ];

    for (const [kind, expectedType] of cases) {
      const element = createPaletteElement(kind, `created-${kind}`);
      expect(element.elementType).toBe(expectedType);
      expect(element.id).toBe(`created-${kind}`);
    }
  });

  it('keeps the initial coarse palette aliases compatible', () => {
    expect(createPaletteElement('event', 'legacy-event').elementType).toBe('startEvent');
    expect(createPaletteElement('task', 'legacy-task').elementType).toBe('userTask');
    expect(createPaletteElement('gateway', 'legacy-gateway').elementType).toBe('exclusiveGateway');
  });

  it('creates an interrupting timer boundary event ready for host attachment', () => {
    const element = createPaletteElement('boundaryTimer', 'timer');
    if (element.elementType !== 'boundaryEvent') throw new Error('expected a boundary event');

    expect(element).toMatchObject({
      attachedToRefId: null,
      cancelActivity: true,
      eventDefinitions: [
        {
          eventDefinitionType: 'timerEventDefinition',
          id: 'timerDefinition',
        },
      ],
    });
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

  it('stores a created data object in the canonical data-object collection', () => {
    useModelerStore.getState().setDocument(structuredClone(sampleDocument));
    const element = createPaletteElement('data', 'created-data');
    useModelerStore.getState().execute(
      createElementCommand(element, {
        x: 500,
        y: 260,
        width: 46,
        height: 62,
        rotation: 0,
        expanded: true,
        xmlRowNumber: 0,
        xmlColumnNumber: 0,
      }),
    );

    let state = useModelerStore.getState();
    const process = state.document.model.processes[0];
    expect(process?.dataObjects?.map((dataObject) => dataObject.id)).toContain('created-data');
    expect(process?.flowElements?.map((flowElement) => flowElement.id)).not.toContain(
      'created-data',
    );
    expect(process?.flowElementMap?.['created-data']).toMatchObject({
      elementType: 'valuedDataObject',
      id: 'created-data',
    });
    expect(state.document.model.locationMap['created-data']).toMatchObject({ x: 500, y: 260 });

    state.undo();
    state = useModelerStore.getState();
    expect(state.document.model.processes[0]?.dataObjects).toEqual([]);
    expect(state.document.model.processes[0]?.flowElementMap?.['created-data']).toBeUndefined();
    expect(state.document.model.locationMap['created-data']).toBeUndefined();
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

  it('deletes associations whose endpoint is deleted and restores them on undo', () => {
    useModelerStore.getState().setDocument(structuredClone(sampleDocument));
    useModelerStore.getState().execute(deleteElementsCommand(['decision']));

    let state = useModelerStore.getState();
    expect(
      state.document.model.processes[0]?.artifacts?.map((artifact) => artifact.id),
    ).not.toContain('approvalLink');
    expect(state.document.model.processes[0]?.artifactMap?.approvalLink).toBeUndefined();
    expect(state.document.model.flowLocationMap.approvalLink).toBeUndefined();

    state.undo();
    state = useModelerStore.getState();
    expect(state.document.model.processes[0]?.artifacts?.map((artifact) => artifact.id)).toContain(
      'approvalLink',
    );
    expect(state.document.model.processes[0]?.artifactMap?.approvalLink).toMatchObject({
      artifactType: 'association',
    });
  });
});
