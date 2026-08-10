import type { Draft } from 'immer';

import type {
  FormEditorDocument,
  FormFieldModel,
  FormModel,
  FormOption,
  FormOutcome,
} from '../generated/editor-protocol';
import { formFieldCapability, paletteCapability } from './capabilities';

export interface FormCommand {
  label: string;
  apply: (document: Draft<FormEditorDocument>) => void;
}

export type FormCommandErrorCode =
  | 'field-not-found'
  | 'invalid-id'
  | 'duplicate-id'
  | 'invalid-index'
  | 'invalid-type'
  | 'invalid-options';

export class FormCommandError extends Error {
  constructor(
    public readonly code: FormCommandErrorCode,
    message: string,
  ) {
    super(message);
    this.name = 'FormCommandError';
  }
}

export interface FieldPropertyChanges {
  id?: string;
  name?: string | null;
  placeholder?: string | null;
  required?: boolean | null;
  readOnly?: boolean | null;
  datePattern?: string | null;
  expression?: string;
  hyperlinkUrl?: string | null;
}

export interface OptionChanges {
  id?: string;
  name?: string;
}

export interface FormPropertyChanges {
  key?: string;
  name?: string;
  description?: string | null;
  outcomeVariableName?: string | null;
}

/** Location of a field inside the recursive field tree. */
interface FieldLocation {
  field: Draft<FormFieldModel>;
  /** Sibling list containing the field (top-level list or container row). */
  siblings: Draft<FormFieldModel>[];
  index: number;
  /** Owning container rows when the field lives inside a container. */
  containerRows?: Draft<FormFieldModel>[][];
  rowIndex?: number;
}

export function updateFormPropertiesCommand(changes: FormPropertyChanges): FormCommand {
  return {
    label: 'Update form properties',
    apply(document) {
      if (changes.key !== undefined && changes.key !== document.model.key) {
        assertNonBlank(changes.key, 'Form key');
        document.model.key = changes.key.trim();
      }
      if (changes.name !== undefined && changes.name !== document.model.name) {
        assertNonBlank(changes.name, 'Form name');
        document.model.name = changes.name.trim();
      }
      if (changes.description !== undefined) document.model.description = changes.description;
      if (changes.outcomeVariableName !== undefined) {
        document.model.outcomeVariableName = changes.outcomeVariableName;
      }
    },
  };
}

export function addFieldCommand(
  wireType: string,
  target: { containerId?: string | null; id?: string } = {},
): FormCommand {
  const capability = paletteCapability(wireType);
  return {
    label: `Add ${capability.label} field`,
    apply(document) {
      const usedIds = collectFieldIds(document.model);
      let id: string;
      if (target.id) {
        assertFieldIdAvailable(document.model, target.id, '');
        id = target.id;
      } else {
        id = allocateId(usedIds, camelCase(wireType));
      }
      usedIds.add(id);
      const field = createFieldDraft(wireType, id, usedIds);
      if (target.containerId) {
        const container = locateField(document.model, target.containerId);
        if (!container || container.field.fieldType !== 'Container') {
          throw new FormCommandError(
            'field-not-found',
            `Container '${target.containerId}' is not part of this form`,
          );
        }
        const rows = (container.field.fields ??= []);
        rows.push([field]);
      } else {
        (document.model.fields ??= []).push(field);
      }
    },
  };
}

export function removeFieldCommand(fieldId: string): FormCommand {
  return {
    label: `Remove field ${fieldId}`,
    apply(document) {
      const location = requireField(document.model, fieldId);
      location.siblings.splice(location.index, 1);
      // A container row that lost its last field must not persist as an
      // empty row: the wire contract rejects those.
      if (
        location.containerRows &&
        location.rowIndex !== undefined &&
        location.siblings.length === 0
      ) {
        location.containerRows.splice(location.rowIndex, 1);
      }
    },
  };
}

/**
 * Moves a field up or down. Top-level fields swap with their neighbours;
 * fields inside a container move with their whole row, matching the
 * row-oriented container wire shape.
 */
export function moveFieldCommand(fieldId: string, offset: -1 | 1): FormCommand {
  return {
    label: `Move field ${fieldId}`,
    apply(document) {
      const location = requireField(document.model, fieldId);
      if (location.containerRows && location.rowIndex !== undefined) {
        const rows = location.containerRows;
        const targetIndex = location.rowIndex + offset;
        if (targetIndex < 0 || targetIndex >= rows.length) {
          throw new FormCommandError('invalid-index', 'The field is already at the edge');
        }
        const [row] = rows.splice(location.rowIndex, 1);
        rows.splice(targetIndex, 0, row!);
        return;
      }
      const targetIndex = location.index + offset;
      if (targetIndex < 0 || targetIndex >= location.siblings.length) {
        throw new FormCommandError('invalid-index', 'The field is already at the edge');
      }
      const [field] = location.siblings.splice(location.index, 1);
      location.siblings.splice(targetIndex, 0, field!);
    },
  };
}

export function updateFieldPropertiesCommand(
  fieldId: string,
  changes: FieldPropertyChanges,
): FormCommand {
  return {
    label: `Update field ${fieldId}`,
    apply(document) {
      const location = requireField(document.model, fieldId);
      const field = location.field;
      if (changes.id !== undefined && changes.id !== field.id) {
        assertNonBlank(changes.id, 'Field id');
        assertFieldIdAvailable(document.model, changes.id.trim(), fieldId);
        field.id = changes.id.trim();
      }
      if (changes.name !== undefined) field.name = changes.name;
      if (changes.placeholder !== undefined) field.placeholder = changes.placeholder;
      if (changes.required !== undefined) field.required = changes.required;
      if (changes.readOnly !== undefined) field.readOnly = changes.readOnly;
      if (changes.datePattern !== undefined) field.datePattern = changes.datePattern;
      if (changes.expression !== undefined) {
        if (field.fieldType !== 'ExpressionFormField') {
          throw new FormCommandError(
            'invalid-type',
            `Field '${fieldId}' does not carry an expression`,
          );
        }
        field.expression = changes.expression;
      }
      if (changes.hyperlinkUrl !== undefined) {
        const params = { ...(field.params ?? {}) };
        if (changes.hyperlinkUrl && changes.hyperlinkUrl.trim()) {
          params.url = changes.hyperlinkUrl.trim();
        } else {
          delete params.url;
        }
        field.params = Object.keys(params).length ? params : null;
      }
    },
  };
}

export function addOptionCommand(fieldId: string): FormCommand {
  return {
    label: `Add option to ${fieldId}`,
    apply(document) {
      const field = requireOptionField(document.model, fieldId);
      const options = (field.options ??= []);
      const usedIds = new Set(options.map((option) => option.id));
      const id = allocateId(usedIds, 'option');
      options.push({ id, name: `Option ${options.length + 1}` });
    },
  };
}

export function updateOptionCommand(
  fieldId: string,
  optionIndex: number,
  changes: OptionChanges,
): FormCommand {
  return {
    label: `Update option ${optionIndex + 1} of ${fieldId}`,
    apply(document) {
      const field = requireOptionField(document.model, fieldId);
      const options = field.options ?? [];
      const option = options[optionIndex];
      if (!option) {
        throw new FormCommandError('invalid-index', `Option ${optionIndex + 1} does not exist`);
      }
      if (changes.id !== undefined && changes.id !== option.id) {
        assertNonBlank(changes.id, 'Option id');
        const id = changes.id.trim();
        if (options.some((candidate, index) => index !== optionIndex && candidate.id === id)) {
          throw new FormCommandError('duplicate-id', `Option id '${id}' is already used`);
        }
        option.id = id;
      }
      if (changes.name !== undefined) option.name = changes.name;
    },
  };
}

export function removeOptionCommand(fieldId: string, optionIndex: number): FormCommand {
  return {
    label: `Remove option ${optionIndex + 1} of ${fieldId}`,
    apply(document) {
      const field = requireOptionField(document.model, fieldId);
      const options = field.options ?? [];
      if (optionIndex < 0 || optionIndex >= options.length) {
        throw new FormCommandError('invalid-index', `Option ${optionIndex + 1} does not exist`);
      }
      options.splice(optionIndex, 1);
    },
  };
}

export function moveOptionCommand(
  fieldId: string,
  optionIndex: number,
  offset: -1 | 1,
): FormCommand {
  return {
    label: `Move option ${optionIndex + 1} of ${fieldId}`,
    apply(document) {
      const field = requireOptionField(document.model, fieldId);
      const options = field.options ?? [];
      const targetIndex = optionIndex + offset;
      if (optionIndex < 0 || optionIndex >= options.length) {
        throw new FormCommandError('invalid-index', `Option ${optionIndex + 1} does not exist`);
      }
      if (targetIndex < 0 || targetIndex >= options.length) {
        throw new FormCommandError('invalid-index', 'The option is already at the edge');
      }
      const [option] = options.splice(optionIndex, 1);
      options.splice(targetIndex, 0, option!);
    },
  };
}

export function addOutcomeCommand(): FormCommand {
  return {
    label: 'Add outcome',
    apply(document) {
      const outcomes = (document.model.outcomes ??= []);
      const usedIds = new Set(
        outcomes.map((outcome) => outcome.id).filter((id): id is string => Boolean(id)),
      );
      const id = allocateId(usedIds, 'outcome');
      outcomes.push({ id, name: `Outcome ${outcomes.length + 1}` });
    },
  };
}

export function updateOutcomeCommand(
  outcomeIndex: number,
  changes: { name?: string | null },
): FormCommand {
  return {
    label: `Update outcome ${outcomeIndex + 1}`,
    apply(document) {
      const outcome = requireOutcome(document.model, outcomeIndex);
      if (changes.name !== undefined) outcome.name = changes.name;
    },
  };
}

export function removeOutcomeCommand(outcomeIndex: number): FormCommand {
  return {
    label: `Remove outcome ${outcomeIndex + 1}`,
    apply(document) {
      requireOutcome(document.model, outcomeIndex);
      document.model.outcomes?.splice(outcomeIndex, 1);
    },
  };
}

export function moveOutcomeCommand(outcomeIndex: number, offset: -1 | 1): FormCommand {
  return {
    label: `Move outcome ${outcomeIndex + 1}`,
    apply(document) {
      const outcomes = document.model.outcomes ?? [];
      requireOutcome(document.model, outcomeIndex);
      const targetIndex = outcomeIndex + offset;
      if (targetIndex < 0 || targetIndex >= outcomes.length) {
        throw new FormCommandError('invalid-index', 'The outcome is already at the edge');
      }
      const [outcome] = outcomes.splice(outcomeIndex, 1);
      outcomes.splice(targetIndex, 0, outcome!);
    },
  };
}

/** Read-side helpers shared by the canvas, properties panel, and preview. */

export function findField(model: FormModel, fieldId: string): FormFieldModel | null {
  return locateField(model, fieldId)?.field ?? null;
}

export function allFields(model: FormModel): FormFieldModel[] {
  const collected: FormFieldModel[] = [];
  const visit = (fields: FormFieldModel[]) => {
    for (const field of fields) {
      collected.push(field);
      if (field.fieldType === 'Container') {
        for (const row of field.fields ?? []) visit(row);
      }
    }
  };
  visit(model.fields ?? []);
  return collected;
}

function locateField(model: Draft<FormModel>, fieldId: string): FieldLocation | null {
  const topLevel = model.fields ?? [];
  const fromTop = locateInList(topLevel, fieldId);
  if (fromTop) return fromTop;

  const stack: Draft<FormFieldModel>[][] = [topLevel];
  while (stack.length) {
    const list = stack.pop()!;
    for (const field of list) {
      if (field.fieldType !== 'Container') continue;
      const rows = field.fields ?? [];
      for (const [rowIndex, row] of rows.entries()) {
        const located = locateInList(row, fieldId);
        if (located) {
          return { ...located, containerRows: rows, rowIndex };
        }
        stack.push(row);
      }
    }
  }
  return null;
}

function locateInList(
  siblings: Draft<FormFieldModel>[],
  fieldId: string,
): FieldLocation | null {
  const index = siblings.findIndex((field) => field.id === fieldId);
  if (index < 0) return null;
  return { field: siblings[index]!, siblings, index };
}

function requireField(model: Draft<FormModel>, fieldId: string): FieldLocation {
  const location = locateField(model, fieldId);
  if (!location) {
    throw new FormCommandError('field-not-found', `Field '${fieldId}' is not part of this form`);
  }
  return location;
}

function requireOptionField(
  model: Draft<FormModel>,
  fieldId: string,
): Draft<FormFieldModel> & { options?: Draft<FormOption>[] } {
  const location = requireField(model, fieldId);
  if (location.field.fieldType !== 'OptionFormField') {
    throw new FormCommandError('invalid-type', `Field '${fieldId}' does not carry options`);
  }
  return location.field;
}

function requireOutcome(model: Draft<FormModel>, outcomeIndex: number): Draft<FormOutcome> {
  const outcome = (model.outcomes ?? [])[outcomeIndex];
  if (!outcome) {
    throw new FormCommandError('invalid-index', `Outcome ${outcomeIndex + 1} does not exist`);
  }
  return outcome;
}

function createFieldDraft(
  wireType: string,
  id: string,
  usedIds: Set<string>,
): Draft<FormFieldModel> {
  const capability = paletteCapability(wireType);
  switch (capability.requiredVariant) {
    case 'OptionFormField':
      // Seed one static option so a freshly created choice field stays valid.
      return {
        fieldType: 'OptionFormField',
        id,
        type: wireType,
        name: null,
        options: [{ id: allocateId(usedIds, `${id}_option`), name: 'Option 1' }],
      };
    case 'ExpressionFormField':
      return { fieldType: 'ExpressionFormField', id, type: wireType, name: null, expression: '' };
    case 'Container':
      return { fieldType: 'Container', id, type: wireType, name: null, fields: [] };
    default:
      return { fieldType: 'BaseField', id, type: wireType, name: null };
  }
}

function collectFieldIds(model: Draft<FormModel>): Set<string> {
  const ids = new Set<string>();
  const visit = (fields: Draft<FormFieldModel>[]) => {
    for (const field of fields) {
      if (field.id) ids.add(field.id);
      if (field.fieldType === 'Container') {
        for (const row of field.fields ?? []) visit(row);
      }
    }
  };
  visit(model.fields ?? []);
  return ids;
}

function assertFieldIdAvailable(model: Draft<FormModel>, id: string, currentFieldId: string) {
  if (currentFieldId === id) return;
  if (collectFieldIds(model).has(id)) {
    throw new FormCommandError('duplicate-id', `Field id '${id}' is already used`);
  }
}

function assertNonBlank(value: string, label: string) {
  if (!value.trim()) {
    throw new FormCommandError('invalid-id', `${label} must not be blank`);
  }
}

function allocateId(usedIds: Set<string>, stem: string): string {
  let counter = 1;
  let candidate = `${stem}${counter}`;
  while (usedIds.has(candidate)) {
    counter += 1;
    candidate = `${stem}${counter}`;
  }
  usedIds.add(candidate);
  return candidate;
}

function camelCase(wireType: string): string {
  return wireType.replace(/-([a-z])/g, (_, letter: string) => letter.toUpperCase());
}

/** Convenience for the properties panel: capability of the selected field. */
export function fieldCapability(field: FormFieldModel) {
  return formFieldCapability(field.type);
}

/**
 * Predicts the id the next `addFieldCommand` will allocate for a palette
 * type, so callers can select the new field right after executing.
 */
export function nextFieldId(model: FormModel, wireType: string): string {
  return allocateId(collectFieldIds(model), camelCase(wireType));
}
