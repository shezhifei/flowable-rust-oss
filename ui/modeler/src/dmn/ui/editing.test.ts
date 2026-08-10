import { describe, expect, it } from 'vitest';

import {
  addInputColumnCommand,
  addOutputColumnCommand,
  addRuleCommand,
  createDmnEditorStore,
  deleteRuleCommand,
  moveRuleCommand,
  setHitPolicyCommand,
} from '../index';
import { sampleDmnDocument } from './dmnSampleDocument';
import { commitCellText, executeDmnCommand } from './editing';

describe('cell editing through the UI commit helper', () => {
  it('commits a valid input cell draft as an undoable command', () => {
    const store = createDmnEditorStore(sampleDmnDocument());

    const error = commitCellText(store, 'leaveDecision', { kind: 'input', row: 0, column: 0 }, '[3..10]');

    expect(error).toBeNull();
    expect(inputText(store.getState().document, 0, 0)).toBe('[3..10]');
    expect(store.getState().undoStack.at(-1)?.label).toBe('Edit input cell 1:1');

    store.getState().undo();
    expect(inputText(store.getState().document, 0, 0)).toBe('> 5');
  });

  it('commits a valid output cell draft and normalizes blank text to null', () => {
    const store = createDmnEditorStore(sampleDmnDocument());

    expect(
      commitCellText(store, 'leaveDecision', { kind: 'output', row: 0, column: 0 }, '"ESCALATED"'),
    ).toBeNull();
    expect(outputText(store.getState().document, 0, 0)).toBe('"ESCALATED"');

    expect(
      commitCellText(store, 'leaveDecision', { kind: 'output', row: 0, column: 0 }, '   '),
    ).toBeNull();
    expect(outputText(store.getState().document, 0, 0)).toBeNull();
  });

  it('rejects out-of-subset unary tests without touching the document', () => {
    const store = createDmnEditorStore(sampleDmnDocument());

    const error = commitCellText(
      store,
      'leaveDecision',
      { kind: 'input', row: 0, column: 0 },
      'leaveDays && role',
    );

    expect(error).toEqual(expect.any(String));
    expect(inputText(store.getState().document, 0, 0)).toBe('> 5');
    expect(store.getState().undoStack).toHaveLength(0);
  });

  it('rejects out-of-subset output expressions without touching the document', () => {
    const store = createDmnEditorStore(sampleDmnDocument());

    const error = commitCellText(
      store,
      'leaveDecision',
      { kind: 'output', row: 0, column: 0 },
      'if approved == true { "ok" };',
    );

    expect(error).toEqual(expect.any(String));
    expect(outputText(store.getState().document, 0, 0)).toBe('"APPROVED"');
    expect(store.getState().undoStack).toHaveLength(0);
  });
});

describe('column and rule editing through UI-dispatched commands', () => {
  it('adds and deletes columns with matching rule entries', () => {
    const store = createDmnEditorStore(sampleDmnDocument());

    expect(executeDmnCommand(store, addInputColumnCommand('leaveDecision'))).toBeNull();
    expect(executeDmnCommand(store, addOutputColumnCommand('leaveDecision'))).toBeNull();

    let table = store.getState().document.model.decisions?.[0]?.decisionTable;
    expect(table?.inputs).toHaveLength(3);
    expect(table?.outputs).toHaveLength(3);
    expect(table?.rules?.[0]?.inputEntries).toHaveLength(3);
    expect(table?.rules?.[0]?.outputEntries).toHaveLength(3);

    store.getState().undo();
    store.getState().undo();
    table = store.getState().document.model.decisions?.[0]?.decisionTable;
    expect(table?.inputs).toHaveLength(2);
    expect(table?.outputs).toHaveLength(2);
  });

  it('adds, moves, and deletes rules', () => {
    const store = createDmnEditorStore(sampleDmnDocument());

    expect(executeDmnCommand(store, addRuleCommand('leaveDecision'))).toBeNull();
    expect(store.getState().document.model.decisions?.[0]?.decisionTable.rules).toHaveLength(3);

    expect(executeDmnCommand(store, moveRuleCommand('leaveDecision', 2, 0))).toBeNull();
    expect(
      store.getState().document.model.decisions?.[0]?.decisionTable.rules?.map(
        (rule) => rule.ruleNumber,
      ),
    ).toEqual([1, 2, 3]);

    expect(executeDmnCommand(store, deleteRuleCommand('leaveDecision', 0))).toBeNull();
    expect(store.getState().document.model.decisions?.[0]?.decisionTable.rules).toHaveLength(2);
  });
});

describe('hit policy editing through the UI select', () => {
  it('changes the hit policy and undoes the change', () => {
    const store = createDmnEditorStore(sampleDmnDocument());

    expect(executeDmnCommand(store, setHitPolicyCommand('leaveDecision', 'PRIORITY'))).toBeNull();
    expect(store.getState().document.model.decisions?.[0]?.decisionTable.hitPolicy).toBe(
      'PRIORITY',
    );
    expect(store.getState().undoStack.at(-1)?.label).toBe('Set hit policy PRIORITY');

    store.getState().undo();
    expect(store.getState().document.model.decisions?.[0]?.decisionTable.hitPolicy).toBe('FIRST');
  });

  it('surfaces command errors instead of throwing', () => {
    const store = createDmnEditorStore(sampleDmnDocument());

    // The sample table has two outputs, so an aggregated COLLECT is rejected.
    const error = executeDmnCommand(store, setHitPolicyCommand('leaveDecision', 'COLLECT', 'SUM'));

    expect(error).toEqual(expect.any(String));
    expect(store.getState().document.model.decisions?.[0]?.decisionTable.hitPolicy).toBe('FIRST');
  });
});

function inputText(document: ReturnType<typeof sampleDmnDocument>, row: number, column: number) {
  return document.model.decisions?.[0]?.decisionTable.rules?.[row]?.inputEntries?.[column]?.text;
}

function outputText(document: ReturnType<typeof sampleDmnDocument>, row: number, column: number) {
  return document.model.decisions?.[0]?.decisionTable.rules?.[row]?.outputEntries?.[column]?.text;
}
