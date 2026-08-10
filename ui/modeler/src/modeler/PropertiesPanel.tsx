import { useState } from 'react';

import type { BpmnEditorDocument, FlowElementEnum } from '../generated/editor-protocol';
import {
  CallActivitySection,
  EventReferenceSection,
  FieldInjectionSection,
  GlobalDefinitionsSection,
  ListenersSection,
  MultiInstanceSection,
} from './AdvancedPropertySections';
import { documentElements } from './diagramModel';
import { useModelerStore } from './modelerStore';
import {
  renameElementIdCommand,
  updateElementPropertiesCommand,
  updateProcessPropertiesCommand,
} from './propertyCommands';
import {
  validateConditionExpression,
  validateElementId,
  validateNumericValue,
} from './propertyValidation';

const EXECUTION_FLAG_TYPES = new Set<FlowElementEnum['elementType']>([
  'task',
  'userTask',
  'serviceTask',
  'caseServiceTask',
  'sendTask',
  'scriptTask',
  'manualTask',
  'receiveTask',
  'businessRuleTask',
  'callActivity',
  'subProcess',
  'transaction',
  'eventSubProcess',
  'adhocSubProcess',
  'boundaryEvent',
]);

const IMPLEMENTATION_TYPES = [
  ['class', 'Java class'],
  ['expression', 'Expression'],
  ['delegateExpression', 'Delegate expression'],
] as const;

export interface PanelRenderState {
  document: BpmnEditorDocument;
  selectedElementIds: string[];
}

export function PropertiesPanel({ panelState }: { panelState?: PanelRenderState } = {}) {
  const storeDocument = useModelerStore((state) => state.document);
  const storeSelectedElementIds = useModelerStore((state) => state.selectedElementIds);
  const document = panelState?.document ?? storeDocument;
  const selectedElementIds = panelState?.selectedElementIds ?? storeSelectedElementIds;

  if (selectedElementIds.length > 1) {
    return (
      <aside className="properties-panel" aria-label="Element properties">
        <PanelHeading
          kicker="Selection"
          title={`${selectedElementIds.length} elements`}
          glyph="▦"
        />
        <div className="empty-properties" data-panel-state="multi-select">
          <span>Multiple elements selected</span>
          <p>
            Bulk property editing is read-only for now. Select a single element to edit its
            properties.
          </p>
        </div>
      </aside>
    );
  }

  const selectedId = selectedElementIds[0] ?? null;
  const element = selectedId
    ? documentElements(document).find((candidate) => candidate.id === selectedId)
    : undefined;

  if (selectedId && !element) {
    return (
      <aside className="properties-panel" aria-label="Element properties">
        <PanelHeading kicker="Selection" title={selectedId} glyph="◎" />
        <div className="empty-properties" data-panel-state="unsupported">
          <span>Not editable yet</span>
          <p>Property editing for this diagram element arrives in a later milestone.</p>
        </div>
      </aside>
    );
  }

  if (!element) {
    return <ProcessProperties document={document} />;
  }
  return <ElementProperties key={element.id ?? selectedId} document={document} element={element} />;
}

function ProcessProperties({ document }: { document: BpmnEditorDocument }) {
  const execute = useModelerStore((state) => state.execute);
  const process = document.model.processes[0];

  if (!process) {
    return (
      <aside className="properties-panel" aria-label="Process properties">
        <PanelHeading kicker="Process" title="No process" glyph="◎" />
        <div className="empty-properties">
          <span>Nothing to edit</span>
          <p>This document does not contain a process.</p>
        </div>
      </aside>
    );
  }

  return (
    <aside className="properties-panel" aria-label="Process properties">
      <PanelHeading kicker="Process" title={process.name ?? 'Untitled process'} glyph="◎" />
      <div className="property-groups" data-panel-state="process">
        <section>
          <h2>General</h2>
          <TextProperty
            property="id"
            label="ID"
            value={process.id ?? ''}
            validate={(draft) => validateElementId(document, process.id ?? null, draft)}
            onCommit={(draft) => execute(updateProcessPropertiesCommand({ id: draft.trim() }))}
          />
          <TextProperty
            property="name"
            label="Name"
            value={process.name ?? ''}
            onCommit={(draft) =>
              execute(updateProcessPropertiesCommand({ name: draft.trim() || null }))
            }
          />
          <TextProperty
            property="documentation"
            label="Documentation"
            value={process.documentation ?? ''}
            multiline
            onCommit={(draft) =>
              execute(updateProcessPropertiesCommand({ documentation: draft.trim() || null }))
            }
          />
        </section>
        <GlobalDefinitionsSection document={document} />
      </div>
    </aside>
  );
}

function ElementProperties({
  document,
  element,
}: {
  document: BpmnEditorDocument;
  element: FlowElementEnum;
}) {
  const execute = useModelerStore((state) => state.execute);
  const elementId = element.id ?? '';
  const commit = (properties: Record<string, unknown>, field: string) =>
    execute(updateElementPropertiesCommand(elementId, properties, `Edit ${field}`));
  const commitText = (field: string) => (draft: string) =>
    commit({ [field]: draft.trim() || null }, field);
  const commitList = (field: string) => (draft: string) =>
    commit(
      {
        [field]: draft
          .split(',')
          .map((entry) => entry.trim())
          .filter(Boolean),
      },
      field,
    );

  return (
    <aside className="properties-panel" aria-label="Element properties">
      <PanelHeading
        kicker={humanize(element.elementType)}
        title={element.name ?? 'Unnamed'}
        glyph={elementGlyph(element)}
      />
      <div className="property-groups" data-panel-state="element">
        <section>
          <h2>General</h2>
          <TextProperty
            property="id"
            label="ID"
            value={element.id ?? ''}
            validate={(draft) => validateElementId(document, element.id ?? null, draft)}
            onCommit={(draft) => execute(renameElementIdCommand(elementId, draft.trim()))}
          />
          <TextProperty
            property="name"
            label="Name"
            value={element.name ?? ''}
            onCommit={commitText('name')}
          />
          <TextProperty
            property="documentation"
            label="Documentation"
            value={element.documentation ?? ''}
            multiline
            onCommit={commitText('documentation')}
          />
        </section>

        {EXECUTION_FLAG_TYPES.has(element.elementType) &&
        'asynchronous' in element &&
        'exclusive' in element ? (
          <section>
            <h2>Execution</h2>
            <CheckboxProperty
              property="asynchronous"
              label="Asynchronous"
              checked={Boolean(element.asynchronous)}
              onCommit={(checked) => commit({ asynchronous: checked }, 'asynchronous')}
            />
            <CheckboxProperty
              property="exclusive"
              label="Exclusive"
              checked={Boolean(element.exclusive)}
              onCommit={(checked) => commit({ exclusive: checked }, 'exclusive')}
            />
          </section>
        ) : null}

        {element.elementType === 'userTask' ? (
          <section>
            <h2>Assignment</h2>
            <TextProperty
              property="assignee"
              label="Assignee"
              value={element.assignee ?? ''}
              onCommit={commitText('assignee')}
            />
            <TextProperty
              property="candidateUsers"
              label="Candidate users"
              value={(element.candidateUsers ?? []).join(', ')}
              onCommit={commitList('candidateUsers')}
            />
            <TextProperty
              property="candidateGroups"
              label="Candidate groups"
              value={(element.candidateGroups ?? []).join(', ')}
              onCommit={commitList('candidateGroups')}
            />
          </section>
        ) : null}

        {element.elementType === 'userTask' ? (
          <section>
            <h2>Form &amp; scheduling</h2>
            <TextProperty
              property="formKey"
              label="Form key"
              value={element.formKey ?? ''}
              onCommit={commitText('formKey')}
            />
            <TextProperty
              property="dueDate"
              label="Due date"
              value={element.dueDate ?? ''}
              onCommit={commitText('dueDate')}
            />
            <TextProperty
              property="priority"
              label="Priority"
              value={element.priority ?? ''}
              validate={validateNumericValue}
              onCommit={commitText('priority')}
            />
            <TextProperty
              property="category"
              label="Category"
              value={element.category ?? ''}
              onCommit={commitText('category')}
            />
          </section>
        ) : null}

        {element.elementType === 'serviceTask' ? (
          <section>
            <h2>Implementation</h2>
            <SelectProperty
              property="implementationType"
              label="Implementation type"
              value={element.implementationType ?? ''}
              options={IMPLEMENTATION_TYPES}
              onCommit={(value) =>
                commit({ implementationType: value || null }, 'implementationType')
              }
            />
            <TextProperty
              property="implementation"
              label={implementationLabel(element.implementationType)}
              value={element.implementation ?? ''}
              onCommit={commitText('implementation')}
            />
            <TextProperty
              property="resultVariableName"
              label="Result variable name"
              value={element.resultVariableName ?? ''}
              onCommit={commitText('resultVariableName')}
            />
          </section>
        ) : null}

        {element.elementType === 'sequenceFlow' ? (
          <section>
            <h2>Condition</h2>
            <TextProperty
              property="conditionExpression"
              label="Condition expression"
              value={element.conditionExpression ?? ''}
              multiline
              validate={validateConditionExpression}
              onCommit={commitText('conditionExpression')}
            />
          </section>
        ) : null}

        <MultiInstanceSection element={element} />
        <ListenersSection element={element} />
        <FieldInjectionSection element={element} />
        <CallActivitySection element={element} />
        <EventReferenceSection document={document} element={element} />
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
  multiline?: boolean;
  onCommit: (draft: string) => void;
  property: string;
  validate?: (draft: string) => string | null;
  value: string;
}

function TextProperty({
  label,
  multiline,
  onCommit,
  property,
  validate,
  value,
}: TextPropertyProps) {
  const [draft, setDraft] = useState(value);
  const [error, setError] = useState<string | null>(null);
  const [committedValue, setCommittedValue] = useState(value);

  // Reset the draft when a new value is committed from outside (undo, another field).
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
    setError(validationError);
    if (validationError) return;
    onCommit(draft);
  };

  const sharedProps = {
    'aria-label': label,
    'aria-invalid': error ? true : undefined,
    'data-property': property,
    className: error ? 'property-input has-error' : 'property-input',
    onBlur: commit,
    value: draft,
  };

  return (
    <div className="property-field">
      <label className="property-label" htmlFor={`property-${property}`}>
        {label}
      </label>
      {multiline ? (
        <textarea
          {...sharedProps}
          id={`property-${property}`}
          rows={3}
          onChange={(event) => setDraft(event.target.value)}
        />
      ) : (
        <input
          {...sharedProps}
          id={`property-${property}`}
          type="text"
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') event.currentTarget.blur();
          }}
        />
      )}
      {error ? (
        <span className="property-error" role="alert">
          {error}
        </span>
      ) : null}
    </div>
  );
}

function CheckboxProperty({
  checked,
  label,
  onCommit,
  property,
}: {
  checked: boolean;
  label: string;
  onCommit: (checked: boolean) => void;
  property: string;
}) {
  return (
    <div className="property-field property-checkbox">
      <label htmlFor={`property-${property}`}>
        <input
          id={`property-${property}`}
          aria-label={label}
          data-property={property}
          type="checkbox"
          checked={checked}
          onChange={(event) => onCommit(event.target.checked)}
        />
        {label}
      </label>
    </div>
  );
}

function SelectProperty({
  label,
  onCommit,
  options,
  property,
  value,
}: {
  label: string;
  onCommit: (value: string) => void;
  options: readonly (readonly [string, string])[];
  property: string;
  value: string;
}) {
  return (
    <div className="property-field">
      <label className="property-label" htmlFor={`property-${property}`}>
        {label}
      </label>
      <select
        id={`property-${property}`}
        aria-label={label}
        data-property={property}
        className="property-input"
        value={value}
        onChange={(event) => onCommit(event.target.value)}
      >
        <option value="">None</option>
        {options.map(([optionValue, optionLabel]) => (
          <option key={optionValue} value={optionValue}>
            {optionLabel}
          </option>
        ))}
      </select>
    </div>
  );
}

function implementationLabel(implementationType: string | null | undefined) {
  switch (implementationType) {
    case 'class':
      return 'Java class';
    case 'expression':
      return 'Expression';
    case 'delegateExpression':
      return 'Delegate expression';
    default:
      return 'Implementation';
  }
}

function elementGlyph(element: FlowElementEnum) {
  if (element.elementType.includes('Event')) return '○';
  if (element.elementType.includes('Gateway')) return '◇';
  if (element.elementType.includes('Task')) return '▢';
  return '▣';
}

function humanize(value: string) {
  return value.replace(/([a-z])([A-Z])/g, '$1 $2').replace(/^./, (letter) => letter.toUpperCase());
}
