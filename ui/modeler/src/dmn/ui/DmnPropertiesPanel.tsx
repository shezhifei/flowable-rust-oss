import { useState } from 'react';
import { useStore } from 'zustand';

import type { CollectOperator, HitPolicy } from '../../generated/editor-protocol';
import {
  COLLECT_OPERATORS,
  CREATABLE_HIT_POLICIES,
  VALUE_TYPE_REFS,
  decisionById,
  deleteInputColumnCommand,
  deleteOutputColumnCommand,
  readDecisionTableProperties,
  setHitPolicyCommand,
  updateDecisionPropertiesCommand,
  updateDefinitionPropertiesCommand,
  updateInputColumnCommand,
  updateOutputColumnCommand,
  type CreatableHitPolicy,
  type DmnEditorStore,
  type DmnValueTypeRef,
} from '../index';
import type { DmnTableSelection } from './DecisionTableEditor';
import { executeDmnCommand } from './editing';

const HIT_POLICY_LABELS: Record<HitPolicy, string> = {
  FIRST: 'First',
  UNIQUE: 'Unique',
  ANY: 'Any',
  COLLECT: 'Collect',
  RULE_ORDER: 'Rule order',
  OUTPUT_ORDER: 'Output order',
  PRIORITY: 'Priority',
  COMPLETE: 'Complete (imported)',
};

export interface DmnPropertiesPanelProps {
  decisionId: string;
  onSelect: (selection: DmnTableSelection) => void;
  selection: DmnTableSelection;
  store: DmnEditorStore;
}

export function DmnPropertiesPanel({
  decisionId,
  onSelect,
  selection,
  store,
}: DmnPropertiesPanelProps) {
  const document = useStore(store, (state) => state.document);
  const decision = decisionById(document, decisionId);

  if (!decision) {
    return (
      <aside className="properties-panel" aria-label="Decision table properties">
        <PanelHeading kicker="Decision table" title="Missing decision" glyph="▤" />
        <div className="empty-properties">
          <span>Nothing to edit</span>
          <p>The selected decision is not part of this document.</p>
        </div>
      </aside>
    );
  }

  const inputs = decision.decisionTable.inputs ?? [];
  const outputs = decision.decisionTable.outputs ?? [];

  if (selection?.kind === 'input' && selection.index < inputs.length) {
    return (
      <InputColumnProperties
        key={`input-${selection.index}`}
        decisionId={decisionId}
        index={selection.index}
        onSelect={onSelect}
        store={store}
      />
    );
  }
  if (selection?.kind === 'output' && selection.index < outputs.length) {
    return (
      <OutputColumnProperties
        key={`output-${selection.index}`}
        decisionId={decisionId}
        index={selection.index}
        onSelect={onSelect}
        store={store}
      />
    );
  }
  return <GeneralProperties decisionId={decisionId} store={store} />;
}

function GeneralProperties({ decisionId, store }: { decisionId: string; store: DmnEditorStore }) {
  const document = useStore(store, (state) => state.document);
  const properties = readDecisionTableProperties(document, decisionId);
  const decision = decisionById(document, decisionId);
  if (!properties || !decision) return null;
  const table = decision.decisionTable;

  return (
    <aside className="properties-panel" aria-label="Decision table properties">
      <PanelHeading kicker="Decision table" title={properties.name ?? 'Untitled decision'} glyph="▤" />
      <div className="property-groups" data-panel-state="dmn-general">
        <section>
          <h2>General</h2>
          <TextProperty
            property="id"
            label="ID"
            value={properties.definitionId ?? ''}
            validate={validateNonBlank('Definition id')}
            onCommit={(draft) =>
              executeDmnCommand(store, updateDefinitionPropertiesCommand({ id: draft.trim() }))
            }
          />
          <TextProperty
            property="name"
            label="Name"
            value={properties.definitionName ?? ''}
            onCommit={(draft) =>
              executeDmnCommand(
                store,
                updateDefinitionPropertiesCommand({ name: draft.trim() || null }),
              )
            }
          />
          <TextProperty
            property="key"
            label="Key"
            value={properties.key}
            validate={validateNonBlank('Decision key')}
            onCommit={(draft) =>
              executeDmnCommand(
                store,
                updateDecisionPropertiesCommand(decisionId, { key: draft.trim() }),
              )
            }
          />
          <TextProperty
            property="decisionName"
            label="Decision name"
            value={properties.name ?? ''}
            onCommit={(draft) =>
              executeDmnCommand(
                store,
                updateDecisionPropertiesCommand(decisionId, { name: draft.trim() || null }),
              )
            }
          />
          <TextProperty
            property="tableId"
            label="Table ID"
            value={properties.tableId}
            validate={validateNonBlank('Decision table id')}
            onCommit={(draft) =>
              executeDmnCommand(
                store,
                updateDecisionPropertiesCommand(decisionId, { tableId: draft.trim() }),
              )
            }
          />
        </section>
        <section>
          <h2>Hit policy</h2>
          <HitPolicySelect decisionId={decisionId} store={store} />
          {table.hitPolicy === 'COLLECT' ? (
            <CollectOperatorSelect decisionId={decisionId} store={store} />
          ) : null}
        </section>
      </div>
    </aside>
  );
}

function HitPolicySelect({ decisionId, store }: { decisionId: string; store: DmnEditorStore }) {
  const document = useStore(store, (state) => state.document);
  const table = decisionById(document, decisionId)?.decisionTable;
  const [error, setError] = useState<string | null>(null);
  if (!table) return null;
  const current = table.hitPolicy;
  const offered = (CREATABLE_HIT_POLICIES as readonly HitPolicy[]).includes(current)
    ? CREATABLE_HIT_POLICIES
    : [current, ...CREATABLE_HIT_POLICIES];

  return (
    <div className="property-field">
      <label className="property-label" htmlFor="property-hitPolicy">
        Hit policy
      </label>
      <select
        id="property-hitPolicy"
        aria-label="Hit policy"
        data-property="hitPolicy"
        className={error ? 'property-input has-error' : 'property-input'}
        value={current}
        onChange={(event) => {
          const next = event.target.value as CreatableHitPolicy;
          setError(executeDmnCommand(store, setHitPolicyCommand(decisionId, next)));
        }}
      >
        {offered.map((policy) => (
          <option key={policy} value={policy}>
            {HIT_POLICY_LABELS[policy]}
          </option>
        ))}
      </select>
      {error ? (
        <span className="property-error" role="alert">
          {error}
        </span>
      ) : null}
    </div>
  );
}

function CollectOperatorSelect({
  decisionId,
  store,
}: {
  decisionId: string;
  store: DmnEditorStore;
}) {
  const document = useStore(store, (state) => state.document);
  const table = decisionById(document, decisionId)?.decisionTable;
  const [error, setError] = useState<string | null>(null);
  if (!table) return null;

  return (
    <div className="property-field">
      <label className="property-label" htmlFor="property-collectOperator">
        Collect aggregator
      </label>
      <select
        id="property-collectOperator"
        aria-label="Collect aggregator"
        data-property="collectOperator"
        className={error ? 'property-input has-error' : 'property-input'}
        value={table.collectOperator ?? ''}
        onChange={(event) => {
          const operator = (event.target.value || null) as CollectOperator | null;
          setError(executeDmnCommand(store, setHitPolicyCommand(decisionId, 'COLLECT', operator)));
        }}
      >
        <option value="">None</option>
        {COLLECT_OPERATORS.map((operator) => (
          <option key={operator} value={operator}>
            {operator}
          </option>
        ))}
      </select>
      {error ? (
        <span className="property-error" role="alert">
          {error}
        </span>
      ) : null}
    </div>
  );
}

function InputColumnProperties({
  decisionId,
  index,
  onSelect,
  store,
}: {
  decisionId: string;
  index: number;
  onSelect: (selection: DmnTableSelection) => void;
  store: DmnEditorStore;
}) {
  const document = useStore(store, (state) => state.document);
  const input = decisionById(document, decisionId)?.decisionTable.inputs?.[index];
  if (!input) return null;

  return (
    <aside className="properties-panel" aria-label="Decision table properties">
      <PanelHeading
        kicker={`Input column ${index + 1}`}
        title={input.label ?? 'Unnamed input'}
        glyph="⇥"
      />
      <div className="property-groups" data-panel-state="dmn-input-column">
        <section>
          <h2>Input</h2>
          <TextProperty
            property="columnLabel"
            label="Label"
            value={input.label ?? ''}
            onCommit={(draft) =>
              executeDmnCommand(
                store,
                updateInputColumnCommand(decisionId, index, { label: draft.trim() || null }),
              )
            }
          />
          <TextProperty
            property="columnExpression"
            label="Expression"
            value={input.inputExpression.text ?? ''}
            onCommit={(draft) =>
              executeDmnCommand(
                store,
                updateInputColumnCommand(decisionId, index, { expression: draft.trim() || null }),
              )
            }
          />
          <TypeRefSelect
            value={input.inputExpression.typeRef ?? ''}
            onCommit={(typeRef) =>
              executeDmnCommand(store, updateInputColumnCommand(decisionId, index, { typeRef }))
            }
          />
        </section>
        <section>
          <h2>Column</h2>
          <button
            type="button"
            className="quiet-action is-danger"
            onClick={() => {
              executeDmnCommand(store, deleteInputColumnCommand(decisionId, index));
              onSelect(null);
            }}
          >
            Delete input column
          </button>
        </section>
      </div>
    </aside>
  );
}

function OutputColumnProperties({
  decisionId,
  index,
  onSelect,
  store,
}: {
  decisionId: string;
  index: number;
  onSelect: (selection: DmnTableSelection) => void;
  store: DmnEditorStore;
}) {
  const document = useStore(store, (state) => state.document);
  const decision = decisionById(document, decisionId);
  const output = decision?.decisionTable.outputs?.[index];
  if (!decision || !output) return null;
  const outputCount = decision.decisionTable.outputs?.length ?? 0;

  return (
    <aside className="properties-panel" aria-label="Decision table properties">
      <PanelHeading
        kicker={`Output column ${index + 1}`}
        title={output.label ?? output.name ?? 'Unnamed output'}
        glyph="⇤"
      />
      <div className="property-groups" data-panel-state="dmn-output-column">
        <section>
          <h2>Output</h2>
          <TextProperty
            property="columnLabel"
            label="Label"
            value={output.label ?? ''}
            onCommit={(draft) =>
              executeDmnCommand(
                store,
                updateOutputColumnCommand(decisionId, index, { label: draft.trim() || null }),
              )
            }
          />
          <TextProperty
            property="columnName"
            label="Name"
            value={output.name ?? ''}
            onCommit={(draft) =>
              executeDmnCommand(
                store,
                updateOutputColumnCommand(decisionId, index, { name: draft.trim() || null }),
              )
            }
          />
          <TypeRefSelect
            value={output.typeRef ?? ''}
            onCommit={(typeRef) =>
              executeDmnCommand(store, updateOutputColumnCommand(decisionId, index, { typeRef }))
            }
          />
          <TextProperty
            property="outputValues"
            label="Output values"
            value={output.outputValues?.text ?? ''}
            onCommit={(draft) =>
              executeDmnCommand(
                store,
                updateOutputColumnCommand(decisionId, index, {
                  outputValues: draft.trim() || null,
                }),
              )
            }
          />
        </section>
        <section>
          <h2>Column</h2>
          <button
            type="button"
            className="quiet-action is-danger"
            disabled={outputCount === 1}
            title={outputCount === 1 ? 'A decision table must keep at least one output' : undefined}
            onClick={() => {
              executeDmnCommand(store, deleteOutputColumnCommand(decisionId, index));
              onSelect(null);
            }}
          >
            Delete output column
          </button>
        </section>
      </div>
    </aside>
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

function TypeRefSelect({
  onCommit,
  value,
}: {
  onCommit: (typeRef: DmnValueTypeRef | null) => string | null;
  value: string;
}) {
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="property-field">
      <label className="property-label" htmlFor="property-columnType">
        Type
      </label>
      <select
        id="property-columnType"
        aria-label="Type"
        data-property="columnType"
        className={error ? 'property-input has-error' : 'property-input'}
        value={value}
        onChange={(event) =>
          setError(onCommit((event.target.value || null) as DmnValueTypeRef | null))
        }
      >
        <option value="">None</option>
        {VALUE_TYPE_REFS.map((typeRef) => (
          <option key={typeRef} value={typeRef}>
            {typeRef}
          </option>
        ))}
      </select>
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
