/**
 * Browser-side gate for the DMN editor FEEL subset described in
 * `docs/plans/modeler-dmn-feel-subset.md`.
 *
 * Rust remains the semantic authority; these checks are editor hints that
 * keep obvious out-of-subset syntax from ever entering the document. Each
 * validator returns `null` for an acceptable draft or a short message
 * describing the first problem.
 */

const UNARY_COMPARISONS = ['<=', '>=', '!=', '==', '<', '>', '='] as const;

const UNARY_PREDICATES = [
  'contains',
  'starts with',
  'ends with',
  'matches',
  'list contains',
] as const;

const VALUE_FUNCTIONS = [
  'date',
  'time',
  'date and time',
  'duration',
  'fn_date',
  'fn_now',
  'fn_addDate',
  'fn_subtractDate',
] as const;

/** Validates a decision-table input cell (FEEL unary tests subset). */
export function validateUnaryTests(text: string): string | null {
  const trimmed = text.trim();
  if (trimmed === '' || trimmed === '-') return null;

  if (trimmed.startsWith('${') || trimmed.startsWith('#{')) {
    return validateElCondition(trimmed);
  }

  const balanceError = checkBalanced(trimmed);
  if (balanceError) return balanceError;

  if (trimmed.startsWith('not(')) {
    if (!trimmed.endsWith(')')) return 'not(...) must end with a closing parenthesis';
    const inner = trimmed.slice(4, -1).trim();
    if (inner === '') return 'not(...) requires at least one unary test';
    return validateUnaryTests(inner);
  }

  const parts = splitTopLevel(trimmed);
  for (const part of parts) {
    const error = validateUnaryTest(part.trim());
    if (error) return error;
  }
  return null;
}

/** Validates a decision-table output cell (FEEL expression subset). */
export function validateFeelExpression(text: string): string | null {
  const trimmed = text.trim();
  if (trimmed === '') return null;
  return scanFeelTokens(trimmed);
}

function validateElCondition(text: string): string | null {
  const opener = text.slice(0, 2);
  if (!text.endsWith('}')) return `${opener}...} conditions must end with }`;
  const rest = text.slice(2);
  if (rest.includes('${') || rest.includes('#{')) {
    return 'Only one complete ${...} or #{...} condition is allowed per cell';
  }
  const inner = rest.slice(0, -1).trim();
  if (inner === '') return 'The condition must not be empty';
  return null;
}

function validateUnaryTest(part: string): string | null {
  if (part === '') return 'Comma-separated entries must not be empty';

  const range = /^[[(](.+)\.\.(.+)[\])]$/.exec(part);
  if (range) {
    const [, lower = '', upper = ''] = range;
    return validateUnaryValue(lower.trim()) ?? validateUnaryValue(upper.trim());
  }
  if ((part.startsWith('[') || part.startsWith('(')) && part.includes('..')) {
    return 'Ranges must look like [1..10] or (1..10]';
  }

  const inList = /^(?:\?\s*)?in\s*\((.*)\)$/s.exec(part);
  if (inList) {
    const inner = (inList[1] ?? '').trim();
    if (inner === '') return 'in (...) requires at least one value';
    for (const value of splitTopLevel(inner)) {
      const error = validateUnaryValue(value.trim());
      if (error) return error;
    }
    return null;
  }

  for (const operator of UNARY_COMPARISONS) {
    if (part.startsWith(operator)) {
      const value = part.slice(operator.length).trim();
      if (value === '') return `A ${operator} comparison needs a value`;
      return validateUnaryValue(value);
    }
  }

  if (part.includes('?')) return validateQuestionTest(part);

  return validateUnaryValue(part);
}

function validateQuestionTest(part: string): string | null {
  const call = /^([A-Za-z][A-Za-z ]*?)\s*\((.*)\)(.*)$/s.exec(part);
  if (!call) return 'Unsupported unary test';
  const [, name = '', args = '', tail = ''] = call;
  const normalized = name.trim().toLowerCase();
  const known =
    (UNARY_PREDICATES as readonly string[]).includes(normalized) ||
    normalized === 'lower case' ||
    normalized === 'upper case' ||
    normalized === 'string length' ||
    normalized === 'substring' ||
    normalized === 'replace';
  if (!known) return `Unsupported unary predicate ${name.trim()}`;
  const inner = args.trim();
  if (!inner.startsWith('?')) return `${name.trim()} tests start with the ? input marker`;
  const remainder = inner.slice(1).trim();
  if (remainder !== '') {
    if (!remainder.startsWith(',')) return `Unsupported arguments for ${name.trim()}`;
    for (const value of splitTopLevel(remainder.slice(1))) {
      const error = validateUnaryValue(value.trim());
      if (error) return error;
    }
  }
  const trailing = tail.trim();
  if (trailing === '') return null;
  for (const operator of UNARY_COMPARISONS) {
    if (trailing.startsWith(operator)) {
      const value = trailing.slice(operator.length).trim();
      if (value === '') return `A ${operator} comparison needs a value`;
      return validateUnaryValue(value);
    }
  }
  return `Unsupported trailing comparison after ${name.trim()}(...)`;
}

function validateUnaryValue(value: string): string | null {
  if (value === '') return 'Missing value';
  if (/^"(?:[^"\\]|\\.)*"$/.test(value)) return null;
  if (value.startsWith('"')) return 'Unterminated string literal';
  if (/^[+-]?\d+(\.\d+)?$/.test(value)) return null;
  if (value === 'true' || value === 'false') return null;
  if (/^\d{4}-\d{2}-\d{2}(T[\d:.]+(Z|[+-]\d{2}:?\d{2})?)?$/.test(value)) return null;
  if (/^[+-]?P(\d+Y)?(\d+M)?(\d+D)?(T(\d+H)?(\d+M)?(\d+S)?)?$/.test(value)) return null;

  const call = /^([A-Za-z][A-Za-z_]*(?:\s+[a-z]+)*)\s*\((.*)\)$/s.exec(value);
  if (call) {
    const name = (call[1] ?? '').trim();
    if ((VALUE_FUNCTIONS as readonly string[]).includes(name)) {
      const inner = (call[2] ?? '').trim();
      if (inner === '') return null;
      for (const argument of splitTopLevel(inner)) {
        const error = validateUnaryValue(argument.trim());
        if (error) return error;
      }
      return null;
    }
    return `Unsupported function ${name} in a unary test`;
  }

  // Variable references, including the `.property` shorthand and nested paths.
  if (/^\.?[A-Za-z_][\w]*( *[.][A-Za-z_][\w]*)*$/.test(value)) return null;

  return `Unsupported value ${value}`;
}

/**
 * Scans an output expression token by token. Identifiers are always accepted
 * (they may name input variables); anything outside the documented operator
 * and punctuation surface is rejected.
 */
function scanFeelTokens(text: string): string | null {
  const stack: string[] = [];
  let index = 0;
  while (index < text.length) {
    const char = text.charAt(index);
    if (/\s/.test(char)) {
      index += 1;
      continue;
    }
    if (char === '"') {
      const end = findStringEnd(text, index);
      if (end === -1) return 'Unterminated string literal';
      index = end + 1;
      continue;
    }
    if (/\d/.test(char)) {
      while (index < text.length && /\d/.test(text.charAt(index))) index += 1;
      if (text.charAt(index) === '.' && /\d/.test(text.charAt(index + 1))) {
        index += 1;
        while (index < text.length && /\d/.test(text.charAt(index))) index += 1;
      }
      continue;
    }
    if (/[A-Za-z_]/.test(char)) {
      while (index < text.length && /\w/.test(text.charAt(index))) index += 1;
      continue;
    }
    if (char === '$' || char === '#') {
      return 'Output expressions do not support ${...} or #{...} conditions';
    }
    if (char === '!') {
      if (text.charAt(index + 1) !== '=') return 'Only != is supported; use not(...) for negation';
      index += 2;
      continue;
    }
    if (char === '<' || char === '>' || char === '=') {
      index += text.charAt(index + 1) === '=' ? 2 : 1;
      continue;
    }
    if (char === '*') {
      index += text.charAt(index + 1) === '*' ? 2 : 1;
      continue;
    }
    if ('+-/%'.includes(char)) {
      index += 1;
      continue;
    }
    if (char === '.') {
      index += text.charAt(index + 1) === '.' ? 2 : 1;
      continue;
    }
    const open = '([{'.indexOf(char);
    if (open !== -1) {
      stack.push(')]}'.charAt(open));
      index += 1;
      continue;
    }
    const close = ')]}'.indexOf(char);
    if (close !== -1) {
      if (stack.pop() !== char) return `Unexpected ${char}`;
      index += 1;
      continue;
    }
    if (char === ',') {
      index += 1;
      continue;
    }
    if (char === ':') {
      // Context entries ({status: "ok"}) and namespaced functions (date:now()).
      index += 1;
      continue;
    }
    return `Unsupported character '${char}' in a FEEL expression`;
  }
  if (stack.length) return `Unclosed bracket; expected ${stack.at(-1)}`;
  return null;
}

function checkBalanced(text: string): string | null {
  const stack: string[] = [];
  let index = 0;
  while (index < text.length) {
    const char = text.charAt(index);
    if (char === '"') {
      const end = findStringEnd(text, index);
      if (end === -1) return 'Unterminated string literal';
      index = end + 1;
      continue;
    }
    const open = '([{'.indexOf(char);
    if (open !== -1) stack.push(')]}'.charAt(open));
    else if (char === '}' && stack.pop() !== '}') return 'Unexpected }';
    else if (')]'.includes(char)) {
      // Unary-test ranges legitimately mix bracket styles: [1..10) or (1..10].
      const expected = stack.pop();
      if (expected !== ')' && expected !== ']') return `Unexpected ${char}`;
    }
    index += 1;
  }
  if (stack.length) return `Unclosed bracket; expected ${stack.at(-1)}`;
  return null;
}

/** Splits on top-level commas, ignoring strings and nested brackets. */
function splitTopLevel(text: string): string[] {
  const parts: string[] = [];
  const stack: string[] = [];
  let start = 0;
  let index = 0;
  while (index < text.length) {
    const char = text.charAt(index);
    if (char === '"') {
      const end = findStringEnd(text, index);
      index = end === -1 ? text.length : end + 1;
      continue;
    }
    const open = '([{'.indexOf(char);
    if (open !== -1) stack.push(')]}'.charAt(open));
    else if (')]}'.includes(char)) stack.pop();
    else if (char === ',' && stack.length === 0) {
      parts.push(text.slice(start, index));
      start = index + 1;
    }
    index += 1;
  }
  parts.push(text.slice(start));
  return parts;
}

function findStringEnd(text: string, start: number): number {
  for (let index = start + 1; index < text.length; index += 1) {
    const char = text.charAt(index);
    if (char === '\\') index += 1;
    else if (char === '"') return index;
  }
  return -1;
}
