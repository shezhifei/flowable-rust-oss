# Modeler DMN expression and validation contract

The first-party DMN editor does not implement a browser-side FEEL parser. Rust
is the semantic authority for decision-table input unary tests, output
expressions, type coercion, hit-policy constraints, and deployment validation.

The public boundary is `flowable_dmn_engine::editor_capabilities`,
`validate_editor_definition`, `validate_editor_expression`, and
`evaluate_editor_expression`. The capability lists are editor hints; accepting
or rejecting a document or expression always goes through those functions.

## Validation flow

`validate_editor_definition` accepts the canonical
`flowable_dmn_model::DmnDefinition` used by the JSON protocol. It clones and
converts the definition through the production `DmnModel::try_from` path, then
runs the same side-effect-free gates as deployment:

1. parse every input unary test;
2. check COLLECT aggregation shape before output coercion;
3. normalize and validate declared input type references;
4. normalize and validate static output values and output type references;
5. validate decision, rule, output, priority, and deployment structure.

Only the clone is normalized. The editor document is never mutated by
validation. No in-memory database is created and no deployment or history state
is written.

The Modeler service also performs DMN XML writer/parser round-trip validation.
A document is valid only when both the engine semantic gate and the converter
gate succeed.

## Input unary tests

Decision-table input cells use the engine's unary-test parser, not the general
output-expression parser. The supported forms are:

- blank or `-` (match any);
- literal equality, `=`, `==`, and `!=`;
- `<`, `<=`, `>`, and `>=` comparisons;
- open and closed ranges such as `[1..10]`, `(1..10]`, `[1..10)`, and
  `(1..10)`;
- comma-separated alternatives;
- `not(<supported test>)`, including comma-separated literal arguments;
- `contains`, `starts with`, `ends with`, and `matches` predicates using `?`;
- `lower case(?)` and `upper case(?)` equality and `string length(?)`
  comparisons;
- `substring` and `replace` transforms;
- `list contains(?, literal-or-variable)` and `? in (...)` / `in (...)`;
- one complete `${...}` or `#{...}` boolean condition;
- `.property` and nested property-path shorthand;
- temporal and duration literals;
- comparisons against `fn_date`, `fn_now`, `fn_addDate`, and
  `fn_subtractDate` aliases.

Declared input `typeRef` is authoritative. Numeric and temporal input types
normalize supported literal tests and reject incompatible values during model
validation, before persistence.

## Output expressions

Output cells execute through `FeelExpressionEngine`. It first uses the typed
FEEL parser and then the existing compatibility evaluator for the long-tail
function catalogue. The combined operator surface is:

```text
+  -  *  /  **  %
and  or
=  !=  <  <=  >  >=  in
```

The typed layer also supports lists, contexts, ranges, `if/then/else`, paths,
filters, `for`, `some`, and `every` expressions.

Supported function spellings are:

```text
abs, ceiling, ceil, floor, round, sqrt, modulo, decimal, even, odd
contains, starts with, ends with, matches, string length, upper case,
lower case, substring, replace, trim
append, concatenate, count, distinct values, flatten, reverse,
list contains, index of, sublist, union, intersect, except
sum, mean, min, max
now, today, fn_date, fn_now, fn_addDate, fn_subtractDate,
date:toDate, date:now, date:addDate, date:subtractDate,
year, month, day, hour, minute, second
```

Expression validation requires a caller-supplied sample context. This is
intentional: a valid expression such as `mean(scores)` cannot be evaluated
against an empty variable map. Whole-document validation therefore validates
unary tests, types, and deployment structure; an interactive preview validates
and evaluates output expressions with sample values for every referenced
input.

## Hit policies and type references

New decision tables may offer:

```text
FIRST, UNIQUE, ANY, COLLECT, RULE_ORDER, OUTPUT_ORDER, PRIORITY
```

Canonical imports may additionally contain `COMPLETE`. The editor must display,
validate, and save imported `COMPLETE` unchanged, but it does not offer it for a
new table. The engine-internal `BATCH` policy is not part of the canonical DMN
editor protocol.

COLLECT aggregations are `COUNT`, `SUM`, `MIN`, and `MAX`. An aggregation uses
exactly one output and that output must declare numeric `typeRef` `number`,
matching the deployment validator.

Editor value types are:

```text
string, boolean, integer, long, double, number,
date, time, dateTime, duration, dayTimeDuration, yearMonthDuration,
context, list
```

## Change discipline

A change to FEEL, unary-test, hit-policy, or type support must update all of the
following in one change:

1. the production parser/evaluator or deployment validator;
2. `flowable-dmn-engine::editor` capability constants;
3. editor validation tests, including a rejection case;
4. this document;
5. generated editor protocol artifacts when the canonical JSON shape changes.

The browser must never infer acceptance from this document alone and must never
silently rewrite an unsupported expression or imported hit policy.
