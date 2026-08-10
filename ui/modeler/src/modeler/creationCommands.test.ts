import { beforeEach, describe, expect, it } from 'vitest';

import {
  createAtPointCommand,
  moveAndReparentElementsCommand,
  nextPaletteElementId,
  reparentElementCommand,
} from './creationCommands';
import { createPaletteElement } from './elementFactory';
import { locateCanonicalElement } from './modelInvariants';
import { useModelerStore } from './modelerStore';
import { sampleDocument } from './sampleDocument';

describe('ownership-aware creation commands', () => {
  beforeEach(() => {
    useModelerStore.getState().setDocument(structuredClone(sampleDocument));
  });

  it('creates into the deepest subprocess and containing lane with undo and redo', () => {
    useModelerStore.getState().setDocument(documentWithNestedSubprocesses());
    useModelerStore
      .getState()
      .execute(createAtPointCommand('userTask', 'nested-created', { x: 390, y: 190 }));

    let state = useModelerStore.getState();
    let located = locateCanonicalElement(state.document, 'nested-created');
    expect(located).toMatchObject({ kind: 'flowElement', ownerId: 'inner-subprocess' });
    expect(state.document.model.processes[0]?.lanes?.[0]?.flowReferences).toContain(
      'nested-created',
    );
    expect(state.document.model.locationMap['nested-created']).toMatchObject({
      x: 312,
      y: 140,
      width: 156,
      height: 100,
    });

    state.undo();
    state = useModelerStore.getState();
    expect(locateCanonicalElement(state.document, 'nested-created')).toBeNull();
    expect(state.document.model.locationMap['nested-created']).toBeUndefined();

    state.redo();
    state = useModelerStore.getState();
    located = locateCanonicalElement(state.document, 'nested-created');
    expect(located).toMatchObject({ ownerId: 'inner-subprocess' });
  });

  it('creates a nested data object only in the canonical data-object collection', () => {
    useModelerStore.getState().setDocument(documentWithNestedSubprocesses());
    useModelerStore
      .getState()
      .execute(createAtPointCommand('data', 'nested-data', { x: 390, y: 190 }));

    const located = locateCanonicalElement(useModelerStore.getState().document, 'nested-data');
    expect(located).toMatchObject({ kind: 'dataObject', ownerId: 'inner-subprocess' });
    expect(located?.owner.flowElements?.map((element) => element.id)).not.toContain('nested-data');
    expect(located?.owner.flowElementMap?.['nested-data']).toMatchObject({
      elementType: 'valuedDataObject',
    });

    useModelerStore.getState().undo();
    expect(locateCanonicalElement(useModelerStore.getState().document, 'nested-data')).toBeNull();
    useModelerStore.getState().redo();
    expect(
      locateCanonicalElement(useModelerStore.getState().document, 'nested-data'),
    ).toMatchObject({ kind: 'dataObject', ownerId: 'inner-subprocess' });
  });

  it('creates in the process and lane referenced by the containing pool', () => {
    useModelerStore.getState().setDocument(documentWithSecondProcess());
    useModelerStore
      .getState()
      .execute(createAtPointCommand('end', 'second-end', { x: 1510, y: 170 }));

    const state = useModelerStore.getState();
    expect(locateCanonicalElement(state.document, 'second-end')).toMatchObject({
      ownerId: 'second-process',
    });
    expect(state.document.model.processes[1]?.lanes?.[0]?.flowReferences).toContain('second-end');
    expect(state.document.model.processes[0]?.flowElementMap?.['second-end']).toBeUndefined();
  });

  it('allocates collision-free canonical palette ids', () => {
    useModelerStore
      .getState()
      .execute(createAtPointCommand('start', 'modeler-start-1', { x: 240, y: 180 }));

    expect(nextPaletteElementId(useModelerStore.getState().document, 'start')).toBe(
      'modeler-start-2',
    );
  });

  it('attaches a timer boundary to the nearest activity border and host lane', () => {
    useModelerStore
      .getState()
      .execute(createAtPointCommand('boundaryTimer', 'review-boundary', { x: 464, y: 185 }));

    let state = useModelerStore.getState();
    const boundary = locateCanonicalElement(state.document, 'review-boundary');
    expect(boundary).toMatchObject({
      kind: 'flowElement',
      ownerId: 'leaveProcess',
      element: { attachedToRefId: 'review', elementType: 'boundaryEvent' },
    });
    expect(state.document.model.locationMap['review-boundary']).toMatchObject({ x: 443, y: 168 });
    expect(state.document.model.processes[0]?.lanes?.[0]?.flowReferences).toContain(
      'review-boundary',
    );
    const review = state.document.model.processes[0]?.flowElements?.find(
      (element) => element.id === 'review',
    );
    if (!review || review.elementType !== 'userTask') throw new Error('expected review task');
    expect(review.boundaryEvents?.map((event) => event.id)).toContain('review-boundary');

    state.undo();
    expect(
      locateCanonicalElement(useModelerStore.getState().document, 'review-boundary'),
    ).toBeNull();
    state.redo();
    state = useModelerStore.getState();
    expect(locateCanonicalElement(state.document, 'review-boundary')).toMatchObject({
      element: { attachedToRefId: 'review' },
    });
  });

  it('reparents an unconnected element across subprocess ownership and lane membership', () => {
    const document = documentWithNestedSubprocesses();
    const process = required(document.model.processes[0]);
    process.flowElements?.push(createPaletteElement('userTask', 'loose-task'));
    document.model.locationMap['loose-task'] = bounds(900, 350, 156, 100);
    useModelerStore.getState().setDocument(document);
    useModelerStore.getState().execute(reparentElementCommand('loose-task', { x: 390, y: 190 }));

    let state = useModelerStore.getState();
    expect(locateCanonicalElement(state.document, 'loose-task')).toMatchObject({
      ownerId: 'inner-subprocess',
    });
    expect(state.document.model.processes[0]?.lanes?.[0]?.flowReferences).toContain('loose-task');

    state.undo();
    state = useModelerStore.getState();
    expect(locateCanonicalElement(state.document, 'loose-task')).toMatchObject({
      ownerId: 'leaveProcess',
    });
    state.redo();
    expect(locateCanonicalElement(useModelerStore.getState().document, 'loose-task')).toMatchObject(
      {
        ownerId: 'inner-subprocess',
      },
    );
  });

  it('moves a connected node between lanes without changing its canonical owner or flows', () => {
    useModelerStore.getState().execute(reparentElementCommand('review', { x: 700, y: 400 }));

    let state = useModelerStore.getState();
    expect(locateCanonicalElement(state.document, 'review')).toMatchObject({
      ownerId: 'leaveProcess',
    });
    expect(state.document.model.processes[0]?.lanes?.[0]?.flowReferences).not.toContain('review');
    expect(state.document.model.processes[0]?.lanes?.[1]?.flowReferences).toContain('review');
    expect(
      state.document.model.processes[0]?.flowElements?.filter(
        (element) =>
          element.elementType === 'sequenceFlow' &&
          (element.sourceRef === 'review' || element.targetRef === 'review'),
      ),
    ).toHaveLength(2);

    state.undo();
    state = useModelerStore.getState();
    expect(state.document.model.processes[0]?.lanes?.[0]?.flowReferences).toContain('review');
    expect(state.document.model.processes[0]?.lanes?.[1]?.flowReferences).not.toContain('review');
    state.redo();
    expect(
      useModelerStore.getState().document.model.processes[0]?.lanes?.[1]?.flowReferences,
    ).toContain('review');
  });

  it('moves and reparents in one history entry using the final snapped center', () => {
    const document = structuredClone(sampleDocument);
    const process = required(document.model.processes[0]);
    process.flowElements?.push(createPaletteElement('userTask', 'loose-task'));
    document.model.locationMap['loose-task'] = bounds(300, 150, 156, 100);
    useModelerStore.getState().setDocument(document);

    useModelerStore
      .getState()
      .execute(moveAndReparentElementsCommand(['loose-task'], 400, 200));

    let state = useModelerStore.getState();
    expect(state.undoStack).toHaveLength(1);
    expect(state.document.model.locationMap['loose-task']).toMatchObject({ x: 700, y: 350 });
    expect(state.document.model.processes[0]?.lanes?.[1]?.flowReferences).toContain('loose-task');

    state.undo();
    state = useModelerStore.getState();
    expect(state.document.model.locationMap['loose-task']).toMatchObject({ x: 300, y: 150 });
    expect(state.document.model.processes[0]?.lanes?.[1]?.flowReferences).not.toContain(
      'loose-task',
    );
  });

  it('rejects a move whose final center escapes every positioned pool', () => {
    useModelerStore
      .getState()
      .execute(moveAndReparentElementsCommand(['review'], -1000, -1000));

    const state = useModelerStore.getState();
    expect(state.undoStack).toHaveLength(0);
    expect(state.document.model.locationMap.review).toMatchObject({ x: 304, y: 135 });
  });

  it('refuses a cross-process reparent that would strand connected sequence flows', () => {
    const document = documentWithSecondProcess();
    useModelerStore.getState().setDocument(document);
    useModelerStore.getState().execute(reparentElementCommand('review', { x: 1510, y: 170 }));

    const state = useModelerStore.getState();
    expect(locateCanonicalElement(state.document, 'review')).toMatchObject({
      ownerId: 'leaveProcess',
    });
    expect(state.undoStack).toHaveLength(0);
    expect(
      state.document.model.processes[0]?.flowElements?.filter(
        (element) =>
          element.elementType === 'sequenceFlow' &&
          (element.sourceRef === 'review' || element.targetRef === 'review'),
      ),
    ).toHaveLength(2);
  });

  it('refuses to reparent a subprocess into itself', () => {
    useModelerStore.getState().setDocument(documentWithNestedSubprocesses());
    useModelerStore
      .getState()
      .execute(reparentElementCommand('outer-subprocess', { x: 280, y: 115 }));

    const state = useModelerStore.getState();
    expect(locateCanonicalElement(state.document, 'outer-subprocess')).toMatchObject({
      ownerId: 'leaveProcess',
    });
    expect(state.undoStack).toHaveLength(0);
  });
});

function bounds(x: number, y: number, width: number, height: number) {
  return {
    x,
    y,
    width,
    height,
    rotation: 0,
    expanded: true,
    xmlRowNumber: 0,
    xmlColumnNumber: 0,
  };
}

function documentWithNestedSubprocesses() {
  const document = structuredClone(sampleDocument);
  const process = required(document.model.processes[0]);
  const outer = createPaletteElement('subprocess', 'outer-subprocess');
  const inner = createPaletteElement('subprocess', 'inner-subprocess');
  if (outer.elementType !== 'subProcess' || inner.elementType !== 'subProcess') {
    throw new Error('expected subprocess fixtures');
  }
  outer.flowElements = [inner];
  outer.flowElementMap = { 'inner-subprocess': inner };
  process.flowElements?.push(outer);
  document.model.locationMap['outer-subprocess'] = bounds(250, 100, 420, 250);
  document.model.locationMap['inner-subprocess'] = bounds(320, 130, 240, 150);
  return document;
}

function documentWithSecondProcess() {
  const document = structuredClone(sampleDocument);
  const first = required(document.model.processes[0]);
  const second = structuredClone(first);
  second.id = 'second-process';
  second.name = 'Second process';
  second.flowElements = [];
  second.flowElementMap = {};
  second.dataObjects = [];
  second.artifacts = [];
  second.artifactMap = {};
  second.lanes = [
    {
      id: 'second-lane',
      name: 'Second lane',
      flowReferences: [],
      attributes: {},
      extensionElements: {},
      xmlRowNumber: 0,
      xmlColumnNumber: 0,
    },
  ];
  document.model.processes.push(second);
  const pool = structuredClone(required(document.model.pools[0]));
  pool.id = 'second-pool';
  pool.name = 'Second pool';
  pool.processRef = 'second-process';
  document.model.pools.push(pool);
  document.model.locationMap['second-pool'] = bounds(1400, 72, 600, 300);
  document.model.locationMap['second-lane'] = bounds(1440, 72, 560, 300);
  return document;
}

function required<T>(value: T | undefined): T {
  if (value === undefined) throw new Error('expected fixture value to exist');
  return value;
}
