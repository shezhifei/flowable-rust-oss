import { useStore } from 'zustand';

import {
  FORM_PALETTE_GROUPS,
  addFieldCommand,
  findField,
  nextFieldId,
  paletteCapability,
  type FormEditorStore,
} from '../index';
import { executeFormCommand } from './editing';

export interface FormPaletteProps {
  store: FormEditorStore;
}

/**
 * Field palette grouped by the contract field families. Clicking adds the
 * field at the top level, or inside the currently selected container.
 */
export function FormPalette({ store }: FormPaletteProps) {
  const document = useStore(store, (state) => state.document);
  const selectedFieldId = useStore(store, (state) => state.selectedFieldId);
  const selected = selectedFieldId ? findField(document.model, selectedFieldId) : null;
  const targetContainer = selected?.fieldType === 'Container' ? selected : null;

  return (
    <aside className="palette-panel" aria-label="Form field palette">
      <div className="panel-kicker">Fields</div>
      {FORM_PALETTE_GROUPS.map((group) => (
        <div key={group.id} className="palette-group">
          <div className="panel-kicker palette-group-label">{group.label}</div>
          <div className="palette-list">
            {group.wireTypes.map((wireType) => {
              const capability = paletteCapability(wireType);
              return (
                <button
                  key={wireType}
                  type="button"
                  title={`Add ${capability.label.toLowerCase()} field`}
                  data-field-type={wireType}
                  onClick={() => {
                    const id = nextFieldId(document.model, wireType);
                    const error = executeFormCommand(
                      store,
                      addFieldCommand(wireType, {
                        containerId: targetContainer ? targetContainer.id : null,
                        id,
                      }),
                    );
                    if (!error) store.getState().selectField(id);
                  }}
                >
                  <span aria-hidden="true">{capability.glyph}</span>
                  {capability.label}
                </button>
              );
            })}
          </div>
        </div>
      ))}
      <div className="palette-hint">
        {targetContainer
          ? `New fields land inside container '${targetContainer.id}'.`
          : 'Click to add a field. Select a container to add fields inside it.'}
      </div>
    </aside>
  );
}
