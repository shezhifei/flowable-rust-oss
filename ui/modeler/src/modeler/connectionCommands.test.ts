import { beforeEach, describe, expect, it } from 'vitest';

import {
  createSequenceFlowCommand,
  nextSequenceFlowId,
  reconnectSequenceFlowCommand,
  sequenceFlowEligibility,
  setSequenceFlowConditionCommand,
} from './connectionCommands';
import { createElementCommand } from './commands';
import { createPaletteElement } from './elementFactory';
import type { FlowElementEnum } from '../generated/editor-protocol';
import { useModelerStore } from './modelerStore';
import { sampleDocument } from './sampleDocument';

describe('canonical sequence-flow commands', () => {
  beforeEach(() => {
    useModelerStore.getState().setDocument(structuredClone(sampleDocument));
  });

  it('creates a routed flow and every canonical mirror in one undo entry', () => {
    createNode('task', 'second-review', 780, 210);
    const store = useModelerStore.getState();
    store.execute(createSequenceFlowCommand('new-flow', 'notify', 'second-review'));

    let state = useModelerStore.getState();
    const process = state.document.model.processes[0];
    expect(process?.flowElements?.find((element) => element.id === 'new-flow')).toMatchObject({
      elementType: 'sequenceFlow',
      sourceRef: 'notify',
      targetRef: 'second-review',
    });
    expect(process?.flowElementMap?.['new-flow']).toMatchObject({ elementType: 'sequenceFlow' });
    expect(outgoingIds(process?.flowElementMap?.notify)).toContain('new-flow');
    expect(incomingIds(process?.flowElementMap?.['second-review'])).toContain('new-flow');
    expect(state.document.model.flowLocationMap['new-flow']!.length).toBeGreaterThanOrEqual(2);
    expect(state.document.model.edgeMap['new-flow']?.waypoints).toEqual(
      state.document.model.flowLocationMap['new-flow'],
    );

    state.undo();
    state = useModelerStore.getState();
    expect(state.document.model.processes[0]?.flowElementMap?.['new-flow']).toBeUndefined();
    state.redo();
    expect(
      useModelerStore.getState().document.model.processes[0]?.flowElementMap?.['new-flow'],
    ).toBeDefined();
  });

  it('rejects illegal event directions and missing DI without history', () => {
    const document = useModelerStore.getState().document;
    expect(sequenceFlowEligibility(document, 'notify', 'start')).toEqual({
      valid: false,
      reason: 'start-event-has-no-incoming',
    });
    expect(sequenceFlowEligibility(document, 'notify', 'reviewTimer')).toEqual({
      valid: false,
      reason: 'boundary-event-has-no-incoming',
    });

    useModelerStore
      .getState()
      .execute(createSequenceFlowCommand('illegal-flow', 'notify', 'start'));
    expect(useModelerStore.getState().undoStack).toHaveLength(0);
  });

  it('reconnects and edits conditions while keeping mirrors reversible', () => {
    useModelerStore
      .getState()
      .execute(reconnectSequenceFlowCommand('approvedFlow', 'decision', 'end'));
    useModelerStore
      .getState()
      .execute(setSequenceFlowConditionCommand('approvedFlow', '${approved == true}', 'UEL'));

    let state = useModelerStore.getState();
    expect(state.document.model.processes[0]?.flowElementMap?.approvedFlow).toMatchObject({
      sourceRef: 'decision',
      targetRef: 'end',
      conditionExpression: '${approved == true}',
      conditionLanguage: 'UEL',
    });
    expect(incomingIds(state.document.model.processes[0]?.flowElementMap?.end)).toContain(
      'approvedFlow',
    );

    state.undo();
    expect(
      useModelerStore.getState().document.model.processes[0]?.flowElementMap?.approvedFlow,
    ).toMatchObject({ conditionExpression: '${approved}' });
    state = useModelerStore.getState();
    state.undo();
    expect(
      useModelerStore.getState().document.model.processes[0]?.flowElementMap?.approvedFlow,
    ).toMatchObject({ targetRef: 'notify' });
  });

  it('allocates stable collision-free flow ids', () => {
    expect(nextSequenceFlowId(useModelerStore.getState().document)).toBe('modeler-flow-1');
  });
});

function createNode(kind: 'task', id: string, x: number, y: number) {
  useModelerStore.getState().execute(
    createElementCommand(createPaletteElement(kind, id), {
      x,
      y,
      width: 156,
      height: 100,
      rotation: 0,
      expanded: true,
      xmlRowNumber: 0,
      xmlColumnNumber: 0,
    }),
  );
  // Isolate the flow command history assertion from setup.
  useModelerStore.setState({ undoStack: [], redoStack: [] });
}

function incomingIds(element: FlowElementEnum | undefined) {
  if (!element) return [];
  switch (element.elementType) {
    case 'sequenceFlow':
    case 'valuedDataObject':
      return [];
    default:
      return element.incomingFlows.map((flow) => flow.id);
  }
}

function outgoingIds(element: FlowElementEnum | undefined) {
  if (!element) return [];
  switch (element.elementType) {
    case 'sequenceFlow':
    case 'valuedDataObject':
      return [];
    default:
      return element.outgoingFlows.map((flow) => flow.id);
  }
}
