import { describe, expect, it } from 'vitest';

import type { FormEditorDocument } from '../generated/editor-protocol';
import {
  FormCommandError,
  addFieldCommand,
  addOptionCommand,
  addOutcomeCommand,
  allFields,
  createFormEditorStore,
  findField,
  moveFieldCommand,
  moveOptionCommand,
  moveOutcomeCommand,
  nextFieldId,
  removeFieldCommand,
  removeOptionCommand,
  removeOutcomeCommand,
  updateFieldPropertiesCommand,
  updateFormPropertiesCommand,
  updateOptionCommand,
  updateOutcomeCommand,
} from './index';
import { sampleFormDocument } from './ui/formSampleDocument';

function emptyDocument(): FormEditorDocument {
  return {
    schemaVersion: '1.0',
    model: { key: 'formKey', name: 'Form', fields: [], outcomes: [] },
  };
}

describe('form field commands', () => {
  it('adds a value field with the BaseField variant and an allocated id', () => {
    const store = createFormEditorStore(emptyDocument());

    store.getState().execute(addFieldCommand('text'));
    store.getState().execute(addFieldCommand('text'));

    const fields = store.getState().document.model.fields ?? [];
    expect(fields.map((field) => field.id)).toEqual(['text1', 'text2']);
    expect(fields[0]).toMatchObject({ fieldType: 'BaseField', type: 'text', name: null });
  });

  it('adds option fields with the OptionFormField variant and a seeded option', () => {
    const store = createFormEditorStore(emptyDocument());

    store.getState().execute(addFieldCommand('dropdown'));

    const field = store.getState().document.model.fields?.[0];
    expect(field?.fieldType).toBe('OptionFormField');
    expect(field).toMatchObject({ type: 'dropdown' });
    if (field?.fieldType !== 'OptionFormField') throw new Error('expected option field');
    expect(field.options).toHaveLength(1);
    expect(field.options?.[0]?.name).toBe('Option 1');
  });

  it('adds expression and container fields with their contract variants', () => {
    const store = createFormEditorStore(emptyDocument());

    store.getState().execute(addFieldCommand('expression'));
    store.getState().execute(addFieldCommand('container'));

    const [expression, container] = store.getState().document.model.fields ?? [];
    expect(expression).toMatchObject({ fieldType: 'ExpressionFormField', expression: '' });
    expect(container).toMatchObject({ fieldType: 'Container', fields: [] });
  });

  it('adds fields into a selected container as new rows', () => {
    const store = createFormEditorStore(sampleFormDocument());

    store.getState().execute(addFieldCommand('text', { containerId: 'periodContainer' }));

    const container = findField(store.getState().document.model, 'periodContainer');
    if (container?.fieldType !== 'Container') throw new Error('expected container');
    expect(container.fields).toHaveLength(3);
    expect(container.fields?.[2]?.[0]?.type).toBe('text');
  });

  it('rejects adds into a missing container', () => {
    const store = createFormEditorStore(emptyDocument());

    expect(() =>
      store.getState().execute(addFieldCommand('text', { containerId: 'missing' })),
    ).toThrowError(FormCommandError);
    expect(store.getState().document.model.fields).toHaveLength(0);
    expect(store.getState().undoStack).toHaveLength(0);
  });

  it('predicts allocated ids with nextFieldId', () => {
    const document = sampleFormDocument();
    expect(nextFieldId(document.model, 'date')).toBe('date1');
    expect(nextFieldId(document.model, 'text')).toBe('text1');
  });

  it('removes fields and drops container rows that become empty', () => {
    const store = createFormEditorStore(sampleFormDocument());

    store.getState().execute(removeFieldCommand('startDate'));

    const container = findField(store.getState().document.model, 'periodContainer');
    if (container?.fieldType !== 'Container') throw new Error('expected container');
    expect(container.fields).toHaveLength(1);
    expect(allFields(store.getState().document.model).map((field) => field.id)).not.toContain(
      'startDate',
    );
  });

  it('moves top-level fields with their neighbours', () => {
    const store = createFormEditorStore(sampleFormDocument());

    store.getState().execute(moveFieldCommand('leaveType', -1));

    const ids = (store.getState().document.model.fields ?? []).map((field) => field.id);
    expect(ids).toEqual(['leaveType', 'employeeName', 'approved', 'periodContainer']);
  });

  it('moves container children as whole rows', () => {
    const store = createFormEditorStore(sampleFormDocument());

    store.getState().execute(moveFieldCommand('endDate', -1));

    const container = findField(store.getState().document.model, 'periodContainer');
    if (container?.fieldType !== 'Container') throw new Error('expected container');
    expect(container.fields?.[0]?.[0]?.id).toBe('endDate');
    expect(container.fields?.[1]?.[0]?.id).toBe('startDate');
  });

  it('rejects moves past the edge', () => {
    const store = createFormEditorStore(sampleFormDocument());

    expect(() => store.getState().execute(moveFieldCommand('employeeName', -1))).toThrowError(
      FormCommandError,
    );
  });

  it('renames field ids and rejects duplicates', () => {
    const store = createFormEditorStore(sampleFormDocument());

    store
      .getState()
      .execute(updateFieldPropertiesCommand('employeeName', { id: 'requesterName' }));
    expect(findField(store.getState().document.model, 'requesterName')).not.toBeNull();

    expect(() =>
      store.getState().execute(updateFieldPropertiesCommand('approved', { id: 'requesterName' })),
    ).toThrowError(FormCommandError);
    expect(() =>
      store.getState().execute(updateFieldPropertiesCommand('approved', { id: '  ' })),
    ).toThrowError(/must not be blank/);
  });

  it('updates label, placeholder, required, and date pattern', () => {
    const store = createFormEditorStore(sampleFormDocument());

    store.getState().execute(
      updateFieldPropertiesCommand('startDate', {
        name: 'First day',
        datePattern: 'yyyy-MM-dd',
        required: true,
      }),
    );

    const field = findField(store.getState().document.model, 'startDate');
    expect(field).toMatchObject({ name: 'First day', datePattern: 'yyyy-MM-dd', required: true });
  });

  it('updates expressions only on expression fields', () => {
    const store = createFormEditorStore(emptyDocument());
    store.getState().execute(addFieldCommand('expression'));
    store.getState().execute(addFieldCommand('text'));

    store
      .getState()
      .execute(updateFieldPropertiesCommand('expression1', { expression: '${total}' }));
    const expression = findField(store.getState().document.model, 'expression1');
    if (expression?.fieldType !== 'ExpressionFormField') throw new Error('expected expression');
    expect(expression.expression).toBe('${total}');

    expect(() =>
      store.getState().execute(updateFieldPropertiesCommand('text1', { expression: '${x}' })),
    ).toThrowError(FormCommandError);
  });

  it('stores hyperlink URLs in params and clears them when blank', () => {
    const store = createFormEditorStore(emptyDocument());
    store.getState().execute(addFieldCommand('hyperlink'));

    store
      .getState()
      .execute(updateFieldPropertiesCommand('hyperlink1', { hyperlinkUrl: 'https://flowable.org' }));
    expect(findField(store.getState().document.model, 'hyperlink1')?.params).toEqual({
      url: 'https://flowable.org',
    });

    store.getState().execute(updateFieldPropertiesCommand('hyperlink1', { hyperlinkUrl: '' }));
    expect(findField(store.getState().document.model, 'hyperlink1')?.params).toBeNull();
  });
});

describe('form option commands', () => {
  it('adds, updates, moves, and removes static options', () => {
    const store = createFormEditorStore(sampleFormDocument());

    store.getState().execute(addOptionCommand('leaveType'));
    store
      .getState()
      .execute(updateOptionCommand('leaveType', 2, { id: 'parental', name: 'Parental leave' }));
    store.getState().execute(moveOptionCommand('leaveType', 2, -1));
    store.getState().execute(removeOptionCommand('leaveType', 0));

    const field = findField(store.getState().document.model, 'leaveType');
    if (field?.fieldType !== 'OptionFormField') throw new Error('expected option field');
    expect(field.options?.map((option) => option.id)).toEqual(['parental', 'sick']);
    expect(field.options?.[0]?.name).toBe('Parental leave');
  });

  it('rejects duplicate option ids', () => {
    const store = createFormEditorStore(sampleFormDocument());

    expect(() =>
      store.getState().execute(updateOptionCommand('leaveType', 1, { id: 'vacation' })),
    ).toThrowError(FormCommandError);
  });

  it('rejects option commands on non-option fields', () => {
    const store = createFormEditorStore(sampleFormDocument());

    expect(() => store.getState().execute(addOptionCommand('employeeName'))).toThrowError(
      /does not carry options/,
    );
  });
});

describe('form outcome commands', () => {
  it('adds, renames, moves, and removes outcomes', () => {
    const store = createFormEditorStore(sampleFormDocument());

    store.getState().execute(addOutcomeCommand());
    store.getState().execute(updateOutcomeCommand(1, { name: 'Save draft' }));
    store.getState().execute(moveOutcomeCommand(1, -1));

    let outcomes = store.getState().document.model.outcomes ?? [];
    expect(outcomes.map((outcome) => outcome.name)).toEqual(['Save draft', 'Submit request']);

    store.getState().execute(removeOutcomeCommand(0));
    outcomes = store.getState().document.model.outcomes ?? [];
    expect(outcomes.map((outcome) => outcome.name)).toEqual(['Submit request']);
  });

  it('rejects out-of-range outcome commands', () => {
    const store = createFormEditorStore(sampleFormDocument());

    expect(() => store.getState().execute(removeOutcomeCommand(7))).toThrowError(FormCommandError);
    expect(() => store.getState().execute(moveOutcomeCommand(0, -1))).toThrowError(
      FormCommandError,
    );
  });
});

describe('form model property commands', () => {
  it('updates key, name, description, and outcome variable', () => {
    const store = createFormEditorStore(sampleFormDocument());

    store.getState().execute(
      updateFormPropertiesCommand({
        key: 'leaveRequestForm',
        name: 'Leave request v2',
        description: 'Updated description',
        outcomeVariableName: 'formOutcome',
      }),
    );

    expect(store.getState().document.model).toMatchObject({
      key: 'leaveRequestForm',
      name: 'Leave request v2',
      description: 'Updated description',
      outcomeVariableName: 'formOutcome',
    });
  });

  it('rejects blank keys and names', () => {
    const store = createFormEditorStore(sampleFormDocument());

    expect(() =>
      store.getState().execute(updateFormPropertiesCommand({ key: ' ' })),
    ).toThrowError(/must not be blank/);
    expect(() =>
      store.getState().execute(updateFormPropertiesCommand({ name: '' })),
    ).toThrowError(/must not be blank/);
    expect(store.getState().document.model.key).toBe('leaveForm');
  });
});
