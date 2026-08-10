import { describe, expect, it } from 'vitest';

import type { DmnEditorDocument } from '../generated/editor-protocol';
import { editCellCommand } from './commands';
import { createDmnEditorStore } from './store';

describe('DMN editor store', () => {
  it('uses canonical documents as the only durable editor state', () => {
    const document = oneCellDocument('first');
    const store = createDmnEditorStore(document);

    expect(store.getState()).toMatchObject({
      document,
      selectedDecisionId: 'decision',
      undoStack: [],
      redoStack: [],
    });
    expect(Object.keys(store.getState())).not.toContain('tableModel');
  });

  it('undoes and redoes atomic canonical cell patches', () => {
    const store = createDmnEditorStore(oneCellDocument('first'));

    store
      .getState()
      .execute(
        editCellCommand('decision', { kind: 'output', row: 0, column: 0 }, { text: 'second' }),
      );
    expect(outputText(store.getState().document)).toBe('second');
    expect(store.getState()).toMatchObject({
      undoStack: [expect.objectContaining({ label: 'Edit output cell 1:1' })],
      redoStack: [],
    });

    store.getState().undo();
    expect(outputText(store.getState().document)).toBe('first');
    expect(store.getState()).toMatchObject({ undoStack: [], redoStack: [expect.any(Object)] });

    store.getState().redo();
    expect(outputText(store.getState().document)).toBe('second');
    expect(store.getState()).toMatchObject({ undoStack: [expect.any(Object)], redoStack: [] });
  });

  it('caps history at one hundred entries and clears redo on a new edit', () => {
    const store = createDmnEditorStore(oneCellDocument('0'));

    for (let index = 1; index <= 110; index += 1) {
      store
        .getState()
        .execute(
          editCellCommand(
            'decision',
            { kind: 'output', row: 0, column: 0 },
            { text: String(index) },
          ),
        );
    }
    expect(store.getState().undoStack).toHaveLength(100);
    store.getState().undo();
    expect(store.getState().redoStack).toHaveLength(1);
    store
      .getState()
      .execute(
        editCellCommand('decision', { kind: 'output', row: 0, column: 0 }, { text: 'branch' }),
      );
    expect(store.getState()).toMatchObject({ redoStack: [] });
    expect(outputText(store.getState().document)).toBe('branch');
  });

  it('replaces a loaded document and resets transient selection and history', () => {
    const store = createDmnEditorStore(oneCellDocument('first'));
    store
      .getState()
      .execute(
        editCellCommand('decision', { kind: 'output', row: 0, column: 0 }, { text: 'changed' }),
      );
    store.getState().selectDecision(null);

    const replacement = oneCellDocument('replacement', 'replacementDecision');
    store.getState().setDocument(replacement);
    expect(store.getState()).toMatchObject({
      document: replacement,
      selectedDecisionId: 'replacementDecision',
      undoStack: [],
      redoStack: [],
    });
  });
});

function outputText(document: DmnEditorDocument) {
  return document.model.decisions?.[0]?.decisionTable.rules?.[0]?.outputEntries?.[0]?.text;
}

function oneCellDocument(text: string, decisionId = 'decision'): DmnEditorDocument {
  return {
    schemaVersion: '1.0',
    model: {
      id: 'definition',
      decisions: [
        {
          id: decisionId,
          name: 'Decision',
          decisionTable: {
            id: 'table',
            hitPolicy: 'FIRST',
            outputs: [{ id: 'output', name: 'result', outputNumber: 1, typeRef: 'string' }],
            rules: [
              {
                id: 'rule',
                ruleNumber: 1,
                inputEntries: [],
                outputEntries: [{ id: 'entry', text, typeRef: 'string' }],
              },
            ],
          },
        },
      ],
    },
  };
}
