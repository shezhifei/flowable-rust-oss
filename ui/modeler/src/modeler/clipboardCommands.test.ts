import { beforeEach, describe, expect, it } from 'vitest';

import { copySelection, pasteClipboardCommand } from './clipboardCommands';
import { createElementCommand } from './commands';
import { createPaletteElement } from './elementFactory';
import { useModelerStore } from './modelerStore';
import { sampleDocument } from './sampleDocument';

describe('canonical BPMN clipboard', () => {
  beforeEach(() => {
    useModelerStore.getState().setDocument(structuredClone(sampleDocument));
  });

  it('copies selected nodes and their internal flow with remapped references and DI', () => {
    const clipboard = copySelection(useModelerStore.getState().document, ['review', 'decision']);
    expect(clipboard).not.toBeNull();
    if (!clipboard) return;
    expect(clipboard.elements.map((element) => element.id)).toEqual([
      'review',
      'decision',
      'decisionFlow',
    ]);

    useModelerStore.getState().execute(pasteClipboardCommand(clipboard));
    let state = useModelerStore.getState();
    const process = state.document.model.processes[0];
    expect(process?.flowElementMap?.['review-copy-1']).toMatchObject({
      elementType: 'userTask',
    });
    expect(process?.flowElementMap?.['decision-copy-1']).toMatchObject({
      elementType: 'complexGateway',
    });
    expect(process?.flowElementMap?.['decisionFlow-copy-1']).toMatchObject({
      elementType: 'sequenceFlow',
      sourceRef: 'review-copy-1',
      targetRef: 'decision-copy-1',
    });
    expect(state.document.model.locationMap['review-copy-1']).toMatchObject({
      x: state.document.model.locationMap.review!.x + 24,
      y: state.document.model.locationMap.review!.y + 24,
    });
    expect(state.document.model.flowLocationMap['decisionFlow-copy-1']).toEqual(
      state.document.model.flowLocationMap.decisionFlow!.map((point) => ({
        ...point,
        x: point.x + 24,
        y: point.y + 24,
      })),
    );

    state.undo();
    state = useModelerStore.getState();
    expect(state.document.model.processes[0]?.flowElementMap?.['review-copy-1']).toBeUndefined();
    state.redo();
    expect(
      useModelerStore.getState().document.model.processes[0]?.flowElementMap?.['review-copy-1'],
    ).toBeDefined();
  });

  it('copies a data object through its canonical collection and allocates further suffixes', () => {
    useModelerStore.getState().execute(
      createElementCommand(createPaletteElement('data', 'requestData'), {
        x: 760,
        y: 360,
        width: 46,
        height: 62,
        rotation: 0,
        expanded: true,
        xmlRowNumber: 0,
        xmlColumnNumber: 0,
      }),
    );
    useModelerStore.setState({ undoStack: [], redoStack: [] });
    const clipboard = copySelection(useModelerStore.getState().document, ['requestData']);
    expect(clipboard?.dataObjects.map((value) => value.id)).toEqual(['requestData']);
    if (!clipboard) return;

    useModelerStore.getState().execute(pasteClipboardCommand(clipboard));
    useModelerStore.getState().execute(pasteClipboardCommand(clipboard));
    const dataIds = useModelerStore
      .getState()
      .document.model.processes[0]?.dataObjects?.map((value) => value.id);
    expect(dataIds).toEqual(expect.arrayContaining(['requestData-copy-1', 'requestData-copy-2']));
    expect(
      useModelerStore
        .getState()
        .document.model.processes[0]?.flowElements?.map((element) => element.id),
    ).not.toContain('requestData-copy-1');
  });

  it('rejects selections spanning different semantic owners', () => {
    const nested = structuredClone(sampleDocument);
    const process = nested.model.processes[0];
    if (!process) throw new Error('sample process is missing');
    const subprocess = process.flowElements?.find(
      (element) => element.elementType === 'subProcess',
    );
    if (!subprocess || subprocess.elementType !== 'subProcess') return;
    useModelerStore.getState().setDocument(nested);

    expect(copySelection(nested, ['review', subprocess.id ?? ''])).toBeNull();
  });
});
