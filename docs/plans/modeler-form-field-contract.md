# Flowable 6.8 Form field contract

This document freezes the Form builder/runtime boundary used by stream C. It
is intentionally a wire contract, not a second form model: imported `type`
strings remain unchanged in `BaseFormField.field_type`, and capability lookup
is performed separately when a document is validated, deployed, or submitted.

## Java 6.8 evidence

The authoritative baseline is tag `flowable-6.8.0` (`0052eb63aee1d831c3a527c2c64c96cbae7a4eaa`):

- [`FormFieldTypes.java`](https://github.com/flowable/flowable-engine/blob/flowable-6.8.0/modules/flowable-form-model/src/main/java/org/flowable/form/model/FormFieldTypes.java)
  declares the exact persisted values.
- [`FormField.java`](https://github.com/flowable/flowable-engine/blob/flowable-6.8.0/modules/flowable-form-model/src/main/java/org/flowable/form/model/FormField.java),
  [`OptionFormField.java`](https://github.com/flowable/flowable-engine/blob/flowable-6.8.0/modules/flowable-form-model/src/main/java/org/flowable/form/model/OptionFormField.java),
  [`ExpressionFormField.java`](https://github.com/flowable/flowable-engine/blob/flowable-6.8.0/modules/flowable-form-model/src/main/java/org/flowable/form/model/ExpressionFormField.java), and
  [`FormContainer.java`](https://github.com/flowable/flowable-engine/blob/flowable-6.8.0/modules/flowable-form-model/src/main/java/org/flowable/form/model/FormContainer.java)
  define the polymorphic variants.
- [`SimpleFormModel.listAllFields`](https://github.com/flowable/flowable-engine/blob/flowable-6.8.0/modules/flowable-form-model/src/main/java/org/flowable/form/model/SimpleFormModel.java)
  recursively flattens container rows.
- [`GetVariablesFromFormSubmissionCmd`](https://github.com/flowable/flowable-engine/blob/flowable-6.8.0/modules/flowable-form-engine/src/main/java/org/flowable/form/engine/impl/cmd/GetVariablesFromFormSubmissionCmd.java)
  skips expression/container fields and defines amount, option, people, and
  functional-group submission coercion.
- [`AbstractGetFormInstanceModelCmd`](https://github.com/flowable/flowable-engine/blob/flowable-6.8.0/modules/flowable-form-engine/src/main/java/org/flowable/form/engine/impl/cmd/AbstractGetFormInstanceModelCmd.java)
  evaluates expression fields and treats layout fields as rendered rather than
  submitted values.

## Exact public wire values

`flowable_form_service::FLOWABLE_6_8_FIELD_TYPES` contains exactly:

| Wire value | Category | Required Rust/Java variant | Runtime behavior |
| --- | --- | --- | --- |
| `text` | value | `BaseField` | text handler |
| `multi-line-text` | value | `BaseField` | text handler |
| `integer` | value | `BaseField` | integral number handler |
| `decimal` | value | `BaseField` | decimal number handler |
| `amount` | value | `BaseField` | decimal number handler |
| `date` | value | `BaseField` | date handler |
| `boolean` | value | `BaseField` | boolean handler |
| `radio-buttons` | option | `OptionFormField` | option object/string to selected id/name |
| `dropdown` | option | `OptionFormField` | option object/string to selected id/name |
| `upload` | value | `BaseField` | upload lifecycle handler |
| `expression` | expression | `ExpressionFormField` | evaluated for display, never submitted |
| `people` | identity | `BaseField` | identity object/string to id |
| `functional-group` | identity | `BaseField` | group object/string to id |
| `container` | container | `Container` | recursively flattens rows, never submitted |
| `hyperlink` | display | `BaseField` | static/composite URL display, never submitted |
| `spacer` | display | `BaseField` | display only |
| `horizontal-line` | display | `BaseField` | display only |
| `headline` | display | `BaseField` | display only |
| `headline-with-line` | display | `BaseField` | display only |

The Flowable 6.8 list does **not** contain `checkbox`. A boolean checkbox is
persisted as `boolean`; a literal `checkbox` type is preserved by decode/encode
but rejected by save/deploy as unsupported.

## Runtime aliases

Aliases are compatibility routes only. They never rewrite persisted JSON:

| Imported value | Runtime route / canonical semantics |
| --- | --- |
| `string` | `text` |
| `long` | `integer` |
| `double`, `float`, `number` | `decimal` |
| `enum` | `dropdown` / option handler |
| `radio` | `radio-buttons` / option handler |

The exact 6.8 values also route without mutation: `multi-line-text → text`,
`amount → decimal`, and `radio-buttons → radio`.

## Save and deployment validation

`flowable_form_service::validate_form_model` is the shared recursive validator.
The modeler maps its issues into `/validate` and PUT validation; form deployment
runs the same contract before inserting deployment/definition rows. It checks:

- field id presence and global uniqueness across all container depths;
- non-empty, supported `type`, while leaving the original string untouched;
- `type`/`fieldType` variant compatibility;
- option id/name presence, duplicate ids, and static-options policy;
- non-empty expression definitions;
- container row shape and recursive children;
- layout (`row/col >= 0`, `colSpan > 0`);
- `readOnly`/`writable` contradictions and non-writable structural/display
  fields;
- required compatibility (expression/container/display cannot be required).

Stable codes are included in messages, including:

- `flowable-form-field-type-required`
- `flowable-form-field-type-unsupported`
- `flowable-form-field-variant-incompatible`
- `flowable-form-field-writeability-incompatible`
- `flowable-form-field-options-invalid`
- `flowable-form-dynamic-options-unsupported`
- `flowable-form-field-expression-invalid`
- `flowable-form-field-container-invalid`
- `flowable-form-field-layout-invalid`

Dynamic `optionsExpression` is deliberately rejected before save/deploy in this
runtime. Static options are fully supported. This avoids persisting a form that
can only fail later when the task form is opened or submitted. Registered custom
runtime handlers remain deployable through `FlowableFormService::with_handlers`;
the modeler, which has no runtime handler registry, rejects unknown types.

## Read-only and unsupported policy

Expression, container, hyperlink, spacer, horizontal-line, headline, and
headline-with-line fields are never placed in `FormData.form_properties`, so a
client cannot submit them accidentally. They remain in the faithful nested
`form_fields` projection for rendering. Expression and hyperlink values are
evaluated on the read path from process variables. Explicit `writable: true` or
`required: true` on these categories is a validation error.

Unknown `type` strings still deserialize and serialize byte-for-value at the
JSON property level. `decode_form_json` therefore remains an import/inspection
boundary; `validate_form`, modeler PUT, and deployment are the authority gates.

## Verification

Primary contract tests:

```powershell
cargo test -p flowable-form-service --test form_field_contract_test
cargo test -p flowable-form-service
cargo test -p flowable-modeler-service
cargo clippy -p flowable-form-service -p flowable-modeler-service --all-targets -- -D warnings
```
