import type { FormFieldModel } from '../generated/editor-protocol';

/**
 * UI hints mirrored from `flowable_form_service::field_types` and frozen by
 * `docs/plans/modeler-form-field-contract.md`.
 *
 * These lists constrain choices the designer can create. They are not a
 * browser-side authority; persisted documents still go through the Rust
 * `validate_form_model` boundary on save and deploy.
 */
export type FormFieldCategory =
  | 'value'
  | 'option'
  | 'identity'
  | 'expression'
  | 'container'
  | 'display';

export type FormFieldVariant = FormFieldModel['fieldType'];

export interface FormFieldCapability {
  wireType: string;
  label: string;
  category: FormFieldCategory;
  requiredVariant: FormFieldVariant;
  /** False for expression/container/display fields: they are never submitted. */
  writable: boolean;
  supportsRequired: boolean;
  supportsPlaceholder: boolean;
  glyph: string;
}

const CAPABILITY_LIST: readonly FormFieldCapability[] = [
  { wireType: 'text', label: 'Text', category: 'value', requiredVariant: 'BaseField', writable: true, supportsRequired: true, supportsPlaceholder: true, glyph: 'T' },
  { wireType: 'multi-line-text', label: 'Multiline text', category: 'value', requiredVariant: 'BaseField', writable: true, supportsRequired: true, supportsPlaceholder: true, glyph: '¶' },
  { wireType: 'integer', label: 'Integer', category: 'value', requiredVariant: 'BaseField', writable: true, supportsRequired: true, supportsPlaceholder: true, glyph: '#' },
  { wireType: 'decimal', label: 'Decimal', category: 'value', requiredVariant: 'BaseField', writable: true, supportsRequired: true, supportsPlaceholder: true, glyph: '∴' },
  { wireType: 'amount', label: 'Amount', category: 'value', requiredVariant: 'BaseField', writable: true, supportsRequired: true, supportsPlaceholder: true, glyph: '$' },
  { wireType: 'date', label: 'Date', category: 'value', requiredVariant: 'BaseField', writable: true, supportsRequired: true, supportsPlaceholder: false, glyph: '◷' },
  { wireType: 'boolean', label: 'Checkbox', category: 'value', requiredVariant: 'BaseField', writable: true, supportsRequired: true, supportsPlaceholder: false, glyph: '☑' },
  { wireType: 'radio-buttons', label: 'Radio buttons', category: 'option', requiredVariant: 'OptionFormField', writable: true, supportsRequired: true, supportsPlaceholder: false, glyph: '◉' },
  { wireType: 'dropdown', label: 'Dropdown', category: 'option', requiredVariant: 'OptionFormField', writable: true, supportsRequired: true, supportsPlaceholder: true, glyph: '▾' },
  { wireType: 'upload', label: 'Upload', category: 'value', requiredVariant: 'BaseField', writable: true, supportsRequired: true, supportsPlaceholder: false, glyph: '⇧' },
  { wireType: 'expression', label: 'Expression', category: 'expression', requiredVariant: 'ExpressionFormField', writable: false, supportsRequired: false, supportsPlaceholder: false, glyph: 'ƒ' },
  { wireType: 'people', label: 'People', category: 'identity', requiredVariant: 'BaseField', writable: true, supportsRequired: true, supportsPlaceholder: true, glyph: '♟' },
  { wireType: 'functional-group', label: 'Group', category: 'identity', requiredVariant: 'BaseField', writable: true, supportsRequired: true, supportsPlaceholder: true, glyph: '⚏' },
  { wireType: 'container', label: 'Container', category: 'container', requiredVariant: 'Container', writable: false, supportsRequired: false, supportsPlaceholder: false, glyph: '▣' },
  { wireType: 'hyperlink', label: 'Hyperlink', category: 'display', requiredVariant: 'BaseField', writable: false, supportsRequired: false, supportsPlaceholder: false, glyph: '🔗' },
  { wireType: 'spacer', label: 'Spacer', category: 'display', requiredVariant: 'BaseField', writable: false, supportsRequired: false, supportsPlaceholder: false, glyph: '␣' },
  { wireType: 'horizontal-line', label: 'Horizontal line', category: 'display', requiredVariant: 'BaseField', writable: false, supportsRequired: false, supportsPlaceholder: false, glyph: '―' },
  { wireType: 'headline', label: 'Headline', category: 'display', requiredVariant: 'BaseField', writable: false, supportsRequired: false, supportsPlaceholder: false, glyph: 'H' },
  { wireType: 'headline-with-line', label: 'Headline with line', category: 'display', requiredVariant: 'BaseField', writable: false, supportsRequired: false, supportsPlaceholder: false, glyph: 'Ħ' },
] as const;

const CAPABILITY_BY_TYPE = new Map(CAPABILITY_LIST.map((entry) => [entry.wireType, entry]));

/**
 * Runtime compatibility aliases from `legacy_field_capability`. Aliases are
 * accepted in imported documents but are never offered on the palette.
 */
const LEGACY_ALIAS_TARGET: Record<string, string> = {
  string: 'text',
  long: 'integer',
  double: 'decimal',
  float: 'decimal',
  number: 'decimal',
  enum: 'dropdown',
  radio: 'radio-buttons',
};

/** The exact Flowable 6.8 wire types, in contract order. */
export const FLOWABLE_6_8_FIELD_TYPES: readonly string[] = CAPABILITY_LIST.map(
  (entry) => entry.wireType,
);

/** Resolve an exact 6.8 wire value or a documented compatibility alias. */
export function formFieldCapability(
  fieldType: string | null | undefined,
): FormFieldCapability | undefined {
  if (!fieldType) return undefined;
  const direct = CAPABILITY_BY_TYPE.get(fieldType);
  if (direct) return direct;
  const aliasTarget = LEGACY_ALIAS_TARGET[fieldType.trim().toLowerCase()];
  return aliasTarget ? CAPABILITY_BY_TYPE.get(aliasTarget) : undefined;
}

/** Whether the wire type is one of the exact 6.8 palette values. */
export function isPaletteFieldType(fieldType: string): boolean {
  return CAPABILITY_BY_TYPE.has(fieldType);
}

export interface FormPaletteGroup {
  id: string;
  label: string;
  wireTypes: readonly string[];
}

/** Palette grouping: value / option / identity / display / container. */
export const FORM_PALETTE_GROUPS: readonly FormPaletteGroup[] = [
  {
    id: 'value',
    label: 'Value fields',
    wireTypes: ['text', 'multi-line-text', 'integer', 'decimal', 'amount', 'date', 'boolean', 'upload'],
  },
  { id: 'option', label: 'Choice fields', wireTypes: ['radio-buttons', 'dropdown'] },
  { id: 'identity', label: 'Identity fields', wireTypes: ['people', 'functional-group'] },
  {
    id: 'display',
    label: 'Display fields',
    wireTypes: ['expression', 'hyperlink', 'spacer', 'horizontal-line', 'headline', 'headline-with-line'],
  },
  { id: 'container', label: 'Layout', wireTypes: ['container'] },
] as const;

export function paletteCapability(wireType: string): FormFieldCapability {
  const capability = CAPABILITY_BY_TYPE.get(wireType);
  if (!capability) throw new Error(`Unknown palette field type '${wireType}'`);
  return capability;
}

/** Field types that carry an editable static option list. */
export function isOptionFieldType(fieldType: string | null | undefined): boolean {
  return formFieldCapability(fieldType)?.requiredVariant === 'OptionFormField';
}

/** Field types whose values are never submitted (expression/display/container). */
export function isSubmittableFieldType(fieldType: string | null | undefined): boolean {
  return formFieldCapability(fieldType)?.writable ?? true;
}
