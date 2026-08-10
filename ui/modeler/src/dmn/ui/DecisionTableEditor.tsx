import { useState } from 'react';
import { useStore } from 'zustand';

import type {
  DecisionRule,
  LiteralExpression,
  UnaryTests,
} from '../../generated/editor-protocol';
import {
  addInputColumnCommand,
  addOutputColumnCommand,
  addRuleCommand,
  decisionById,
  deleteRuleCommand,
  moveRuleCommand,
  type DmnEditorStore,
} from '../index';
import { commitCellText } from './editing';

/** Transient grid selection: a column header opened in the properties panel. */
export type DmnTableSelection =
  | { kind: 'input'; index: number }
  | { kind: 'output'; index: number }
  | null;

export interface DecisionTableEditorProps {
  decisionId: string;
  onSelect: (selection: DmnTableSelection) => void;
  selection: DmnTableSelection;
  store: DmnEditorStore;
}

export function DecisionTableEditor({
  decisionId,
  onSelect,
  selection,
  store,
}: DecisionTableEditorProps) {
  const document = useStore(store, (state) => state.document);
  const decision = decisionById(document, decisionId);
  if (!decision) {
    return (
      <div className="dmn-table-missing" role="alert">
        Decision {decisionId} is not part of this document.
      </div>
    );
  }
  const table = decision.decisionTable;
  const inputs = table.inputs ?? [];
  const outputs = table.outputs ?? [];
  const rules = table.rules ?? [];

  return (
    <div className="dmn-table-scroll">
      <table
        className="decision-table"
        aria-label={`Decision table ${decision.name ?? decision.id}`}
      >
        <thead>
          <tr className="dmn-group-row">
            <th className="dmn-rule-corner" scope="col">
              <span className="hit-policy-badge" title="Hit policy">
                {table.hitPolicy}
                {table.hitPolicy === 'COLLECT' && table.collectOperator
                  ? ` ${table.collectOperator}`
                  : ''}
              </span>
            </th>
            <th className="dmn-group dmn-group-input" colSpan={inputs.length} scope="colgroup">
              Inputs
              <button
                type="button"
                className="dmn-add-column"
                aria-label="Add input column"
                title="Add input column"
                onClick={() => store.getState().execute(addInputColumnCommand(decisionId))}
              >
                +
              </button>
            </th>
            <th className="dmn-group dmn-group-output" colSpan={outputs.length} scope="colgroup">
              Outputs
              <button
                type="button"
                className="dmn-add-column"
                aria-label="Add output column"
                title="Add output column"
                onClick={() => store.getState().execute(addOutputColumnCommand(decisionId))}
              >
                +
              </button>
            </th>
          </tr>
          <tr className="dmn-column-row">
            <th className="dmn-rule-corner" scope="col">
              <span className="visually-hidden">Rule</span>#
            </th>
            {inputs.map((input, index) => (
              <th
                key={input.id ?? `input-${index}`}
                className={columnClass('input', selection, index)}
                scope="col"
              >
                <button
                  type="button"
                  className="dmn-column-header"
                  aria-label={`Input column ${index + 1}`}
                  aria-pressed={isColumnSelected(selection, 'input', index)}
                  onClick={() => onSelect({ kind: 'input', index })}
                >
                  <span className="dmn-column-label">{input.label || `Input ${index + 1}`}</span>
                  <span className="dmn-column-meta">
                    {input.inputExpression.text || '−'}
                    {input.inputExpression.typeRef ? ` : ${input.inputExpression.typeRef}` : ''}
                  </span>
                </button>
              </th>
            ))}
            {outputs.map((output, index) => (
              <th
                key={output.id ?? `output-${index}`}
                className={columnClass('output', selection, index)}
                scope="col"
              >
                <button
                  type="button"
                  className="dmn-column-header"
                  aria-label={`Output column ${index + 1}`}
                  aria-pressed={isColumnSelected(selection, 'output', index)}
                  onClick={() => onSelect({ kind: 'output', index })}
                >
                  <span className="dmn-column-label">{output.label || output.name || `Output ${index + 1}`}</span>
                  <span className="dmn-column-meta">
                    {output.name || '−'}
                    {output.typeRef ? ` : ${output.typeRef}` : ''}
                  </span>
                </button>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rules.map((rule, rowIndex) => (
            <RuleRow
              key={rule.id ?? `rule-${rowIndex}`}
              decisionId={decisionId}
              inputCount={inputs.length}
              outputCount={outputs.length}
              ruleIndex={rowIndex}
              rule={rule}
              ruleCount={rules.length}
              store={store}
            />
          ))}
        </tbody>
      </table>
      <div className="dmn-table-footer">
        <button
          type="button"
          className="quiet-action"
          onClick={() => store.getState().execute(addRuleCommand(decisionId))}
        >
          + Add rule
        </button>
      </div>
    </div>
  );
}

interface RuleRowProps {
  decisionId: string;
  inputCount: number;
  outputCount: number;
  rule: DecisionRule;
  ruleCount: number;
  ruleIndex: number;
  store: DmnEditorStore;
}

function RuleRow({
  decisionId,
  inputCount,
  outputCount,
  rule,
  ruleCount,
  ruleIndex,
  store,
}: RuleRowProps) {
  const execute = store.getState().execute;
  const inputEntries = rule.inputEntries ?? [];
  const outputEntries = rule.outputEntries ?? [];
  return (
    <tr>
      <th className="dmn-rule-number" scope="row">
        <span>{ruleIndex + 1}</span>
        <span className="dmn-rule-actions">
          <button
            type="button"
            aria-label={`Move rule ${ruleIndex + 1} up`}
            title="Move rule up"
            disabled={ruleIndex === 0}
            onClick={() => execute(moveRuleCommand(decisionId, ruleIndex, ruleIndex - 1))}
          >
            ↑
          </button>
          <button
            type="button"
            aria-label={`Move rule ${ruleIndex + 1} down`}
            title="Move rule down"
            disabled={ruleIndex === ruleCount - 1}
            onClick={() => execute(moveRuleCommand(decisionId, ruleIndex, ruleIndex + 1))}
          >
            ↓
          </button>
          <button
            type="button"
            aria-label={`Delete rule ${ruleIndex + 1}`}
            title="Delete rule"
            disabled={ruleCount === 1}
            onClick={() => execute(deleteRuleCommand(decisionId, ruleIndex))}
          >
            ×
          </button>
        </span>
      </th>
      {Array.from({ length: inputCount }, (_, columnIndex) => (
        <CellEditor
          key={inputEntries[columnIndex]?.id ?? `input-${columnIndex}`}
          address={{ kind: 'input', row: ruleIndex, column: columnIndex }}
          decisionId={decisionId}
          entry={inputEntries[columnIndex] ?? null}
          store={store}
        />
      ))}
      {Array.from({ length: outputCount }, (_, columnIndex) => (
        <CellEditor
          key={outputEntries[columnIndex]?.id ?? `output-${columnIndex}`}
          address={{ kind: 'output', row: ruleIndex, column: columnIndex }}
          decisionId={decisionId}
          entry={outputEntries[columnIndex] ?? null}
          store={store}
        />
      ))}
    </tr>
  );
}

interface CellEditorProps {
  address: { kind: 'input'; row: number; column: number } | { kind: 'output'; row: number; column: number };
  decisionId: string;
  entry: UnaryTests | LiteralExpression | null;
  store: DmnEditorStore;
}

function CellEditor({ address, decisionId, entry, store }: CellEditorProps) {
  const value = entry?.text ?? '';
  const [draft, setDraft] = useState(value);
  const [error, setError] = useState<string | null>(null);
  const [committedValue, setCommittedValue] = useState(value);

  // Reset the draft when a new value is committed from outside (undo, reload).
  if (value !== committedValue) {
    setCommittedValue(value);
    setDraft(value);
    setError(null);
  }

  const label = `${address.kind} cell ${address.row + 1}:${address.column + 1}`;

  const commit = () => {
    if (draft === value && !error) return;
    const commitError = commitCellText(store, decisionId, address, draft);
    setError(commitError);
  };

  return (
    <td className={error ? 'dmn-cell has-error' : 'dmn-cell'}>
      <input
        type="text"
        className="dmn-cell-input"
        aria-label={label}
        aria-invalid={error ? true : undefined}
        data-cell-kind={address.kind}
        value={draft}
        placeholder="−"
        onBlur={commit}
        onChange={(event) => {
          setDraft(event.target.value);
          setError(null);
        }}
        onKeyDown={(event) => {
          if (event.key === 'Enter') event.currentTarget.blur();
          if (event.key === 'Escape') {
            setDraft(value);
            setError(null);
          }
        }}
      />
      {error ? (
        <span className="dmn-cell-error" role="alert">
          {error}
        </span>
      ) : null}
    </td>
  );
}

function isColumnSelected(
  selection: DmnTableSelection,
  kind: 'input' | 'output',
  index: number,
): boolean {
  return selection?.kind === kind && selection.index === index;
}

function columnClass(kind: 'input' | 'output', selection: DmnTableSelection, index: number) {
  const base = kind === 'input' ? 'dmn-column dmn-column-input' : 'dmn-column dmn-column-output';
  return isColumnSelected(selection, kind, index) ? `${base} is-selected` : base;
}
