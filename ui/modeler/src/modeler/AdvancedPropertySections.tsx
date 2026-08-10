import type {
  BpmnEditorDocument,
  EventDefinitionEnum,
  FieldExtension,
  FlowableListener,
  FlowElementEnum,
  IOParameter,
  Message,
  MultiInstanceLoopCharacteristics,
  Signal,
} from '../generated/editor-protocol';
import { useModelerStore } from './modelerStore';
import {
  createEmptyFieldExtension,
  createEmptyIOParameter,
  createEmptyListener,
  createEmptyLoopCharacteristics,
  createEmptyMessage,
  createEmptySignal,
  updateElementPropertiesCommand,
  updateEventDefinitionRefCommand,
  updateModelMessagesCommand,
  updateModelSignalsCommand,
} from './propertyCommands';

const MULTI_INSTANCE_TYPES = new Set<FlowElementEnum['elementType']>([
  'userTask',
  'serviceTask',
  'scriptTask',
  'manualTask',
  'receiveTask',
  'sendTask',
  'businessRuleTask',
  'callActivity',
  'subProcess',
  'transaction',
  'adhocSubProcess',
]);

const EXECUTION_LISTENER_TYPES = new Set<FlowElementEnum['elementType']>([
  'userTask',
  'serviceTask',
  'scriptTask',
  'manualTask',
  'receiveTask',
  'sendTask',
  'businessRuleTask',
  'callActivity',
  'subProcess',
  'transaction',
  'eventSubProcess',
  'adhocSubProcess',
  'startEvent',
  'endEvent',
  'boundaryEvent',
  'intermediateCatchEvent',
  'intermediateThrowEvent',
  'sequenceFlow',
]);

const IMPLEMENTATION_OPTIONS = [
  ['class', 'Java class'],
  ['expression', 'Expression'],
  ['delegateExpression', 'Delegate expression'],
] as const;

const TASK_LISTENER_EVENTS = [
  ['create', 'create'],
  ['assignment', 'assignment'],
  ['complete', 'complete'],
  ['delete', 'delete'],
] as const;

const EXECUTION_LISTENER_EVENTS = [
  ['start', 'start'],
  ['end', 'end'],
  ['take', 'take'],
] as const;

export function supportsMultiInstance(element: FlowElementEnum): boolean {
  return MULTI_INSTANCE_TYPES.has(element.elementType) && 'loopCharacteristics' in element;
}

export function supportsExecutionListeners(element: FlowElementEnum): boolean {
  return EXECUTION_LISTENER_TYPES.has(element.elementType) && 'executionListeners' in element;
}

export function supportsTaskListeners(element: FlowElementEnum): boolean {
  return element.elementType === 'userTask' && 'taskListeners' in element;
}

export function supportsFieldInjection(element: FlowElementEnum): boolean {
  return (
    (element.elementType === 'serviceTask' || element.elementType === 'callActivity') &&
    'fieldExtensions' in element
  );
}

export function supportsCallActivity(element: FlowElementEnum): boolean {
  return element.elementType === 'callActivity';
}

type ActivityLike = FlowElementEnum & {
  loopCharacteristics?: MultiInstanceLoopCharacteristics | null;
  fieldExtensions?: FieldExtension[];
  taskListeners?: FlowableListener[];
  executionListeners?: FlowableListener[];
};

function asActivity(element: FlowElementEnum): ActivityLike {
  return element as ActivityLike;
}

export function MultiInstanceSection({ element }: { element: FlowElementEnum }) {
  if (!supportsMultiInstance(element)) return null;
  const execute = useModelerStore((state) => state.execute);
  const elementId = element.id ?? '';
  const activity = asActivity(element);
  const loop = activity.loopCharacteristics ?? null;
  const enabled = Boolean(loop);

  const commitLoop = (next: MultiInstanceLoopCharacteristics | null, label: string) => {
    execute(updateElementPropertiesCommand(elementId, { loopCharacteristics: next }, label));
  };

  const patchLoop = (
    patch: Partial<MultiInstanceLoopCharacteristics>,
    label: string,
  ) => {
    const base = loop ?? createEmptyLoopCharacteristics();
    commitLoop({ ...base, ...patch } as MultiInstanceLoopCharacteristics, label);
  };

  return (
    <section data-property-group="multi-instance">
      <h2>Multi-instance</h2>
      <div className="property-field property-checkbox">
        <label htmlFor="property-mi-enabled">
          <input
            id="property-mi-enabled"
            aria-label="Enable multi-instance"
            data-property="multiInstanceEnabled"
            type="checkbox"
            checked={enabled}
            onChange={(event) =>
              commitLoop(
                event.target.checked ? createEmptyLoopCharacteristics() : null,
                event.target.checked ? 'Enable multi-instance' : 'Disable multi-instance',
              )
            }
          />
          Enable multi-instance
        </label>
      </div>
      {enabled && loop ? (
        <>
          <div className="property-field property-checkbox">
            <label htmlFor="property-mi-sequential">
              <input
                id="property-mi-sequential"
                aria-label="Sequential"
                data-property="multiInstanceSequential"
                type="checkbox"
                checked={Boolean(loop.sequential)}
                onChange={(event) =>
                  patchLoop({ sequential: event.target.checked }, 'Edit multi-instance sequential')
                }
              />
              Sequential
            </label>
          </div>
          <TextRow
            property="multiInstanceCollection"
            label="Collection"
            value={loop.collectionString ?? ''}
            onCommit={(draft) =>
              patchLoop({ collectionString: draft.trim() || null }, 'Edit multi-instance collection')
            }
          />
          <TextRow
            property="multiInstanceElementVariable"
            label="Element variable"
            value={loop.elementVariable ?? ''}
            onCommit={(draft) =>
              patchLoop(
                { elementVariable: draft.trim() || null },
                'Edit multi-instance element variable',
              )
            }
          />
          <TextRow
            property="multiInstanceCompletionCondition"
            label="Completion condition"
            value={loop.completionCondition ?? ''}
            multiline
            onCommit={(draft) =>
              patchLoop(
                { completionCondition: draft.trim() || null },
                'Edit multi-instance completion condition',
              )
            }
          />
        </>
      ) : null}
    </section>
  );
}

export function ListenersSection({ element }: { element: FlowElementEnum }) {
  const execute = useModelerStore((state) => state.execute);
  const elementId = element.id ?? '';
  const showTask = supportsTaskListeners(element);
  const showExecution = supportsExecutionListeners(element);
  if (!showTask && !showExecution) return null;

  const activity = asActivity(element);
  const taskListeners: FlowableListener[] = showTask ? (activity.taskListeners ?? []) : [];
  const executionListeners: FlowableListener[] = showExecution
    ? (activity.executionListeners ?? [])
    : [];

  return (
    <>
      {showTask ? (
        <ListenerList
          title="Task listeners"
          group="task-listeners"
          listeners={taskListeners}
          eventOptions={TASK_LISTENER_EVENTS}
          defaultEvent="create"
          onChange={(next, label) =>
            execute(updateElementPropertiesCommand(elementId, { taskListeners: next }, label))
          }
        />
      ) : null}
      {showExecution ? (
        <ListenerList
          title="Execution listeners"
          group="execution-listeners"
          listeners={executionListeners}
          eventOptions={EXECUTION_LISTENER_EVENTS}
          defaultEvent="start"
          onChange={(next, label) =>
            execute(
              updateElementPropertiesCommand(elementId, { executionListeners: next }, label),
            )
          }
        />
      ) : null}
    </>
  );
}

function ListenerList({
  defaultEvent,
  eventOptions,
  group,
  listeners,
  onChange,
  title,
}: {
  defaultEvent: string;
  eventOptions: readonly (readonly [string, string])[];
  group: string;
  listeners: FlowableListener[];
  onChange: (next: FlowableListener[], label: string) => void;
  title: string;
}) {
  return (
    <section data-property-group={group}>
      <h2>{title}</h2>
      {listeners.length === 0 ? (
        <p className="property-note">No listeners configured.</p>
      ) : (
        listeners.map((listener, index) => (
          <div key={index} className="advanced-row" data-listener-index={index}>
            <SelectRow
              property={`${group}-event-${index}`}
              label="Event"
              value={listener.event ?? ''}
              options={eventOptions}
              onCommit={(value) => {
                const next = listeners.map((entry, entryIndex) =>
                  entryIndex === index ? { ...entry, event: value || null } : entry,
                );
                onChange(next, `Edit ${title.toLowerCase()} event`);
              }}
            />
            <SelectRow
              property={`${group}-type-${index}`}
              label="Implementation type"
              value={listener.implementationType ?? ''}
              options={IMPLEMENTATION_OPTIONS}
              onCommit={(value) => {
                const next = listeners.map((entry, entryIndex) =>
                  entryIndex === index
                    ? { ...entry, implementationType: value || null }
                    : entry,
                );
                onChange(next, `Edit ${title.toLowerCase()} type`);
              }}
            />
            <TextRow
              property={`${group}-impl-${index}`}
              label="Implementation"
              value={listener.implementation ?? ''}
              onCommit={(draft) => {
                const next = listeners.map((entry, entryIndex) =>
                  entryIndex === index
                    ? { ...entry, implementation: draft.trim() || null }
                    : entry,
                );
                onChange(next, `Edit ${title.toLowerCase()} implementation`);
              }}
            />
            <button
              type="button"
              className="quiet-action is-danger"
              aria-label={`Remove ${title.toLowerCase()} ${index + 1}`}
              onClick={() =>
                onChange(
                  listeners.filter((_, entryIndex) => entryIndex !== index),
                  `Remove ${title.toLowerCase()}`,
                )
              }
            >
              Remove
            </button>
          </div>
        ))
      )}
      <button
        type="button"
        className="quiet-action"
        onClick={() =>
          onChange([...listeners, createEmptyListener(defaultEvent)], `Add ${title.toLowerCase()}`)
        }
      >
        + Add listener
      </button>
    </section>
  );
}

export function FieldInjectionSection({ element }: { element: FlowElementEnum }) {
  if (!supportsFieldInjection(element)) return null;
  const execute = useModelerStore((state) => state.execute);
  const elementId = element.id ?? '';
  const fields: FieldExtension[] = asActivity(element).fieldExtensions ?? [];

  const commit = (next: FieldExtension[], label: string) =>
    execute(updateElementPropertiesCommand(elementId, { fieldExtensions: next }, label));

  return (
    <section data-property-group="field-injection">
      <h2>Field injection</h2>
      {fields.length === 0 ? <p className="property-note">No field extensions.</p> : null}
      {fields.map((field, index) => (
        <div key={index} className="advanced-row" data-field-index={index}>
          <TextRow
            property={`fieldName-${index}`}
            label="Field name"
            value={field.fieldName ?? ''}
            onCommit={(draft) => {
              const next = fields.map((entry, entryIndex) =>
                entryIndex === index ? { ...entry, fieldName: draft.trim() || null } : entry,
              );
              commit(next, 'Edit field name');
            }}
          />
          <TextRow
            property={`fieldString-${index}`}
            label="String value"
            value={field.stringValue ?? ''}
            onCommit={(draft) => {
              const next = fields.map((entry, entryIndex) =>
                entryIndex === index
                  ? {
                      ...entry,
                      stringValue: draft.trim() || null,
                      expression: draft.trim() ? null : entry.expression,
                    }
                  : entry,
              );
              commit(next, 'Edit field string value');
            }}
          />
          <TextRow
            property={`fieldExpression-${index}`}
            label="Expression"
            value={field.expression ?? ''}
            onCommit={(draft) => {
              const next = fields.map((entry, entryIndex) =>
                entryIndex === index
                  ? {
                      ...entry,
                      expression: draft.trim() || null,
                      stringValue: draft.trim() ? null : entry.stringValue,
                    }
                  : entry,
              );
              commit(next, 'Edit field expression');
            }}
          />
          <button
            type="button"
            className="quiet-action is-danger"
            aria-label={`Remove field ${index + 1}`}
            onClick={() =>
              commit(
                fields.filter((_, entryIndex) => entryIndex !== index),
                'Remove field extension',
              )
            }
          >
            Remove
          </button>
        </div>
      ))}
      <button
        type="button"
        className="quiet-action"
        onClick={() => commit([...fields, createEmptyFieldExtension()], 'Add field extension')}
      >
        + Add field
      </button>
    </section>
  );
}

export function CallActivitySection({ element }: { element: FlowElementEnum }) {
  if (!supportsCallActivity(element) || element.elementType !== 'callActivity') return null;
  const execute = useModelerStore((state) => state.execute);
  const elementId = element.id ?? '';
  const inParameters = element.inParameters ?? [];
  const outParameters = element.outParameters ?? [];

  const commitProps = (properties: Record<string, unknown>, label: string) =>
    execute(updateElementPropertiesCommand(elementId, properties, label));

  return (
    <>
      <section data-property-group="call-activity">
        <h2>Called element</h2>
        <TextRow
          property="calledElement"
          label="Called element"
          value={element.calledElement ?? ''}
          onCommit={(draft) =>
            commitProps({ calledElement: draft.trim() || null }, 'Edit called element')
          }
        />
      </section>
      <ParameterList
        title="In parameters"
        group="in-parameters"
        parameters={inParameters}
        onChange={(next, label) => commitProps({ inParameters: next }, label)}
      />
      <ParameterList
        title="Out parameters"
        group="out-parameters"
        parameters={outParameters}
        onChange={(next, label) => commitProps({ outParameters: next }, label)}
      />
    </>
  );
}

function ParameterList({
  group,
  onChange,
  parameters,
  title,
}: {
  group: string;
  onChange: (next: IOParameter[], label: string) => void;
  parameters: IOParameter[];
  title: string;
}) {
  return (
    <section data-property-group={group}>
      <h2>{title}</h2>
      {parameters.length === 0 ? <p className="property-note">No parameters.</p> : null}
      {parameters.map((parameter, index) => (
        <div key={index} className="advanced-row" data-parameter-index={index}>
          <TextRow
            property={`${group}-source-${index}`}
            label="Source"
            value={parameter.source ?? ''}
            onCommit={(draft) => {
              const next = parameters.map((entry, entryIndex) =>
                entryIndex === index ? { ...entry, source: draft.trim() || null } : entry,
              );
              onChange(next, `Edit ${title.toLowerCase()} source`);
            }}
          />
          <TextRow
            property={`${group}-target-${index}`}
            label="Target"
            value={parameter.target ?? ''}
            onCommit={(draft) => {
              const next = parameters.map((entry, entryIndex) =>
                entryIndex === index ? { ...entry, target: draft.trim() || null } : entry,
              );
              onChange(next, `Edit ${title.toLowerCase()} target`);
            }}
          />
          <button
            type="button"
            className="quiet-action is-danger"
            aria-label={`Remove ${title.toLowerCase()} ${index + 1}`}
            onClick={() =>
              onChange(
                parameters.filter((_, entryIndex) => entryIndex !== index),
                `Remove ${title.toLowerCase()}`,
              )
            }
          >
            Remove
          </button>
        </div>
      ))}
      <button
        type="button"
        className="quiet-action"
        onClick={() =>
          onChange([...parameters, createEmptyIOParameter()], `Add ${title.toLowerCase()}`)
        }
      >
        + Add parameter
      </button>
    </section>
  );
}

export function GlobalDefinitionsSection({ document }: { document: BpmnEditorDocument }) {
  const execute = useModelerStore((state) => state.execute);
  const signals = document.model.signals ?? [];
  const messages = document.model.messages ?? [];

  const nextDefinitionId = (prefix: string, used: Set<string>) => {
    let counter = 1;
    let candidate = `${prefix}${counter}`;
    while (used.has(candidate)) {
      counter += 1;
      candidate = `${prefix}${counter}`;
    }
    return candidate;
  };

  return (
    <>
      <section data-property-group="signals">
        <h2>Signals</h2>
        {signals.length === 0 ? <p className="property-note">No signal definitions.</p> : null}
        {signals.map((signal, index) => (
          <div key={signal.id ?? index} className="advanced-row" data-signal-index={index}>
            <TextRow
              property={`signalId-${index}`}
              label="Signal id"
              value={signal.id ?? ''}
              onCommit={(draft) => {
                const id = draft.trim();
                if (!id) return;
                const next = signals.map((entry, entryIndex) =>
                  entryIndex === index ? { ...entry, id, name: entry.name || id } : entry,
                );
                execute(updateModelSignalsCommand(next));
              }}
            />
            <TextRow
              property={`signalName-${index}`}
              label="Signal name"
              value={signal.name ?? ''}
              onCommit={(draft) => {
                const next = signals.map((entry, entryIndex) =>
                  entryIndex === index ? { ...entry, name: draft.trim() || null } : entry,
                );
                execute(updateModelSignalsCommand(next));
              }}
            />
            <button
              type="button"
              className="quiet-action is-danger"
              aria-label={`Remove signal ${index + 1}`}
              onClick={() =>
                execute(updateModelSignalsCommand(signals.filter((_, i) => i !== index)))
              }
            >
              Remove
            </button>
          </div>
        ))}
        <button
          type="button"
          className="quiet-action"
          onClick={() => {
            const used = new Set(signals.map((entry) => entry.id).filter(Boolean) as string[]);
            const id = nextDefinitionId('signal', used);
            execute(updateModelSignalsCommand([...signals, createEmptySignal(id)]));
          }}
        >
          + Add signal
        </button>
      </section>
      <section data-property-group="messages">
        <h2>Messages</h2>
        {messages.length === 0 ? <p className="property-note">No message definitions.</p> : null}
        {messages.map((message, index) => (
          <div key={message.id ?? index} className="advanced-row" data-message-index={index}>
            <TextRow
              property={`messageId-${index}`}
              label="Message id"
              value={message.id ?? ''}
              onCommit={(draft) => {
                const id = draft.trim();
                if (!id) return;
                const next = messages.map((entry, entryIndex) =>
                  entryIndex === index ? { ...entry, id, name: entry.name || id } : entry,
                );
                execute(updateModelMessagesCommand(next));
              }}
            />
            <TextRow
              property={`messageName-${index}`}
              label="Message name"
              value={message.name ?? ''}
              onCommit={(draft) => {
                const next = messages.map((entry, entryIndex) =>
                  entryIndex === index ? { ...entry, name: draft.trim() || null } : entry,
                );
                execute(updateModelMessagesCommand(next));
              }}
            />
            <button
              type="button"
              className="quiet-action is-danger"
              aria-label={`Remove message ${index + 1}`}
              onClick={() =>
                execute(updateModelMessagesCommand(messages.filter((_, i) => i !== index)))
              }
            >
              Remove
            </button>
          </div>
        ))}
        <button
          type="button"
          className="quiet-action"
          onClick={() => {
            const used = new Set(messages.map((entry) => entry.id).filter(Boolean) as string[]);
            const id = nextDefinitionId('message', used);
            execute(updateModelMessagesCommand([...messages, createEmptyMessage(id)]));
          }}
        >
          + Add message
        </button>
      </section>
    </>
  );
}

export function EventReferenceSection({
  document,
  element,
}: {
  document: BpmnEditorDocument;
  element: FlowElementEnum;
}) {
  if (!('eventDefinitions' in element)) return null;
  const definitions = (element.eventDefinitions ?? []) as EventDefinitionEnum[];
  const signalDefinition = definitions.find(
    (definition) => definition.eventDefinitionType === 'signalEventDefinition',
  );
  const messageDefinition = definitions.find(
    (definition) => definition.eventDefinitionType === 'messageEventDefinition',
  );
  // Show the ref editors when a matching definition exists, or always offer
  // both for pure intermediate/boundary/start/end events so authors can attach one.
  const isEventElement =
    element.elementType === 'startEvent' ||
    element.elementType === 'endEvent' ||
    element.elementType === 'boundaryEvent' ||
    element.elementType === 'intermediateCatchEvent' ||
    element.elementType === 'intermediateThrowEvent';
  if (!isEventElement && !signalDefinition && !messageDefinition) return null;

  const execute = useModelerStore((state) => state.execute);
  const elementId = element.id ?? '';
  const signals = document.model.signals ?? [];
  const messages = document.model.messages ?? [];
  const signalRef =
    signalDefinition && 'signalRef' in signalDefinition
      ? (signalDefinition.signalRef ?? '')
      : '';
  const messageRef =
    messageDefinition && 'messageRef' in messageDefinition
      ? (messageDefinition.messageRef ?? '')
      : '';

  // Only show signal/message groups that are relevant or already present.
  const showSignal = Boolean(signalDefinition) || (isEventElement && !messageDefinition);
  const showMessage = Boolean(messageDefinition) || (isEventElement && !signalDefinition);

  return (
    <section data-property-group="event-references">
      <h2>Event references</h2>
      {showSignal ? (
        <SelectRow
          property="signalRef"
          label="Signal"
          value={signalRef}
          options={signals
            .filter((signal): signal is Signal & { id: string } => Boolean(signal.id))
            .map((signal) => [signal.id, signal.name || signal.id] as const)}
          onCommit={(value) =>
            execute(
              updateEventDefinitionRefCommand(
                elementId,
                'signalEventDefinition',
                value.trim() || null,
              ),
            )
          }
        />
      ) : null}
      {showMessage ? (
        <SelectRow
          property="messageRef"
          label="Message"
          value={messageRef}
          options={messages
            .filter((message): message is Message & { id: string } => Boolean(message.id))
            .map((message) => [message.id, message.name || message.id] as const)}
          onCommit={(value) =>
            execute(
              updateEventDefinitionRefCommand(
                elementId,
                'messageEventDefinition',
                value.trim() || null,
              ),
            )
          }
        />
      ) : null}
    </section>
  );
}

function TextRow({
  label,
  multiline,
  onCommit,
  property,
  value,
}: {
  label: string;
  multiline?: boolean;
  onCommit: (draft: string) => void;
  property: string;
  value: string;
}) {
  return (
    <div className="property-field">
      <label className="property-label" htmlFor={`property-${property}`}>
        {label}
      </label>
      {multiline ? (
        <textarea
          id={`property-${property}`}
          aria-label={label}
          data-property={property}
          className="property-input"
          rows={2}
          defaultValue={value}
          key={`${property}:${value}`}
          onBlur={(event) => onCommit(event.currentTarget.value)}
        />
      ) : (
        <input
          id={`property-${property}`}
          aria-label={label}
          data-property={property}
          className="property-input"
          type="text"
          defaultValue={value}
          key={`${property}:${value}`}
          onBlur={(event) => onCommit(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') event.currentTarget.blur();
          }}
        />
      )}
    </div>
  );
}

function SelectRow({
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
