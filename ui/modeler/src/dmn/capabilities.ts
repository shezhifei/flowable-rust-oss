import type { CollectOperator, HitPolicy } from '../generated/editor-protocol';

/**
 * UI hints mirrored from `flowable_dmn_engine::editor_capabilities`.
 *
 * These lists constrain choices the editor can create. They are not a
 * browser-side semantic validator; persisted documents still go through the
 * Rust validation boundary.
 */
export const CREATABLE_HIT_POLICIES = [
  'FIRST',
  'UNIQUE',
  'ANY',
  'COLLECT',
  'RULE_ORDER',
  'OUTPUT_ORDER',
  'PRIORITY',
] as const satisfies readonly HitPolicy[];

export const ROUND_TRIP_HIT_POLICIES = [
  ...CREATABLE_HIT_POLICIES,
  'COMPLETE',
] as const satisfies readonly HitPolicy[];

export const COLLECT_OPERATORS = [
  'COUNT',
  'SUM',
  'MIN',
  'MAX',
] as const satisfies readonly CollectOperator[];

export const VALUE_TYPE_REFS = [
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
] as const;

export type CreatableHitPolicy = (typeof CREATABLE_HIT_POLICIES)[number];
export type DmnValueTypeRef = (typeof VALUE_TYPE_REFS)[number];

export function isCreatableHitPolicy(value: HitPolicy): value is CreatableHitPolicy {
  return CREATABLE_HIT_POLICIES.some((candidate) => candidate === value);
}

export function isDmnValueTypeRef(value: string): value is DmnValueTypeRef {
  return VALUE_TYPE_REFS.some((candidate) => candidate === value);
}
