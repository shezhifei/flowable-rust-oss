import { useState } from 'react';
import { useStore } from 'zustand';

import type { FormFieldModel } from '../../generated/editor-protocol';
import {
  addOptionCommand,
  addOutcomeCommand,
  fieldCapability,
  findField,
  moveOptionCommand,
  moveOutcomeCommand,
  removeFieldCommand,
  removeOptionCommand,
  removeOutcomeCommand,
  updateFieldPropertiesCommand,
  updateFormPropertiesCommand,
  updateOptionCommand,
  updateOutcomeCommand,
  type FormEditorStore,
  type FormValidationIssue,
} from '../index';
import { executeFormCommand } from './editing';

export interface FormPropertiesPanelProps {
  issues: FormValidationIssue[];
  store: FormEditorStore;
}

export function FormPropertiesPanel({ issues, store }: FormPropertiesPanelProps) {
  const document = useStore(store, (state) => state.document);
  const selectedFieldId = useStore(store, (state) => state.selectedFieldId);
  const field = selectedFieldId ? findField(document.model, selectedFieldId) : null;

  if (!field) {
    return <GeneralProperties issues={issues} store={store} />;
  }
  return (
    <FieldProperties
      key={field.id}
      field={field}
      issues={issues.filter((issue) => issue.elementId === field.id)}
      store={store}
    />
  );
}

function GeneralProperties({
  issues,
  store,
}: {
  issues: FormValidationIssue[];
  store: FormEditorStore;
}) {
  const document = useStore(store, (state) => state.document);
  const model = document.model;
  const modelIssues = issues.filter((issue) => issue.elementId === null);

  return (
    <aside className="properties-panel" aria-label="Form properties">
      <PanelHeading kicker="Form" title={model.name || 'Untitled form'} glyph="▤" />
      <div className="property-groups" data-panel-state="form-general">
        {modelIssues.length ? (
          <section>
            <h2>Validation</h2>
            <ul className="property-issue-list">
              {modelIssues.map((issue, index) => (
                <li key={index} role="alert">
                  {issue.message}
                </li>
              ))}
            </ul>
          </section>
        ) : null}
        <section>
          <h2>General</h2>
          <TextProperty
            property="formName"
            label="Name"
            value={model.name}
            validate={validateNonBlank('Form name')}
            onCommit={(draft) =>
              executeFormCommand(store, updateFormPropertiesCommand({ name: draft }))
            }
          />
          <TextProperty
            property="formKey"
            label="Key"
            value={model.key}
            validate={validateNonBlank('Form key')}
            onCommit={(draft) =>
              executeFormCommand(store, updateFormPropertiesCommand({ key: draft }))
            }
          />
          <TextProperty
            property="formDescription"
            label="Description"
            value={model.description ?? ''}
            onCommit={(draft) =>
              executeFormCommand(
                store,
                updateFormPropertiesCommand({ description: draft.trim() || null }),
              )
            }
          />
          <TextProperty
            property="outcomeVariableName"
            label="Outcome variable"
            value={model.outcomeVariableName ?? ''}
            onCommit={(draft) =>
              executeFormCommand(
                store,
                updateFormPropertiesCommand({ outcomeVariableName: draft.trim() || null }),
              )
            }
          />
        </section>
        <OutcomesSection store={store} />
      </div>
    </aside>
  );
}

function OutcomesSection({ store }: { store: FormEditorStore }) {
  const document = useStore(store, (state) => state.document);
  const outcomes = document.model.outcomes ?? [];

  return (
    <section>
      <h2>Outcomes</h2>
      {outcomes.length === 0 ? (
        <p className="property-note">No outcomes. The form submits with a default complete action.</p>
      ) : (
        outcomes.map((outcome, index) => (
          <OutcomeRow
            key={outcome.id ?? `outcome-${index}`}
            index={index}
            name={outcome.name ?? ''}
            outcomeCount={outcomes.length}
            store={store}
          />
        ))
      )}
      <button
        type="button"
        className="quiet-action"
        onClick={() => executeFormCommand(store, addOutcomeCommand())}
      >
        + Add outcome
      </button>
    </section>
  );
}

function OutcomeRow({
  index,
  name,
  outcomeCount,
  store,
}: {
  index: number;
  name: string;
  outcomeCount: number;
  store: FormEditorStore;
}) {
  return (
    <div className="outcome-row">
      <TextProperty
        property={`outcomeName${index + 1}`}
        label={`Outcome ${index + 1}`}
        value={name}
        onCommit={(draft) =>
          executeFormCommand(store, updateOutcomeCommand(index, { name: draft.trim() || null }))
        }
      />
      <span className="outcome-row-actions">
        <button
          type="button"
          aria-label={`Move outcome ${index + 1} up`}
          disabled={index === 0}
          onClick={() => executeFormCommand(store, moveOutcomeCommand(index, -1))}
        >
          ↑
        </button>
        <button
          type="button"
          aria-label={`Move outcome ${index + 1} down`}
          disabled={index === outcomeCount - 1}
          onClick={() => executeFormCommand(store, moveOutcomeCommand(index, 1))}
        >
          ↓
        </button>
        <button
          type="button"
          aria-label={`Delete outcome ${index + 1}`}
          onClick={() => executeFormCommand(store, removeOutcomeCommand(index))}
        >
          ×
        </button>
      </span>
    </div>
  );
}

function FieldProperties({
  field,
  issues,
  store,
}: {
  field: FormFieldModel;
  issues: FormValidationIssue[];
  store: FormEditorStore;
}) {
  const capability = fieldCapability(field);
  const isContainer = field.fieldType === 'Container';
  const isExpression = field.fieldType === 'ExpressionFormField';
  const isOption = field.fieldType === 'OptionFormField';
  const isHyperlink = field.type === 'hyperlink';
  const isDate = field.type === 'date';
  const hyperlinkUrl = field.params?.url ?? '';

  return (
    <aside className="properties-panel" aria-label="Form field properties">
      <PanelHeading
        kicker={capability?.label ?? field.type ?? 'Field'}
        title={field.name?.trim() ? field.name : field.id}
        glyph={capability?.glyph ?? '?'}
      />
      <div className="property-groups" data-panel-state="form-field">
        {issues.length ? (
          <section>
            <h2>Validation</h2>
            <ul className="property-issue-list">
              {issues.map((issue, index) => (
                <li key={index} role="alert">
                  {issue.message}
                </li>
              ))}
            </ul>
          </section>
        ) : null}
        <section>
          <h2>Field</h2>
          <TextProperty
            property="fieldId"
            label="ID"
            value={field.id}
            validate={validateNonBlank('Field id')}
            onCommit={(draft) =>
              executeFormCommand(store, updateFieldPropertiesCommand(field.id, { id: draft }))
            }
          />
          <TextProperty
            property="fieldLabel"
            label="Label"
            value={field.name ?? ''}
            onCommit={(draft) =>
              executeFormCommand(
                store,
                updateFieldPropertiesCommand(field.id, { name: draft.trim() || null }),
              )
            }
          />
          {capability?.supportsPlaceholder ? (
            <TextProperty
              property="fieldPlaceholder"
              label="Placeholder"
              value={field.placeholder ?? ''}
              onCommit={(draft) =>
                executeFormCommand(
                  store,
                  updateFieldPropertiesCommand(field.id, { placeholder: draft.trim() || null }),
                )
              }
            />
          ) : null}
          {isDate ? (
            <TextProperty
              property="fieldDatePattern"
              label="Date pattern"
              value={field.datePattern ?? ''}
              onCommit={(draft) =>
                executeFormCommand(
                  store,
                  updateFieldPropertiesCommand(field.id, { datePattern: draft.trim() || null }),
                )
              }
            />
          ) : null}
          {isExpression && field.fieldType === 'ExpressionFormField' ? (
            <TextProperty
              property="fieldExpression"
              label="Expression"
              value={field.expression}
              onCommit={(draft) =>
                executeFormCommand(store, updateFieldPropertiesCommand(field.id, { expression: draft }))
              }
            />
          ) : null}
          {isHyperlink ? (
            <TextProperty
              property="fieldHyperlinkUrl"
              label="URL"
              value={hyperlinkUrl}
              onCommit={(draft) =>
                executeFormCommand(
                  store,
                  updateFieldPropertiesCommand(field.id, { hyperlinkUrl: draft }),
                )
              }
            />
          ) : null}
          {capability?.supportsRequired ? (
            <CheckboxProperty
              property="fieldRequired"
              label="Required"
              value={field.required === true}
              onCommit={(checked) =>
                executeFormCommand(
                  store,
                  updateFieldPropertiesCommand(field.id, { required: checked ? true : null }),
                )
              }
            />
          ) : null}
          {capability?.writable ? (
            <CheckboxProperty
              property="fieldReadOnly"
              label="Read-only"
              value={field.readOnly === true}
              onCommit={(checked) =>
                executeFormCommand(
                  store,
                  updateFieldPropertiesCommand(field.id, { readOnly: checked ? true : null }),
                )
              }
            />
          ) : null}
        </section>
        {isOption ? <OptionsSection field={field} store={store} /> : null}
        <section>
          <h2>{isContainer ? 'Container' : 'Field'}</h2>
          <button
            type="button"
            className="quiet-action is-danger"
            onClick={() => {
              executeFormCommand(store, removeFieldCommand(field.id));
              store.getState().selectField(null);
            }}
          >
            Delete field
          </button>
        </section>
      </div>
    </aside>
  );
}

function OptionsSection({ field, store }: { field: FormFieldModel; store: FormEditorStore }) {
  const document = useStore(store, (state) => state.document);
  const current = findField(document.model, field.id);
  const options = current?.fieldType === 'OptionFormField' ? (current.options ?? []) : [];

  return (
    <section>
      <h2>Options</h2>
      {options.map((option, index) => (
        <div key={option.id || `option-${index}`} className="option-row">
          <TextProperty
            property={`optionId${index + 1}`}
            label={`Option ${index + 1} id`}
            value={option.id}
            validate={validateNonBlank('Option id')}
            onCommit={(draft) =>
              executeFormCommand(store, updateOptionCommand(field.id, index, { id: draft }))
            }
          />
          <TextProperty
            property={`optionName${index + 1}`}
            label={`Option ${index + 1} label`}
            value={option.name}
            onCommit={(draft) =>
              executeFormCommand(store, updateOptionCommand(field.id, index, { name: draft }))
            }
          />
          <span className="option-row-actions">
            <button
              type="button"
              aria-label={`Move option ${index + 1} up`}
              disabled={index === 0}
              onClick={() => executeFormCommand(store, moveOptionCommand(field.id, index, -1))}
            >
              ↑
            </button>
            <button
              type="button"
              aria-label={`Move option ${index + 1} down`}
              disabled={index === options.length - 1}
              onClick={() => executeFormCommand(store, moveOptionCommand(field.id, index, 1))}
            >
              ↓
            </button>
            <button
              type="button"
              aria-label={`Delete option ${index + 1}`}
              onClick={() => executeFormCommand(store, removeOptionCommand(field.id, index))}
            >
              ×
            </button>
          </span>
        </div>
      ))}
      <button
        type="button"
        className="quiet-action"
        onClick={() => executeFormCommand(store, addOptionCommand(field.id))}
      >
        + Add option
      </button>
    </section>
  );
}

function PanelHeading({ kicker, title, glyph }: { kicker: string; title: string; glyph: string }) {
  return (
    <div className="properties-heading">
      <div>
        <span className="panel-kicker">{kicker}</span>
        <h1>{title}</h1>
      </div>
      <span className="selection-glyph" aria-hidden="true">
        {glyph}
      </span>
    </div>
  );
}

interface TextPropertyProps {
  label: string;
  onCommit: (draft: string) => string | null;
  property: string;
  validate?: (draft: string) => string | null;
  value: string;
}

function TextProperty({ label, onCommit, property, validate, value }: TextPropertyProps) {
  const [draft, setDraft] = useState(value);
  const [error, setError] = useState<string | null>(null);
  const [committedValue, setCommittedValue] = useState(value);

  // Reset the draft when a new value is committed from outside (undo, reload).
  if (value !== committedValue) {
    setCommittedValue(value);
    setDraft(value);
    setError(null);
  }

  const commit = () => {
    if (draft === value) {
      setError(null);
      return;
    }
    const validationError = validate?.(draft) ?? null;
    if (validationError) {
      setError(validationError);
      return;
    }
    setError(onCommit(draft));
  };

  return (
    <div className="property-field">
      <label className="property-label" htmlFor={`property-${property}`}>
        {label}
      </label>
      <input
        id={`property-${property}`}
        type="text"
        aria-label={label}
        aria-invalid={error ? true : undefined}
        data-property={property}
        className={error ? 'property-input has-error' : 'property-input'}
        onBlur={commit}
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter') event.currentTarget.blur();
        }}
      />
      {error ? (
        <span className="property-error" role="alert">
          {error}
        </span>
      ) : null}
    </div>
  );
}

function CheckboxProperty({
  label,
  onCommit,
  property,
  value,
}: {
  label: string;
  onCommit: (checked: boolean) => string | null;
  property: string;
  value: boolean;
}) {
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="property-checkbox">
      <label htmlFor={`property-${property}`}>
        <input
          id={`property-${property}`}
          type="checkbox"
          aria-label={label}
          data-property={property}
          checked={value}
          onChange={(event) => setError(onCommit(event.target.checked))}
        />
        {label}
      </label>
      {error ? (
        <span className="property-error" role="alert">
          {error}
        </span>
      ) : null}
    </div>
  );
}

function validateNonBlank(label: string) {
  return (draft: string) => (draft.trim() === '' ? `${label} must not be blank` : null);
}
