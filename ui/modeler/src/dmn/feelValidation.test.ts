import { describe, expect, it } from 'vitest';

import { validateFeelExpression, validateUnaryTests } from './feelValidation';

describe('validateUnaryTests', () => {
  it.each([
    '',
    '   ',
    '-',
    '"manager"',
    '42',
    '-3.5',
    'true',
    '= "gold"',
    '== 5',
    '!= 5',
    '> 18',
    '<= 65',
    '[1..10]',
    '(1..10]',
    '[1..10)',
    '(1..10)',
    '"a", "b", "c"',
    '1, 2, 3',
    'not("x")',
    'not(1, 2)',
    'not([1..10])',
    'contains(?, "ab")',
    'starts with(?, "A")',
    'ends with(?, "z")',
    'matches(?, "^a.*$")',
    'list contains(?, roles)',
    'lower case(?) = "abc"',
    'upper case(?) = "ABC"',
    'string length(?) > 5',
    '? in (1, 2, 3)',
    'in ("a", "b")',
    '${leaveDays > 3}',
    '#{role == "manager"}',
    '.premium',
    'customer.tier',
    'leaveDays',
    '2024-01-31',
    'P10D',
    'date("2024-01-31")',
    'duration("P2Y")',
    '> fn_now()',
    '<= fn_addDate("P1D")',
  ])('accepts the subset form %j', (text) => {
    expect(validateUnaryTests(text)).toBeNull();
  });

  it.each([
    'a && b',
    'x || y',
    'foo();',
    '${a} ${b}',
    'not(',
    'not()',
    '[1..10',
    '1..10]',
    '>',
    '=',
    '"unterminated',
    'contains(?, "a"',
    'unknown predicate(?)',
    'in ()',
    '?, 5',
  ])('rejects the out-of-subset form %j', (text) => {
    expect(validateUnaryTests(text)).toEqual(expect.any(String));
  });
});

describe('validateFeelExpression', () => {
  it.each([
    '',
    '   ',
    '"APPROVED"',
    '42',
    'amount * 0.2',
    'price - discount',
    'total / count',
    '2 ** 10',
    'amount % 2',
    'a > 1 and b < 2',
    'x or y',
    'not(approved)',
    'if leaveDays > 5 then "long" else "short"',
    'mean(scores)',
    'sum(items)',
    'string length(name)',
    'upper case(status)',
    'list contains(roles, "admin")',
    'fn_addDate("P1D")',
    'date:now()',
    '[1..10]',
    '[1, 2, 3]',
    '{status: "ok", count: 2}',
    'some item in items satisfies item > 0',
    'for x in items return x * 2',
    'customer.name',
  ])('accepts the subset expression %j', (text) => {
    expect(validateFeelExpression(text)).toBeNull();
  });

  it.each([
    'a && b',
    'x || y',
    'foo();',
    'x ! y',
    '${approved}',
    '#{a > 1}',
    '"unterminated',
    '(1 + 2',
    '1 + 2)',
    'a = `b`',
    'x; y',
  ])('rejects the out-of-subset expression %j', (text) => {
    expect(validateFeelExpression(text)).toEqual(expect.any(String));
  });
});
