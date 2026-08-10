import { describe, expect, it } from 'vitest';

import type { DmnEditorDocument } from '../generated/editor-protocol';
import { decisionServicesFor, readDecisionTableProperties } from './adapters';
import { CREATABLE_HIT_POLICIES, ROUND_TRIP_HIT_POLICIES, VALUE_TYPE_REFS } from './capabilities';
import {
  addInputColumnCommand,
  addOutputColumnCommand,
  addRuleCommand,
  deleteInputColumnCommand,
  deleteOutputColumnCommand,
  deleteRuleCommand,
  DmnCommandError,
  editCellCommand,
  moveRuleCommand,
  setHitPolicyCommand,
  updateDecisionPropertiesCommand,
  updateDefinitionPropertiesCommand,
  updateInputColumnCommand,
  updateOutputColumnCommand,
} from './commands';
import { createDmnEditorStore } from './store';

describe('canonical DMN table commands', () => {
  it('adapts the decision id as key without adding a second semantic field', () => {
    const document = makeDocument();

    expect(readDecisionTableProperties(document, 'loan')).toEqual({
      definitionId: 'definitions',
      definitionName: 'Eligibility',
      definitionNamespace: 'https://flowable.org/dmn',
      key: 'loan',
      name: 'Loan decision',
      tableId: 'loanTable',
    });
    expect(decisionServicesFor(document, 'loan').map((service) => service.id)).toEqual([
      'loanService',
    ]);
    expect(document.model.decisions?.[0]).not.toHaveProperty('key');
  });

  it('updates definition and decision properties and rewrites canonical decision references', () => {
    const store = createDmnEditorStore(makeDocument());
    const before = store.getState().document;

    store.getState().execute(
      updateDefinitionPropertiesCommand({
        id: 'renamedDefinitions',
        name: 'Renamed eligibility',
        namespace: 'https://example.test/eligibility',
      }),
    );
    store.getState().execute(
      updateDecisionPropertiesCommand('loan', {
        key: 'loanEligibility',
        name: 'Loan eligibility',
        tableId: 'loanEligibilityTable',
      }),
    );

    const definition = store.getState().document.model;
    expect(definition).toMatchObject({
      id: 'renamedDefinitions',
      name: 'Renamed eligibility',
      namespace: 'https://example.test/eligibility',
    });
    expect(definition.decisions?.[0]).toMatchObject({
      id: 'loanEligibility',
      name: 'Loan eligibility',
      decisionTable: { id: 'loanEligibilityTable' },
    });
    expect(definition.decisions?.[1]?.requiredDecisions).toEqual(['loanEligibility']);
    expect(definition.decisionServices?.[0]).toMatchObject({
      requiredDecisions: ['loanEligibility'],
      outputDecisions: ['loanEligibility'],
    });
    expect(definition.authorityRequirements?.[0]).toMatchObject({
      requiredDecision: 'loanEligibility',
      decision: 'loanEligibility',
    });

    store.getState().undo();
    store.getState().undo();
    expect(store.getState().document).toEqual(before);
    store.getState().redo();
    store.getState().redo();
    expect(store.getState().document.model.decisions?.[0]?.id).toBe('loanEligibility');
  });

  it('inserts, updates, and deletes an input column with aligned rule entries', () => {
    const store = createDmnEditorStore(makeDocument());

    store
      .getState()
      .execute(
        addInputColumnCommand(
          'loan',
          { id: 'income', label: 'Income', expression: 'income', typeRef: 'number' },
          1,
        ),
      );
    let table = loanTable(store.getState().document);
    expect(table.inputs?.map((input) => [input.id, input.inputNumber])).toEqual([
      ['age', 1],
      ['income', 2],
      ['country', 3],
    ]);
    expect(table.rules?.map((rule) => rule.inputEntries?.length)).toEqual([3, 3]);
    expect(table.rules?.[0]?.inputEntries?.[1]).toMatchObject({ text: null });

    store.getState().execute(
      updateInputColumnCommand('loan', 1, {
        label: 'Annual income',
        expression: 'applicant.income',
        typeRef: 'double',
      }),
    );
    table = loanTable(store.getState().document);
    expect(table.inputs?.[1]).toMatchObject({
      label: 'Annual income',
      inputExpression: { text: 'applicant.income', typeRef: 'double' },
    });

    store.getState().execute(deleteInputColumnCommand('loan', 0));
    table = loanTable(store.getState().document);
    expect(table.inputs?.map((input) => [input.id, input.inputNumber])).toEqual([
      ['income', 1],
      ['country', 2],
    ]);
    expect(table.rules?.[0]?.inputEntries?.map((entry) => entry.text)).toEqual([null, '"US"']);
  });

  it('inserts, updates, and deletes an output column with typed canonical entries', () => {
    const store = createDmnEditorStore(makeDocument());

    store.getState().execute(
      addOutputColumnCommand(
        'loan',
        {
          id: 'reason',
          label: 'Reason',
          name: 'reason',
          typeRef: 'string',
          outputValues: '"age", "country"',
        },
        0,
      ),
    );
    let table = loanTable(store.getState().document);
    expect(table.outputs?.map((output) => [output.id, output.outputNumber])).toEqual([
      ['reason', 1],
      ['approved', 2],
    ]);
    expect(table.outputs?.[0]).toMatchObject({
      name: 'reason',
      typeRef: 'string',
      outputValues: { text: '"age", "country"' },
    });
    expect(table.rules?.[0]?.outputEntries?.[0]).toMatchObject({ text: null, typeRef: 'string' });

    store.getState().execute(
      updateOutputColumnCommand('loan', 0, {
        label: 'Explanation',
        name: 'explanation',
        typeRef: 'context',
        outputValues: null,
      }),
    );
    table = loanTable(store.getState().document);
    expect(table.outputs?.[0]).toMatchObject({
      label: 'Explanation',
      name: 'explanation',
      typeRef: 'context',
      outputValues: null,
    });

    store.getState().execute(deleteOutputColumnCommand('loan', 1));
    table = loanTable(store.getState().document);
    expect(table.outputs?.map((output) => [output.id, output.outputNumber])).toEqual([
      ['reason', 1],
    ]);
    expect(table.rules?.every((rule) => rule.outputEntries?.length === 1)).toBe(true);
  });

  it('adds, deletes, and reorders rules while preserving one-based wire numbers', () => {
    const store = createDmnEditorStore(makeDocument());

    store.getState().execute(addRuleCommand('loan', 'ruleInserted', 1));
    let rules = loanTable(store.getState().document).rules ?? [];
    expect(rules.map((rule) => [rule.id, rule.ruleNumber])).toEqual([
      ['ruleAdultUs', 1],
      ['ruleInserted', 2],
      ['ruleFallback', 3],
    ]);
    expect(rules[1]?.inputEntries).toHaveLength(2);
    expect(rules[1]?.outputEntries).toHaveLength(1);

    store.getState().execute(moveRuleCommand('loan', 2, 0));
    rules = loanTable(store.getState().document).rules ?? [];
    expect(rules.map((rule) => [rule.id, rule.ruleNumber])).toEqual([
      ['ruleFallback', 1],
      ['ruleAdultUs', 2],
      ['ruleInserted', 3],
    ]);

    store.getState().execute(deleteRuleCommand('loan', 1));
    rules = loanTable(store.getState().document).rules ?? [];
    expect(rules.map((rule) => [rule.id, rule.ruleNumber])).toEqual([
      ['ruleFallback', 1],
      ['ruleInserted', 2],
    ]);
  });

  it('edits input unary tests and typed output expressions in place', () => {
    const store = createDmnEditorStore(makeDocument());
    const inputEntryId = loanTable(store.getState().document).rules?.[0]?.inputEntries?.[0]?.id;
    const outputEntryId = loanTable(store.getState().document).rules?.[0]?.outputEntries?.[0]?.id;

    store
      .getState()
      .execute(editCellCommand('loan', { kind: 'input', row: 0, column: 0 }, { text: '[21..65]' }));
    store
      .getState()
      .execute(
        editCellCommand(
          'loan',
          { kind: 'output', row: 0, column: 0 },
          { text: 'age >= 25', typeRef: 'boolean' },
        ),
      );

    const rule = loanTable(store.getState().document).rules?.[0];
    expect(rule?.inputEntries?.[0]).toEqual({ id: inputEntryId, text: '[21..65]' });
    expect(rule?.outputEntries?.[0]).toEqual({
      id: outputEntryId,
      text: 'age >= 25',
      typeRef: 'boolean',
    });
    store.getState().undo();
    store.getState().undo();
    expect(loanTable(store.getState().document).rules?.[0]?.inputEntries?.[0]?.text).toBe('>= 18');
  });

  it('offers only engine-creatable policies while preserving imported COMPLETE', () => {
    const complete = makeDocument();
    const table = loanTable(complete);
    table.hitPolicy = 'COMPLETE';
    const store = createDmnEditorStore(complete);

    expect(CREATABLE_HIT_POLICIES).not.toContain('COMPLETE');
    expect(ROUND_TRIP_HIT_POLICIES).toContain('COMPLETE');
    expect(store.getState().document).toEqual(complete);

    expect(() =>
      store.getState().execute(setHitPolicyCommand('loan', 'COMPLETE' as never)),
    ).toThrowError(
      expect.objectContaining<Partial<DmnCommandError>>({ code: 'unsupported-hit-policy' }),
    );
    expect(store.getState()).toMatchObject({ document: complete, undoStack: [] });

    store.getState().execute(setHitPolicyCommand('loan', 'FIRST'));
    expect(loanTable(store.getState().document)).toMatchObject({
      hitPolicy: 'FIRST',
      collectOperator: null,
    });
    store.getState().undo();
    expect(loanTable(store.getState().document).hitPolicy).toBe('COMPLETE');
  });

  it('enforces the engine aggregation shape when selecting an operator', () => {
    const document = makeDocument();
    const table = loanTable(document);
    table.outputs![0]!.typeRef = 'number';
    const store = createDmnEditorStore(document);

    store.getState().execute(setHitPolicyCommand('loan', 'COLLECT', 'SUM'));
    expect(loanTable(store.getState().document)).toMatchObject({
      hitPolicy: 'COLLECT',
      collectOperator: 'SUM',
    });
    expect(() =>
      store
        .getState()
        .execute(addOutputColumnCommand('loan', { name: 'second', typeRef: 'number' })),
    ).toThrowError(
      expect.objectContaining<Partial<DmnCommandError>>({ code: 'invalid-collect-operator' }),
    );
    expect(() =>
      store.getState().execute(updateOutputColumnCommand('loan', 0, { typeRef: 'string' })),
    ).toThrowError(
      expect.objectContaining<Partial<DmnCommandError>>({ code: 'invalid-collect-operator' }),
    );
    expect(store.getState().undoStack).toHaveLength(1);
  });

  it('rejects destructive and malformed commands without creating history', () => {
    const malformed = makeDocument();
    loanTable(malformed).rules?.[0]?.inputEntries?.pop();
    const malformedStore = createDmnEditorStore(malformed);
    expect(() => malformedStore.getState().execute(addRuleCommand('loan'))).toThrowError(
      expect.objectContaining<Partial<DmnCommandError>>({ code: 'invalid-table-shape' }),
    );
    expect(malformedStore.getState().undoStack).toHaveLength(0);

    const store = createDmnEditorStore(makeDocument());
    expect(() =>
      store.getState().execute(updateDecisionPropertiesCommand('loan', { key: 'risk' })),
    ).toThrowError(expect.objectContaining<Partial<DmnCommandError>>({ code: 'duplicate-id' }));
    store.getState().execute(deleteRuleCommand('loan', 1));
    expect(() => store.getState().execute(deleteRuleCommand('loan', 0))).toThrowError(
      expect.objectContaining<Partial<DmnCommandError>>({ code: 'last-rule' }),
    );
    expect(() => store.getState().execute(deleteOutputColumnCommand('loan', 0))).toThrowError(
      expect.objectContaining<Partial<DmnCommandError>>({ code: 'last-output' }),
    );
    expect(() =>
      store.getState().execute(updateInputColumnCommand('loan', 0, { typeRef: 'money' as never })),
    ).toThrowError(
      expect.objectContaining<Partial<DmnCommandError>>({ code: 'invalid-value-type' }),
    );
    expect(store.getState().undoStack).toHaveLength(1);
  });

  it('keeps capability values aligned with the public Rust editor contract', () => {
    expect(CREATABLE_HIT_POLICIES).toEqual([
      'FIRST',
      'UNIQUE',
      'ANY',
      'COLLECT',
      'RULE_ORDER',
      'OUTPUT_ORDER',
      'PRIORITY',
    ]);
    expect(VALUE_TYPE_REFS).toEqual([
      'string',
      'boolean',
      'integer',
      'long',
      'double',
      'number',
      'date',
      'time',
      'dateTime',
      'duration',
      'dayTimeDuration',
      'yearMonthDuration',
      'context',
      'list',
    ]);
  });
});

function loanTable(document: DmnEditorDocument) {
  const decision = document.model.decisions?.find((candidate) => candidate.id === 'loan');
  if (!decision) throw new Error('loan decision fixture missing');
  return decision.decisionTable;
}

function makeDocument(): DmnEditorDocument {
  return {
    schemaVersion: '1.0',
    model: {
      id: 'definitions',
      name: 'Eligibility',
      namespace: 'https://flowable.org/dmn',
      decisions: [
        {
          id: 'loan',
          name: 'Loan decision',
          decisionTable: {
            id: 'loanTable',
            hitPolicy: 'UNIQUE',
            collectOperator: null,
            inputs: [
              {
                id: 'age',
                label: 'Age',
                inputNumber: 1,
                inputExpression: { id: 'ageExpression', text: 'age', typeRef: 'integer' },
              },
              {
                id: 'country',
                label: 'Country',
                inputNumber: 2,
                inputExpression: {
                  id: 'countryExpression',
                  text: 'country',
                  typeRef: 'string',
                },
              },
            ],
            outputs: [
              {
                id: 'approved',
                label: 'Approved',
                name: 'approved',
                outputNumber: 1,
                typeRef: 'boolean',
              },
            ],
            rules: [
              {
                id: 'ruleAdultUs',
                ruleNumber: 1,
                inputEntries: [
                  { id: 'ruleAdultUsAge', text: '>= 18' },
                  { id: 'ruleAdultUsCountry', text: '"US"' },
                ],
                outputEntries: [{ id: 'ruleAdultUsApproved', text: 'true', typeRef: 'boolean' }],
              },
              {
                id: 'ruleFallback',
                ruleNumber: 2,
                inputEntries: [
                  { id: 'ruleFallbackAge', text: '-' },
                  { id: 'ruleFallbackCountry', text: '-' },
                ],
                outputEntries: [{ id: 'ruleFallbackApproved', text: 'false', typeRef: 'boolean' }],
              },
            ],
          },
        },
        {
          id: 'risk',
          name: 'Risk decision',
          requiredDecisions: ['loan'],
          decisionTable: {
            id: 'riskTable',
            hitPolicy: 'FIRST',
            outputs: [{ id: 'riskOutput', name: 'risk', outputNumber: 1, typeRef: 'string' }],
            rules: [
              {
                id: 'riskRule',
                ruleNumber: 1,
                inputEntries: [],
                outputEntries: [{ id: 'riskValue', text: '"low"', typeRef: 'string' }],
              },
            ],
          },
        },
      ],
      decisionServices: [
        {
          id: 'loanService',
          name: 'Loan service',
          requiredDecisions: ['loan'],
          outputDecisions: ['loan'],
        },
      ],
      authorityRequirements: [
        {
          id: 'loanAuthority',
          requiredDecision: 'loan',
          decision: 'loan',
        },
      ],
    },
  };
}
