import type { Draft } from 'immer';

import type {
  CollectOperator,
  Decision,
  DecisionRule,
  DecisionTable,
  DmnEditorDocument,
  DmnDefinition,
  HitPolicy,
  InputClause,
  LiteralExpression,
  OutputClause,
  UnaryTests,
} from '../generated/editor-protocol';
import {
  COLLECT_OPERATORS,
  isCreatableHitPolicy,
  isDmnValueTypeRef,
  type CreatableHitPolicy,
  type DmnValueTypeRef,
} from './capabilities';

export interface DmnCommand {
  label: string;
  apply: (document: Draft<DmnEditorDocument>) => void;
}

export type DmnCommandErrorCode =
  | 'decision-not-found'
  | 'invalid-id'
  | 'duplicate-id'
  | 'invalid-index'
  | 'invalid-table-shape'
  | 'last-output'
  | 'last-rule'
  | 'unsupported-hit-policy'
  | 'invalid-collect-operator'
  | 'invalid-value-type';

export class DmnCommandError extends Error {
  constructor(
    public readonly code: DmnCommandErrorCode,
    message: string,
  ) {
    super(message);
    this.name = 'DmnCommandError';
  }
}

export interface DefinitionPropertyChanges {
  id?: string | null;
  name?: string | null;
  namespace?: string | null;
}

export interface DecisionPropertyChanges {
  /** Canonical adapter for `Decision.id`. */
  key?: string;
  name?: string | null;
  tableId?: string;
}

export interface InputColumnDraft {
  id?: string;
  label?: string | null;
  expression?: string | null;
  typeRef?: DmnValueTypeRef | null;
}

export interface InputColumnChanges {
  id?: string;
  label?: string | null;
  expression?: string | null;
  typeRef?: DmnValueTypeRef | null;
}

export interface OutputColumnDraft {
  id?: string;
  label?: string | null;
  name?: string | null;
  typeRef?: DmnValueTypeRef | null;
  outputValues?: string | null;
}

export type OutputColumnChanges = OutputColumnDraft;

export type DmnCellAddress =
  { kind: 'input'; row: number; column: number } | { kind: 'output'; row: number; column: number };

export interface DmnCellValue {
  text: string | null;
  /** Only output entries carry a canonical cell-level typeRef. */
  typeRef?: DmnValueTypeRef | null;
}

export function updateDefinitionPropertiesCommand(changes: DefinitionPropertyChanges): DmnCommand {
  return {
    label: 'Update DMN definition properties',
    apply(document) {
      if (changes.id !== undefined) {
        if (changes.id !== null) assertNonBlankId(changes.id, 'definition id');
        document.model.id = changes.id;
      }
      if (changes.name !== undefined) document.model.name = changes.name;
      if (changes.namespace !== undefined) document.model.namespace = changes.namespace;
    },
  };
}

export function updateDecisionPropertiesCommand(
  decisionId: string,
  changes: DecisionPropertyChanges,
): DmnCommand {
  return {
    label: `Update decision ${decisionId}`,
    apply(document) {
      const decision = requireDecision(document.model, decisionId);
      if (changes.key !== undefined && changes.key !== decision.id) {
        assertNonBlankId(changes.key, 'decision key');
        assertDecisionKeyAvailable(document.model, changes.key);
        replaceDecisionReferences(document.model, decision.id, changes.key);
        decision.id = changes.key;
      }
      if (changes.name !== undefined) decision.name = changes.name;
      if (changes.tableId !== undefined && changes.tableId !== decision.decisionTable.id) {
        assertNonBlankId(changes.tableId, 'decision table id');
        assertCanonicalIdAvailable(document.model, changes.tableId, decision.decisionTable.id);
        decision.decisionTable.id = changes.tableId;
      }
    },
  };
}

export function addInputColumnCommand(
  decisionId: string,
  column: InputColumnDraft = {},
  index?: number,
): DmnCommand {
  return {
    label: `Add input column to ${decisionId}`,
    apply(document) {
      const table = requireTable(document.model, decisionId);
      assertTableShape(table);
      const inputs = (table.inputs ??= []);
      const insertionIndex = insertionPoint(index, inputs.length);
      const usedIds = collectCanonicalIds(document.model);
      const id = allocateOrAssertId(usedIds, column.id, 'InputClause');
      const expressionId = allocateId(usedIds, `${id}_expression`);
      const inputExpression: Draft<LiteralExpression> = {
        id: expressionId,
        text: column.expression ?? null,
        typeRef: checkedTypeRef(column.typeRef),
      };
      const input: Draft<InputClause> = {
        id,
        label: column.label ?? null,
        inputNumber: insertionIndex + 1,
        inputExpression,
      };
      inputs.splice(insertionIndex, 0, input);
      for (const [rowIndex, rule] of (table.rules ?? []).entries()) {
        const entries = (rule.inputEntries ??= []);
        const ruleStem = rule.id ?? `Rule_${rowIndex + 1}`;
        entries.splice(insertionIndex, 0, {
          id: allocateId(usedIds, `${ruleStem}_${id}`),
          text: null,
        });
      }
      renumberTable(table);
    },
  };
}

export function updateInputColumnCommand(
  decisionId: string,
  index: number,
  changes: InputColumnChanges,
): DmnCommand {
  return {
    label: `Update input column ${index + 1}`,
    apply(document) {
      const table = requireTable(document.model, decisionId);
      assertTableShape(table);
      const input = requireAt(table.inputs ?? [], index, 'input column');
      if (changes.id !== undefined && changes.id !== input.id) {
        assertNonBlankId(changes.id, 'input column id');
        assertCanonicalIdAvailable(document.model, changes.id, input.id ?? undefined);
        input.id = changes.id;
      }
      if (changes.label !== undefined) input.label = changes.label;
      if (changes.expression !== undefined) input.inputExpression.text = changes.expression;
      if (changes.typeRef !== undefined) {
        input.inputExpression.typeRef = checkedTypeRef(changes.typeRef);
      }
    },
  };
}

export function deleteInputColumnCommand(decisionId: string, index: number): DmnCommand {
  return {
    label: `Delete input column ${index + 1}`,
    apply(document) {
      const table = requireTable(document.model, decisionId);
      assertTableShape(table);
      requireAt(table.inputs ?? [], index, 'input column');
      table.inputs?.splice(index, 1);
      for (const rule of table.rules ?? []) rule.inputEntries?.splice(index, 1);
      renumberTable(table);
    },
  };
}

export function addOutputColumnCommand(
  decisionId: string,
  column: OutputColumnDraft = {},
  index?: number,
): DmnCommand {
  return {
    label: `Add output column to ${decisionId}`,
    apply(document) {
      const table = requireTable(document.model, decisionId);
      assertTableShape(table);
      if (table.hitPolicy === 'COLLECT' && table.collectOperator) {
        throw new DmnCommandError(
          'invalid-collect-operator',
          'An aggregated COLLECT table must have exactly one output',
        );
      }
      const outputs = (table.outputs ??= []);
      const insertionIndex = insertionPoint(index, outputs.length);
      const usedIds = collectCanonicalIds(document.model);
      const id = allocateOrAssertId(usedIds, column.id, 'OutputClause');
      const typeRef = checkedTypeRef(column.typeRef);
      const output: Draft<OutputClause> = {
        id,
        label: column.label ?? null,
        name: column.name ?? null,
        typeRef,
        outputNumber: insertionIndex + 1,
        outputValues:
          column.outputValues === undefined || column.outputValues === null
            ? null
            : {
                id: allocateId(usedIds, `${id}_values`),
                text: column.outputValues,
              },
      };
      outputs.splice(insertionIndex, 0, output);
      for (const [rowIndex, rule] of (table.rules ?? []).entries()) {
        const entries = (rule.outputEntries ??= []);
        const ruleStem = rule.id ?? `Rule_${rowIndex + 1}`;
        entries.splice(insertionIndex, 0, {
          id: allocateId(usedIds, `${ruleStem}_${id}`),
          text: null,
          typeRef,
        });
      }
      renumberTable(table);
    },
  };
}

export function updateOutputColumnCommand(
  decisionId: string,
  index: number,
  changes: OutputColumnChanges,
): DmnCommand {
  return {
    label: `Update output column ${index + 1}`,
    apply(document) {
      const table = requireTable(document.model, decisionId);
      assertTableShape(table);
      const output = requireAt(table.outputs ?? [], index, 'output column');
      if (changes.id !== undefined && changes.id !== output.id) {
        assertNonBlankId(changes.id, 'output column id');
        assertCanonicalIdAvailable(document.model, changes.id, output.id ?? undefined);
        output.id = changes.id;
      }
      if (changes.label !== undefined) output.label = changes.label;
      if (changes.name !== undefined) output.name = changes.name;
      if (changes.typeRef !== undefined) {
        const typeRef = checkedTypeRef(changes.typeRef);
        if (table.hitPolicy === 'COLLECT' && table.collectOperator && typeRef !== 'number') {
          throw new DmnCommandError(
            'invalid-collect-operator',
            'An aggregated COLLECT output must use typeRef number',
          );
        }
        output.typeRef = typeRef;
      }
      if (changes.outputValues !== undefined) {
        output.outputValues = setUnaryTestsText(
          document.model,
          output.outputValues,
          changes.outputValues,
          `${output.id ?? 'OutputClause'}_values`,
        );
      }
    },
  };
}

export function deleteOutputColumnCommand(decisionId: string, index: number): DmnCommand {
  return {
    label: `Delete output column ${index + 1}`,
    apply(document) {
      const table = requireTable(document.model, decisionId);
      assertTableShape(table);
      const outputs = table.outputs ?? [];
      requireAt(outputs, index, 'output column');
      if (outputs.length === 1) {
        throw new DmnCommandError('last-output', 'A decision table must keep at least one output');
      }
      outputs.splice(index, 1);
      for (const rule of table.rules ?? []) rule.outputEntries?.splice(index, 1);
      renumberTable(table);
    },
  };
}

export function addRuleCommand(decisionId: string, id?: string, index?: number): DmnCommand {
  return {
    label: `Add rule to ${decisionId}`,
    apply(document) {
      const table = requireTable(document.model, decisionId);
      assertTableShape(table);
      const rules = (table.rules ??= []);
      const insertionIndex = insertionPoint(index, rules.length);
      const usedIds = collectCanonicalIds(document.model);
      const ruleId = allocateOrAssertId(usedIds, id, 'DecisionRule');
      const rule: Draft<DecisionRule> = {
        id: ruleId,
        ruleNumber: insertionIndex + 1,
        inputEntries: (table.inputs ?? []).map((input, columnIndex) => ({
          id: allocateId(usedIds, `${ruleId}_${input.id ?? `InputClause_${columnIndex + 1}`}`),
          text: null,
        })),
        outputEntries: (table.outputs ?? []).map((output, columnIndex) => ({
          id: allocateId(usedIds, `${ruleId}_${output.id ?? `OutputClause_${columnIndex + 1}`}`),
          text: null,
          typeRef: output.typeRef ?? null,
        })),
      };
      rules.splice(insertionIndex, 0, rule);
      renumberTable(table);
    },
  };
}

export function deleteRuleCommand(decisionId: string, index: number): DmnCommand {
  return {
    label: `Delete rule ${index + 1}`,
    apply(document) {
      const table = requireTable(document.model, decisionId);
      assertTableShape(table);
      const rules = table.rules ?? [];
      requireAt(rules, index, 'rule');
      if (rules.length === 1) {
        throw new DmnCommandError('last-rule', 'A decision table must keep at least one rule');
      }
      rules.splice(index, 1);
      renumberTable(table);
    },
  };
}

export function moveRuleCommand(
  decisionId: string,
  fromIndex: number,
  toIndex: number,
): DmnCommand {
  return {
    label: `Move rule ${fromIndex + 1}`,
    apply(document) {
      const table = requireTable(document.model, decisionId);
      assertTableShape(table);
      const rules = table.rules ?? [];
      const rule = requireAt(rules, fromIndex, 'rule');
      requireAt(rules, toIndex, 'rule');
      if (fromIndex === toIndex) return;
      rules.splice(fromIndex, 1);
      rules.splice(toIndex, 0, rule);
      renumberTable(table);
    },
  };
}

export function editCellCommand(
  decisionId: string,
  address: DmnCellAddress,
  value: DmnCellValue,
): DmnCommand {
  return {
    label: `Edit ${address.kind} cell ${address.row + 1}:${address.column + 1}`,
    apply(document) {
      const table = requireTable(document.model, decisionId);
      assertTableShape(table);
      const rule = requireAt(table.rules ?? [], address.row, 'rule');
      if (address.kind === 'input') {
        if (value.typeRef !== undefined) {
          throw new DmnCommandError(
            'invalid-value-type',
            'Input cell types are declared by the input expression column',
          );
        }
        const entry = requireAt(rule.inputEntries ?? [], address.column, 'input cell');
        entry.text = value.text;
        return;
      }
      const entry = requireAt(rule.outputEntries ?? [], address.column, 'output cell');
      entry.text = value.text;
      if (value.typeRef !== undefined) entry.typeRef = checkedTypeRef(value.typeRef);
    },
  };
}

export function setHitPolicyCommand(
  decisionId: string,
  hitPolicy: CreatableHitPolicy,
  collectOperator: CollectOperator | null = null,
): DmnCommand {
  return {
    label: `Set hit policy ${hitPolicy}`,
    apply(document) {
      const table = requireTable(document.model, decisionId);
      if (!isCreatableHitPolicy(hitPolicy as HitPolicy)) {
        throw new DmnCommandError(
          'unsupported-hit-policy',
          `Hit policy ${String(hitPolicy)} cannot be newly selected`,
        );
      }
      if (hitPolicy !== 'COLLECT') {
        if (collectOperator !== null) {
          throw new DmnCommandError(
            'invalid-collect-operator',
            'A collect operator is only valid with COLLECT',
          );
        }
        table.hitPolicy = hitPolicy;
        table.collectOperator = null;
        return;
      }
      if (collectOperator !== null && !COLLECT_OPERATORS.includes(collectOperator)) {
        throw new DmnCommandError(
          'invalid-collect-operator',
          `Unsupported collect operator ${String(collectOperator)}`,
        );
      }
      if (collectOperator !== null) assertAggregatedCollectShape(table);
      table.hitPolicy = hitPolicy;
      table.collectOperator = collectOperator;
    },
  };
}

function requireDecision(definition: Draft<DmnDefinition>, decisionId: string): Draft<Decision> {
  const decision = definition.decisions?.find((candidate) => candidate.id === decisionId);
  if (!decision) {
    throw new DmnCommandError('decision-not-found', `Decision ${decisionId} does not exist`);
  }
  return decision;
}

function requireTable(definition: Draft<DmnDefinition>, decisionId: string): Draft<DecisionTable> {
  return requireDecision(definition, decisionId).decisionTable;
}

function requireAt<T>(values: Draft<T>[], index: number, label: string): Draft<T> {
  if (!Number.isInteger(index) || index < 0 || index >= values.length) {
    throw new DmnCommandError('invalid-index', `${label} index ${index} is out of range`);
  }
  return values[index] as Draft<T>;
}

function insertionPoint(index: number | undefined, length: number): number {
  if (index === undefined) return length;
  if (!Number.isInteger(index) || index < 0 || index > length) {
    throw new DmnCommandError('invalid-index', `Insertion index ${index} is out of range`);
  }
  return index;
}

function assertTableShape(table: Draft<DecisionTable>) {
  const inputCount = table.inputs?.length ?? 0;
  const outputCount = table.outputs?.length ?? 0;
  for (const [index, rule] of (table.rules ?? []).entries()) {
    if (
      (rule.inputEntries?.length ?? 0) !== inputCount ||
      (rule.outputEntries?.length ?? 0) !== outputCount
    ) {
      throw new DmnCommandError(
        'invalid-table-shape',
        `Rule ${rule.id ?? index + 1} does not match the table columns`,
      );
    }
  }
}

function assertAggregatedCollectShape(table: Draft<DecisionTable>) {
  const outputs = table.outputs ?? [];
  if (outputs.length !== 1 || outputs[0]?.typeRef !== 'number') {
    throw new DmnCommandError(
      'invalid-collect-operator',
      'An aggregated COLLECT table requires exactly one number output',
    );
  }
}

function renumberTable(table: Draft<DecisionTable>) {
  table.inputs?.forEach((input, index) => {
    input.inputNumber = index + 1;
  });
  table.outputs?.forEach((output, index) => {
    output.outputNumber = index + 1;
  });
  table.rules?.forEach((rule, index) => {
    rule.ruleNumber = index + 1;
  });
}

function checkedTypeRef(typeRef: DmnValueTypeRef | null | undefined): DmnValueTypeRef | null {
  if (typeRef === undefined || typeRef === null) return null;
  if (!isDmnValueTypeRef(typeRef)) {
    throw new DmnCommandError('invalid-value-type', `Unsupported DMN typeRef ${String(typeRef)}`);
  }
  return typeRef;
}

function assertNonBlankId(id: string, label: string) {
  if (id.trim().length === 0) {
    throw new DmnCommandError('invalid-id', `${label} must not be blank`);
  }
}

function assertDecisionKeyAvailable(definition: Draft<DmnDefinition>, key: string) {
  if (definition.decisions?.some((decision) => decision.id === key)) {
    throw new DmnCommandError('duplicate-id', `Decision key ${key} already exists`);
  }
}

function assertCanonicalIdAvailable(
  definition: Draft<DmnDefinition>,
  id: string,
  currentId?: string,
) {
  const ids = collectCanonicalIds(definition);
  if (currentId) ids.delete(currentId);
  if (ids.has(id)) throw new DmnCommandError('duplicate-id', `Canonical id ${id} already exists`);
}

function allocateOrAssertId(usedIds: Set<string>, preferred: string | undefined, prefix: string) {
  if (preferred !== undefined) {
    assertNonBlankId(preferred, `${prefix} id`);
    if (usedIds.has(preferred)) {
      throw new DmnCommandError('duplicate-id', `Canonical id ${preferred} already exists`);
    }
    usedIds.add(preferred);
    return preferred;
  }
  return allocateId(usedIds, prefix);
}

function allocateId(usedIds: Set<string>, prefix: string): string {
  let candidate = prefix;
  let suffix = 2;
  while (usedIds.has(candidate)) {
    candidate = `${prefix}_${suffix}`;
    suffix += 1;
  }
  usedIds.add(candidate);
  return candidate;
}

function collectCanonicalIds(definition: Draft<DmnDefinition>): Set<string> {
  const ids = new Set<string>();
  addId(ids, definition.id);
  for (const decision of definition.decisions ?? []) {
    addId(ids, decision.id);
    addId(ids, decision.decisionTable.id);
    for (const input of decision.decisionTable.inputs ?? []) {
      addId(ids, input.id);
      addId(ids, input.inputExpression.id);
    }
    for (const output of decision.decisionTable.outputs ?? []) {
      addId(ids, output.id);
      addId(ids, output.outputValues?.id);
    }
    for (const rule of decision.decisionTable.rules ?? []) {
      addId(ids, rule.id);
      for (const entry of rule.inputEntries ?? []) addId(ids, entry.id);
      for (const entry of rule.outputEntries ?? []) addId(ids, entry.id);
    }
  }
  for (const service of definition.decisionServices ?? []) addId(ids, service.id);
  for (const source of definition.knowledgeSources ?? []) addId(ids, source.id);
  for (const requirement of definition.authorityRequirements ?? []) addId(ids, requirement.id);
  return ids;
}

function addId(ids: Set<string>, id: string | null | undefined) {
  if (id) ids.add(id);
}

function replaceDecisionReferences(
  definition: Draft<DmnDefinition>,
  previousId: string,
  nextId: string,
) {
  for (const decision of definition.decisions ?? []) {
    decision.requiredDecisions = replaceReferences(decision.requiredDecisions, previousId, nextId);
  }
  for (const service of definition.decisionServices ?? []) {
    service.requiredDecisions = replaceReferences(service.requiredDecisions, previousId, nextId);
    service.outputDecisions = replaceReferences(service.outputDecisions, previousId, nextId);
  }
  for (const requirement of definition.authorityRequirements ?? []) {
    if (requirement.requiredDecision === previousId) requirement.requiredDecision = nextId;
    if (requirement.decision === previousId) requirement.decision = nextId;
  }
}

function replaceReferences(
  references: Draft<string[]> | undefined,
  previousId: string,
  nextId: string,
): Draft<string[]> {
  return (references ?? []).map((reference) => (reference === previousId ? nextId : reference));
}

function setUnaryTestsText(
  definition: Draft<DmnDefinition>,
  current: Draft<UnaryTests> | null | undefined,
  text: string | null,
  idPrefix: string,
): Draft<UnaryTests> | null {
  if (text === null) return null;
  return {
    id: current?.id ?? allocateId(collectCanonicalIds(definition), idPrefix),
    text,
  };
}
