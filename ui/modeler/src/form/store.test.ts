import { describe, expect, it } from 'vitest';

import {
  addFieldCommand,
  createFormEditorStore,
  updateFieldPropertiesCommand,
} from './index';
import { sampleFormDocument } from './ui/formSampleDocument';

describe('form editor store', () => {
  it('executes commands and tracks undo and redo stacks', () => {
    const store = createFormEditorStore(sampleFormDocument());

    store.getState().execute(addFieldCommand('text'));

    expect(store.getState().undoStack).toHaveLength(1);
    expect(store.getState().undoStack[0]?.label).toBe('Add Text field');
    expect(store.getState().redoStack).toHaveLength(0);
    expect(store.getState().document.model.fields).toHaveLength(5);
  });

  it('undoes and redoes command patches', () => {
    const store = createFormEditorStore(sampleFormDocument());

    store
      .getState()
      .execute(updateFieldPropertiesCommand('employeeName', { name: 'Requester' }));
    expect(findName(store)).toBe('Requester');

    store.getState().undo();
    expect(findName(store)).toBe('Employee name');
    expect(store.getState().redoStack).toHaveLength(1);

    store.getState().redo();
    expect(findName(store)).toBe('Requester');
  });

  it('clears the redo stack when a new command executes', () => {
    const store = createFormEditorStore(sampleFormDocument());

    store.getState().execute(addFieldCommand('text'));
    store.getState().undo();
    store.getState().execute(addFieldCommand('date'));

    expect(store.getState().redoStack).toHaveLength(0);
    const ids = (store.getState().document.model.fields ?? []).map((field) => field.id);
    expect(ids).toContain('date1');
    expect(ids).not.toContain('text1');
  });

  it('tracks the selected field and resets state on setDocument', () => {
    const store = createFormEditorStore(sampleFormDocument());

    store.getState().selectField('leaveType');
    store.getState().execute(addFieldCommand('text'));
    expect(store.getState().selectedFieldId).toBe('leaveType');

    store.getState().setDocument(sampleFormDocument());
    expect(store.getState().selectedFieldId).toBeNull();
    expect(store.getState().undoStack).toHaveLength(0);
    expect(store.getState().redoStack).toHaveLength(0);
  });

  it('ignores no-op commands without polluting the history', () => {
    const store = createFormEditorStore(sampleFormDocument());

    store
      .getState()
      .execute(updateFieldPropertiesCommand('employeeName', { name: 'Employee name' }));

    expect(store.getState().undoStack).toHaveLength(0);
  });
});

function findName(store: ReturnType<typeof createFormEditorStore>): string | null | undefined {
  return (store.getState().document.model.fields ?? []).find(
    (field) => field.id === 'employeeName',
  )?.name;
}
