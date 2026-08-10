import type { FormEditorDocument, FormFieldModel } from '../generated/editor-protocol';
import { formFieldCapability } from './capabilities';

/**
 * Editor hint layer mirroring `flowable_form_service::validation`.
 *
 * The Rust `validate_form_model` boundary remains the authority on save and
 * deploy; this validator only gives the designer immediate feedback with the
 * same stable codes so messages line up with server rejections.
 */
export const FORM_FIELD_ID_REQUIRED = 'flowable-form-field-id-required';
export const FORM_FIELD_ID_DUPLICATE = 'flowable-form-field-id-duplicate';
export const MISSING_FIELD_TYPE = 'flowable-form-field-type-required';
export const UNSUPPORTED_FIELD_TYPE = 'flowable-form-field-type-unsupported';
export const INCOMPATIBLE_FIELD_VARIANT = 'flowable-form-field-variant-incompatible';
export const INCOMPATIBLE_WRITEABILITY = 'flowable-form-field-writeability-incompatible';
export const INVALID_OPTIONS = 'flowable-form-field-options-invalid';
export const DYNAMIC_OPTIONS_UNSUPPORTED = 'flowable-form-dynamic-options-unsupported';
export const INVALID_EXPRESSION = 'flowable-form-field-expression-invalid';
export const INVALID_CONTAINER = 'flowable-form-field-container-invalid';
export const INVALID_LAYOUT = 'flowable-form-field-layout-invalid';

export interface FormValidationIssue {
  elementId: string | null;
  code: string;
  message: string;
}

export function validateFormDocument(document: FormEditorDocument): FormValidationIssue[] {
  const issues: FormValidationIssue[] = [];
  const model = document.model;
  if (!model.key.trim()) {
    issues.push({ elementId: null, code: 'flowable-form-key-required', message: 'form key is required' });
  }
  if (!model.name.trim()) {
    issues.push({ elementId: null, code: 'flowable-form-name-required', message: 'form name is required' });
  }
  const ids = new Set<string>();
  validateFields(model.fields ?? [], ids, issues);
  return issues;
}

function validateFields(
  fields: FormFieldModel[],
  ids: Set<string>,
  issues: FormValidationIssue[],
) {
  for (const field of fields) {
    validateId(field, ids, issues);
    validateLayout(field, issues);
    const fieldType = field.type?.trim() ? field.type : null;
    if (!fieldType) {
      push(issues, field, MISSING_FIELD_TYPE, 'form field type is required');
      continue;
    }
    const capability = formFieldCapability(fieldType);
    if (!capability) {
      push(issues, field, UNSUPPORTED_FIELD_TYPE, `form field type \`${fieldType}\` is not supported`);
      continue;
    }
    if (capability.requiredVariant !== field.fieldType) {
      push(
        issues,
        field,
        INCOMPATIBLE_FIELD_VARIANT,
        `form field type \`${fieldType}\` requires ${capability.requiredVariant}, not ${field.fieldType}`,
      );
    }
    validateWriteability(field, capability.writable, capability.supportsRequired, issues);

    if (field.fieldType === 'Container') {
      const rows = field.fields ?? [];
      if (rows.some((row) => row.length === 0)) {
        push(issues, field, INVALID_CONTAINER, 'form container rows must not be empty');
      }
      for (const row of rows) validateFields(row, ids, issues);
    } else if (field.fieldType === 'OptionFormField') {
      validateOptions(field, issues);
    } else if (field.fieldType === 'ExpressionFormField') {
      if (!field.expression.trim()) {
        push(issues, field, INVALID_EXPRESSION, 'expression form field requires a non-empty expression');
      } else if (!hasBalancedUelSegments(field.expression)) {
        push(issues, field, INVALID_EXPRESSION, 'expression form field contains an unclosed `${...}` segment');
      }
    }
  }
}

function validateId(field: FormFieldModel, ids: Set<string>, issues: FormValidationIssue[]) {
  const id = field.id.trim();
  if (!id) {
    push(issues, field, FORM_FIELD_ID_REQUIRED, 'form field id is required');
  } else if (ids.has(id)) {
    push(issues, field, FORM_FIELD_ID_DUPLICATE, `duplicate form field id \`${id}\``);
  } else {
    ids.add(id);
  }
}

function validateWriteability(
  field: FormFieldModel,
  writableType: boolean,
  supportsRequired: boolean,
  issues: FormValidationIssue[],
) {
  if (field.readOnly === true && field.writable === true) {
    push(issues, field, INCOMPATIBLE_WRITEABILITY, 'read-only form field cannot also be writable');
  } else if (!writableType && field.writable === true) {
    push(
      issues,
      field,
      INCOMPATIBLE_WRITEABILITY,
      'structural, expression, and display fields cannot be writable',
    );
  }
  if (!supportsRequired && field.required === true) {
    push(
      issues,
      field,
      INCOMPATIBLE_WRITEABILITY,
      'structural, expression, and display fields cannot be required',
    );
  }
}

function validateOptions(field: FormFieldModel, issues: FormValidationIssue[]) {
  if (field.fieldType !== 'OptionFormField') return;
  const optionType = field.optionType?.trim();
  if (optionType && optionType !== 'static' && optionType !== (field.type ?? '')) {
    push(issues, field, INVALID_OPTIONS, 'optionType must be `static` or match the option field type');
  }
  const optionIds = new Set<string>();
  for (const option of field.options ?? []) {
    if (!option.id.trim() || !option.name.trim()) {
      push(issues, field, INVALID_OPTIONS, 'option id and name must both be non-empty');
    } else if (optionIds.has(option.id.trim())) {
      push(issues, field, INVALID_OPTIONS, `duplicate option id \`${option.id.trim()}\``);
    } else {
      optionIds.add(option.id.trim());
    }
  }
  const hasDynamicOptions = Boolean(field.optionsExpression?.trim());
  if (hasDynamicOptions) {
    push(
      issues,
      field,
      DYNAMIC_OPTIONS_UNSUPPORTED,
      'dynamic optionsExpression is not supported by this runtime; use static options',
    );
  } else if ((field.options ?? []).length === 0) {
    push(issues, field, INVALID_OPTIONS, 'option form field requires at least one static option');
  }
}

function validateLayout(field: FormFieldModel, issues: FormValidationIssue[]) {
  const layout = field.layout;
  if (!layout) return;
  const rowInvalid = layout.row != null && layout.row < 0;
  const colInvalid = layout.col != null && layout.col < 0;
  const colSpanInvalid = layout.colSpan != null && layout.colSpan <= 0;
  if (rowInvalid || colInvalid || colSpanInvalid) {
    push(
      issues,
      field,
      INVALID_LAYOUT,
      'layout row/col must be non-negative and colSpan must be positive',
    );
  }
}

function hasBalancedUelSegments(expression: string): boolean {
  let index = 0;
  while (index + 1 < expression.length) {
    if (expression[index] !== '$' || expression[index + 1] !== '{') {
      index += 1;
      continue;
    }
    index += 2;
    let depth = 1;
    while (index < expression.length && depth > 0) {
      if (expression[index] === '{') depth += 1;
      else if (expression[index] === '}') depth -= 1;
      index += 1;
    }
    if (depth !== 0) return false;
  }
  return true;
}

function push(issues: FormValidationIssue[], field: FormFieldModel, code: string, message: string) {
  issues.push({ elementId: field.id.trim() ? field.id : null, code, message });
}
