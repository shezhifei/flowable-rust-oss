import { useStore } from 'zustand';

import type { FormFieldModel } from '../../generated/editor-protocol';
import {
  fieldCapability,
  moveFieldCommand,
  removeFieldCommand,
  type FormEditorStore,
  type FormValidationIssue,
} from '../index';
import { executeFormCommand } from './editing';

export interface FormCanvasProps {
  issues: FormValidationIssue[];
  store: FormEditorStore;
}

/**
 * Design-time canvas: the ordered field list with selection, reordering, and
 * recursive container rows. Editing happens through the properties panel.
 */
export function FormCanvas({ issues, store }: FormCanvasProps) {
  const document = useStore(store, (state) => state.document);
  const fields = document.model.fields ?? [];
  const issuesByField = new Map<string, FormValidationIssue[]>();
  for (const issue of issues) {
    if (!issue.elementId) continue;
    const list = issuesByField.get(issue.elementId) ?? [];
    list.push(issue);
    issuesByField.set(issue.elementId, list);
  }

  return (
    <div className="form-canvas-scroll">
      <ol className="form-canvas" aria-label="Form field list">
        {fields.length === 0 ? (
          <li className="form-canvas-empty">
            This form has no fields yet. Pick a field type from the palette.
          </li>
        ) : (
          fields.map((field, index) => (
            <FieldRow
              key={field.id || `field-${index}`}
              depth={0}
              field={field}
              index={index}
              issues={issuesByField}
              siblingCount={fields.length}
              store={store}
            />
          ))
        )}
      </ol>
    </div>
  );
}

interface FieldRowProps {
  depth: number;
  field: FormFieldModel;
  index: number;
  issues: Map<string, FormValidationIssue[]>;
  siblingCount: number;
  store: FormEditorStore;
}

function FieldRow({ depth, field, index, issues, siblingCount, store }: FieldRowProps) {
  const selectedFieldId = useStore(store, (state) => state.selectedFieldId);
  const capability = fieldCapability(field);
  const selected = selectedFieldId === field.id;
  const fieldIssues = issues.get(field.id) ?? [];

  const label = field.name?.trim() ? field.name : capability?.label || field.type || 'Untyped';
  const containerRows = field.fieldType === 'Container' ? (field.fields ?? []) : [];

  return (
    <li className="form-field-item">
      <div
        className={fieldClasses(selected, fieldIssues.length > 0, depth)}
        data-field-id={field.id}
      >
        <button
          type="button"
          className="form-field-select"
          aria-pressed={selected}
          aria-label={`Select field ${field.id}`}
          onClick={() => store.getState().selectField(selected ? null : field.id)}
        >
          <span className="form-field-glyph" aria-hidden="true">
            {capability?.glyph ?? '?'}
          </span>
          <span className="form-field-label">
            {label}
            {field.required === true ? <em aria-label="required">*</em> : null}
          </span>
          <span className="form-field-meta">
            {field.type ?? 'untyped'} · {field.id || '(no id)'}
          </span>
        </button>
        <span className="form-field-actions">
          <button
            type="button"
            aria-label={`Move field ${field.id} up`}
            title="Move up"
            disabled={index === 0}
            onClick={() => executeFormCommand(store, moveFieldCommand(field.id, -1))}
          >
            ↑
          </button>
          <button
            type="button"
            aria-label={`Move field ${field.id} down`}
            title="Move down"
            disabled={index === siblingCount - 1}
            onClick={() => executeFormCommand(store, moveFieldCommand(field.id, 1))}
          >
            ↓
          </button>
          <button
            type="button"
            aria-label={`Delete field ${field.id}`}
            title="Delete field"
            onClick={() => {
              executeFormCommand(store, removeFieldCommand(field.id));
              if (store.getState().selectedFieldId === field.id) {
                store.getState().selectField(null);
              }
            }}
          >
            ×
          </button>
        </span>
      </div>
      {fieldIssues.length ? (
        <ul className="form-field-issues">
          {fieldIssues.map((issue, issueIndex) => (
            <li key={issueIndex} role="alert">
              {issue.message}
            </li>
          ))}
        </ul>
      ) : null}
      {field.fieldType === 'Container' ? (
        <ol className="form-container-rows" aria-label={`Container ${field.id} rows`}>
          {containerRows.length === 0 ? (
            <li className="form-container-empty">Empty container — add fields from the palette.</li>
          ) : (
            containerRows.map((row, rowIndex) => (
              <li key={rowIndex} className="form-container-row">
                <ol className="form-container-row-fields">
                  {row.map((child, childIndex) => (
                    <FieldRow
                      key={child.id || `row-${rowIndex}-field-${childIndex}`}
                      depth={depth + 1}
                      field={child}
                      index={rowIndex}
                      issues={issues}
                      siblingCount={containerRows.length}
                      store={store}
                    />
                  ))}
                </ol>
              </li>
            ))
          )}
        </ol>
      ) : null}
    </li>
  );
}

function fieldClasses(selected: boolean, hasIssues: boolean, depth: number): string {
  const classes = ['form-field'];
  if (selected) classes.push('is-selected');
  if (hasIssues) classes.push('has-issues');
  if (depth > 0) classes.push('is-nested');
  return classes.join(' ');
}
