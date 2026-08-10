import { describe, expect, it } from 'vitest';

import {
  collectModelIds,
  validateConditionExpression,
  validateElementId,
  validateNumericValue,
} from './propertyValidation';
import { sampleDocument } from './sampleDocument';

describe('collectModelIds', () => {
  it('covers processes, elements, artifacts, pools, and lanes', () => {
    const ids = collectModelIds(sampleDocument);
    for (const expected of [
      'leaveProcess',
      'review',
      'requestFlow',
      'reviewTimer',
      'approvalNote',
      'approvalGroup',
      'approvalLink',
      'leavePool',
      'managerLane',
    ]) {
      expect(ids.has(expected), expected).toBe(true);
    }
    expect(ids.has('ghost')).toBe(false);
  });
});

describe('validateElementId', () => {
  it('requires a non-blank id without whitespace', () => {
    expect(validateElementId(sampleDocument, 'review', '')).toBe('ID is required');
    expect(validateElementId(sampleDocument, 'review', '   ')).toBe('ID is required');
    expect(validateElementId(sampleDocument, 'review', 'has space')).toBe(
      'ID must not contain whitespace',
    );
  });

  it('rejects ids already used by any diagram element', () => {
    expect(validateElementId(sampleDocument, 'review', 'notify')).toBe(
      'ID "notify" is already used',
    );
    expect(validateElementId(sampleDocument, 'review', 'approvalGroup')).toBe(
      'ID "approvalGroup" is already used',
    );
  });

  it('accepts the current id and fresh ids', () => {
    expect(validateElementId(sampleDocument, 'review', 'review')).toBeNull();
    expect(validateElementId(sampleDocument, 'review', 'audit')).toBeNull();
  });
});

describe('validateNumericValue', () => {
  it('allows clearing and whole numbers, rejects other text', () => {
    expect(validateNumericValue('')).toBeNull();
    expect(validateNumericValue('  ')).toBeNull();
    expect(validateNumericValue('50')).toBeNull();
    expect(validateNumericValue('-1')).toBeNull();
    expect(validateNumericValue('1.5')).toBe('Enter a whole number');
    expect(validateNumericValue('abc')).toBe('Enter a whole number');
  });
});

describe('validateConditionExpression', () => {
  it('allows empty conditions and balanced UEL text', () => {
    expect(validateConditionExpression('')).toBeNull();
    expect(validateConditionExpression('${approved}')).toBeNull();
    expect(validateConditionExpression('${(a + b) > 2}')).toBeNull();
  });

  it('rejects unbalanced brackets without parsing the expression', () => {
    expect(validateConditionExpression('${approved')).toBe('Brackets are not balanced');
    expect(validateConditionExpression('${a > } b}')).toBe('Brackets are not balanced');
    expect(validateConditionExpression('${(a}')).toBe('Brackets are not balanced');
  });
});
