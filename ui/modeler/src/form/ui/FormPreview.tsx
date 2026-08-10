import type { FormFieldModel, FormModel } from '../../generated/editor-protocol';
import { formFieldCapability } from '../index';

export interface FormPreviewProps {
  model: FormModel;
}

/**
 * Read-only fill-in rendering of the form document. It mirrors the runtime
 * field semantics: choice fields render their static options, booleans render
 * as checkboxes, and expression/display/container fields never produce input
 * controls that could be submitted.
 */
export function FormPreview({ model }: FormPreviewProps) {
  const fields = model.fields ?? [];
  return (
    <div className="form-preview-scroll">
      <form className="form-preview" aria-label="Form preview" onSubmit={(event) => event.preventDefault()}>
        <h2 className="form-preview-title">{model.name || 'Untitled form'}</h2>
        {model.description ? <p className="form-preview-description">{model.description}</p> : null}
        {fields.length === 0 ? (
          <p className="form-preview-empty">This form has no fields yet.</p>
        ) : (
          fields.map((field, index) => <PreviewField key={field.id || index} field={field} />)
        )}
        <div className="form-preview-outcomes">
          {(model.outcomes ?? []).length === 0 ? (
            <button type="button" className="primary-button" disabled>
              Complete
            </button>
          ) : (
            (model.outcomes ?? []).map((outcome, index) => (
              <button key={outcome.id ?? index} type="button" className="quiet-action" disabled>
                {outcome.name || `Outcome ${index + 1}`}
              </button>
            ))
          )}
        </div>
      </form>
    </div>
  );
}

function PreviewField({ field }: { field: FormFieldModel }) {
  const label = field.name?.trim() || field.id;
  const required = field.required === true;
  const type = field.type ?? '';

  if (field.fieldType === 'Container') {
    return (
      <fieldset className="form-preview-container">
        {label ? <legend>{label}</legend> : null}
        {(field.fields ?? []).map((row, rowIndex) => (
          <div key={rowIndex} className="form-preview-row">
            {row.map((child, childIndex) => (
              <PreviewField key={child.id || childIndex} field={child} />
            ))}
          </div>
        ))}
      </fieldset>
    );
  }

  switch (type) {
    case 'headline':
      return <h3 className="form-preview-headline">{label}</h3>;
    case 'headline-with-line':
      return (
        <h3 className="form-preview-headline form-preview-headline-lined">{label}</h3>
      );
    case 'horizontal-line':
      return <hr className="form-preview-line" />;
    case 'spacer':
      return <div className="form-preview-spacer" aria-hidden="true" />;
    case 'hyperlink':
      return (
        <div className="form-preview-field">
          <a href={field.params?.url ?? '#'} className="form-preview-link">
            {label}
          </a>
        </div>
      );
    case 'expression': {
      const expression = field.fieldType === 'ExpressionFormField' ? field.expression : '';
      return (
        <div className="form-preview-field">
          <output className="form-preview-expression" title={expression}>
            {expression || '…'}
          </output>
        </div>
      );
    }
    default:
      break;
  }

  return (
    <div className="form-preview-field">
      <label className="form-preview-label">
        {label}
        {required ? <em aria-label="required">*</em> : null}
      </label>
      <PreviewControl field={field} type={type} />
    </div>
  );
}

function PreviewControl({ field, type }: { field: FormFieldModel; type: string }) {
  const placeholder = field.placeholder ?? undefined;
  switch (type) {
    case 'multi-line-text':
      return <textarea rows={3} placeholder={placeholder} readOnly />;
    case 'integer':
    case 'decimal':
    case 'amount':
      return <input type="number" placeholder={placeholder} readOnly />;
    case 'date':
      return <input type="date" readOnly />;
    case 'boolean':
      return <input type="checkbox" disabled aria-label={field.name ?? field.id} />;
    case 'dropdown':
      return (
        <select disabled>
          <option value="">{field.placeholder ?? 'Select…'}</option>
          {field.fieldType === 'OptionFormField'
            ? (field.options ?? []).map((option) => (
                <option key={option.id} value={option.id}>
                  {option.name}
                </option>
              ))
            : null}
        </select>
      );
    case 'radio-buttons':
      return (
        <div className="form-preview-radio-group" role="radiogroup" aria-label={field.name ?? field.id}>
          {field.fieldType === 'OptionFormField'
            ? (field.options ?? []).map((option) => (
                <label key={option.id}>
                  <input type="radio" name={`preview-${field.id}`} disabled /> {option.name}
                </label>
              ))
            : null}
        </div>
      );
    case 'upload':
      return <input type="file" disabled />;
    case 'people':
      return <input type="text" placeholder={placeholder ?? 'Select a person…'} readOnly />;
    case 'functional-group':
      return <input type="text" placeholder={placeholder ?? 'Select a group…'} readOnly />;
    default: {
      // Unknown/imported types still render as their text-handler alias shape.
      const capability = formFieldCapability(type);
      if (capability?.requiredVariant === 'OptionFormField') {
        return (
          <select disabled>
            {field.fieldType === 'OptionFormField'
              ? (field.options ?? []).map((option) => (
                  <option key={option.id} value={option.id}>
                    {option.name}
                  </option>
                ))
              : null}
          </select>
        );
      }
      return <input type="text" placeholder={placeholder} readOnly />;
    }
  }
}
