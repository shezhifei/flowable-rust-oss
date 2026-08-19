import type {
  BpmnEditorDocument,
  Escalation,
  EventDefinitionEnum,
  FieldExtension,
  FlowableListener,
  FlowElementEnum,
  FormProperty,
  IOParameter,
  Message,
  MultiInstanceLoopCharacteristics,
  Signal,
} from '../generated/editor-protocol';
import { useModelerStore } from './modelerStore';
import {
  createEmptyEscalation,
  createEmptyFieldExtension,
  createEmptyFormProperty,
  createEmptyIOParameter,
  createEmptyListener,
  createEmptyLoopCharacteristics,
  createEmptyMessage,
  createEmptySignal,
  updateElementPropertiesCommand,
  updateEventDefinitionCodeCommand,
  updateEventDefinitionRefCommand,
  updateFormPropertiesCommand,
  updateModelEscalationsCommand,
  updateModelMessagesCommand,
  updateModelSignalsCommand,
  updateTimerDefinitionCommand,
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

function supportsMultiInstance(element: FlowElementEnum): boolean {
  return MULTI_INSTANCE_TYPES.has(element.elementType) && 'loopCharacteristics' in element;
}

function supportsExecutionListeners(element: FlowElementEnum): boolean {
  return EXECUTION_LISTENER_TYPES.has(element.elementType) && 'executionListeners' in element;
}

function supportsTaskListeners(element: FlowElementEnum): boolean {
  return element.elementType === 'userTask' && 'taskListeners' in element;
}

function supportsFieldInjection(element: FlowElementEnum): boolean {
  return (
    (element.elementType === 'serviceTask' || element.elementType === 'callActivity') &&
    'fieldExtensions' in element
  );
}

function supportsCallActivity(element: FlowElementEnum): boolean {
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
  const execute = useModelerStore((state) => state.execute);
  if (!supportsMultiInstance(element)) return null;
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
  const showTask = supportsTaskListeners(element);
  const showExecution = supportsExecutionListeners(element);
  if (!showTask && !showExecution) return null;
  const elementId = element.id ?? '';
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
  const execute = useModelerStore((state) => state.execute);
  if (!supportsFieldInjection(element)) return null;
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
  const execute = useModelerStore((state) => state.execute);
  if (!supportsCallActivity(element) || element.elementType !== 'callActivity') return null;
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
  const escalations = document.model.escalations ?? [];

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
      <section data-property-group="escalations">
        <h2>Escalations</h2>
        {escalations.length === 0 ? (
          <p className="property-note">No escalation definitions.</p>
        ) : null}
        {escalations.map((escalation, index) => (
          <div key={escalation.id ?? index} className="advanced-row" data-escalation-index={index}>
            <TextRow
              property={`escalationId-${index}`}
              label="Escalation id"
              value={escalation.id ?? ''}
              onCommit={(draft) => {
                const id = draft.trim();
                if (!id) return;
                const next = escalations.map((entry, entryIndex) =>
                  entryIndex === index ? { ...entry, id, name: entry.name || id } : entry,
                );
                execute(updateModelEscalationsCommand(next));
              }}
            />
            <TextRow
              property={`escalationName-${index}`}
              label="Escalation name"
              value={escalation.name ?? ''}
              onCommit={(draft) => {
                const next = escalations.map((entry, entryIndex) =>
                  entryIndex === index ? { ...entry, name: draft.trim() || null } : entry,
                );
                execute(updateModelEscalationsCommand(next));
              }}
            />
            <TextRow
              property={`escalationCatalogCode-${index}`}
              label="Escalation code"
              value={escalation.escalationCode ?? ''}
              onCommit={(draft) => {
                const next = escalations.map((entry, entryIndex) =>
                  entryIndex === index
                    ? { ...entry, escalationCode: draft.trim() || null }
                    : entry,
                );
                execute(updateModelEscalationsCommand(next));
              }}
            />
            <button
              type="button"
              className="quiet-action is-danger"
              aria-label={`Remove escalation ${index + 1}`}
              onClick={() =>
                execute(updateModelEscalationsCommand(escalations.filter((_, i) => i !== index)))
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
            const used = new Set(escalations.map((entry) => entry.id).filter(Boolean) as string[]);
            const id = nextDefinitionId('escalation', used);
            execute(updateModelEscalationsCommand([...escalations, createEmptyEscalation(id)]));
          }}
        >
          + Add escalation
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
  const execute = useModelerStore((state) => state.execute);
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

const EVENT_ELEMENT_TYPES = new Set<FlowElementEnum['elementType']>([
  'startEvent',
  'endEvent',
  'boundaryEvent',
  'intermediateCatchEvent',
  'intermediateThrowEvent',
]);

/**
 * Error and escalation editors. Both take a free-text code alongside the
 * catalog reference because Flowable matches thrown errors by code — a boundary
 * error event works with a code alone, no `<error>` declaration needed. Errors
 * have no catalog to pick from (the model carries `errors` as a plain code map),
 * so `errorRef` is a text field; escalations select from `model.escalations`.
 */
export function ErrorEscalationSection({
  document,
  element,
}: {
  document: BpmnEditorDocument;
  element: FlowElementEnum;
}) {
  const execute = useModelerStore((state) => state.execute);
  if (!('eventDefinitions' in element)) return null;
  const definitions = (element.eventDefinitions ?? []) as EventDefinitionEnum[];
  const errorDefinition = definitions.find(
    (definition) => definition.eventDefinitionType === 'errorEventDefinition',
  );
  const escalationDefinition = definitions.find(
    (definition) => definition.eventDefinitionType === 'escalationEventDefinition',
  );
  if (!errorDefinition && !escalationDefinition && !EVENT_ELEMENT_TYPES.has(element.elementType)) {
    return null;
  }

  const elementId = element.id ?? '';
  const escalations = document.model.escalations ?? [];
  const fieldOf = (definition: EventDefinitionEnum | undefined, field: string) =>
    definition && field in definition ? ((definition[field] as string | null) ?? '') : '';

  return (
    <section data-property-group="error-escalation">
      <h2>Errors and escalations</h2>
      <TextRow
        property="errorRef"
        label="Error ref"
        value={fieldOf(errorDefinition, 'errorRef')}
        onCommit={(draft) =>
          execute(
            updateEventDefinitionRefCommand(elementId, 'errorEventDefinition', draft.trim() || null),
          )
        }
      />
      <TextRow
        property="errorCode"
        label="Error code"
        value={fieldOf(errorDefinition, 'errorCode')}
        onCommit={(draft) =>
          execute(
            updateEventDefinitionCodeCommand(
              elementId,
              'errorEventDefinition',
              draft.trim() || null,
            ),
          )
        }
      />
      <SelectRow
        property="escalationRef"
        label="Escalation"
        value={fieldOf(escalationDefinition, 'escalationRef')}
        options={escalations
          .filter((escalation): escalation is Escalation & { id: string } => Boolean(escalation.id))
          .map((escalation) => [escalation.id, escalation.name || escalation.id] as const)}
        onCommit={(value) =>
          execute(
            updateEventDefinitionRefCommand(
              elementId,
              'escalationEventDefinition',
              value.trim() || null,
            ),
          )
        }
      />
      <TextRow
        property="escalationCode"
        label="Escalation code"
        value={fieldOf(escalationDefinition, 'escalationCode')}
        onCommit={(draft) =>
          execute(
            updateEventDefinitionCodeCommand(
              elementId,
              'escalationEventDefinition',
              draft.trim() || null,
            ),
          )
        }
      />
    </section>
  );
}

const TIMER_ELEMENT_TYPES = new Set<FlowElementEnum['elementType']>([
  'startEvent',
  'boundaryEvent',
  'intermediateCatchEvent',
]);

const TIMER_KINDS = [
  ['timeDuration', 'Duration'],
  ['timeDate', 'Date'],
  ['timeCycle', 'Cycle'],
] as const;

type TimerKind = (typeof TIMER_KINDS)[number][0];

const TIMER_KIND_HINTS: Record<TimerKind, string> = {
  timeCycle: 'ISO repeating interval or cron, e.g. R3/PT10M',
  timeDate: 'ISO 8601 instant, e.g. 2026-12-24T09:00:00Z',
  timeDuration: 'ISO 8601 duration, e.g. PT48H',
};

/**
 * Timer editor for the events that can carry a `timerEventDefinition`. The three
 * timer kinds are mutually exclusive in BPMN, so this offers a kind selector
 * plus one value editor rather than three independent fields; switching kinds
 * carries the value over and clears the others.
 */
export function TimerDefinitionSection({ element }: { element: FlowElementEnum }) {
  const execute = useModelerStore((state) => state.execute);
  if (!('eventDefinitions' in element)) return null;
  const definitions = (element.eventDefinitions ?? []) as EventDefinitionEnum[];
  const timer = definitions.find(
    (definition) => definition.eventDefinitionType === 'timerEventDefinition',
  );
  // Offer the editor on timer-capable events before a definition exists, but
  // never hide one that is already attached to some other element type.
  if (!timer && !TIMER_ELEMENT_TYPES.has(element.elementType)) return null;

  const elementId = element.id ?? '';
  const valueOf = (kind: TimerKind) =>
    timer && kind in timer ? ((timer[kind] as string | null) ?? '') : '';
  const stringField = (field: 'calendarName' | 'endDate') =>
    timer && field in timer ? ((timer[field] as string | null) ?? '') : '';
  // The stored kind wins; an unconfigured timer defaults to duration.
  const activeKind = TIMER_KINDS.find(([kind]) => valueOf(kind) !== '')?.[0] ?? 'timeDuration';
  const activeLabel = TIMER_KINDS.find(([kind]) => kind === activeKind)?.[1] ?? 'Value';

  return (
    <section data-property-group="timer-definition">
      <h2>Timer</h2>
      <SelectRow
        property="timerType"
        label="Timer type"
        value={activeKind}
        includeNone={false}
        options={TIMER_KINDS}
        onCommit={(value) => {
          const kind = value as TimerKind;
          if (kind === activeKind) return;
          // Carry the current value across so switching kinds is not data loss.
          execute(updateTimerDefinitionCommand(elementId, { [kind]: valueOf(activeKind) || null }));
        }}
      />
      <TextRow
        property={activeKind}
        label={`${activeLabel} — ${TIMER_KIND_HINTS[activeKind]}`}
        value={valueOf(activeKind)}
        onCommit={(draft) =>
          execute(updateTimerDefinitionCommand(elementId, { [activeKind]: draft.trim() || null }))
        }
      />
      <TextRow
        property="calendarName"
        label="Business calendar"
        value={stringField('calendarName')}
        onCommit={(draft) =>
          execute(updateTimerDefinitionCommand(elementId, { calendarName: draft.trim() || null }))
        }
      />
      {activeKind === 'timeCycle' ? (
        <TextRow
          property="endDate"
          label="End date — stops the cycle"
          value={stringField('endDate')}
          onCommit={(draft) =>
            execute(updateTimerDefinitionCommand(elementId, { endDate: draft.trim() || null }))
          }
        />
      ) : null}
    </section>
  );
}

const FORM_PROPERTY_ELEMENT_TYPES = new Set<FlowElementEnum['elementType']>([
  'startEvent',
  'userTask',
]);

const FORM_PROPERTY_TYPES = [
  ['string', 'String'],
  ['long', 'Long'],
  ['boolean', 'Boolean'],
  ['date', 'Date'],
  ['enum', 'Enum'],
] as const;

/**
 * Inline form editor for the two element types Java lets carry
 * `flowable:formProperty`: a user task form and a start form. Each row is one
 * form field, and the whole list is written back as a unit so an add, an edit or
 * a removal is a single undo step.
 *
 * `formValues` — the choices behind an enum field — are carried through
 * untouched rather than edited here; the row shows them read-only so a field of
 * type Enum does not look empty.
 */
export function FormPropertiesSection({ element }: { element: FlowElementEnum }) {
  const execute = useModelerStore((state) => state.execute);
  if (!FORM_PROPERTY_ELEMENT_TYPES.has(element.elementType)) return null;
  const elementId = element.id ?? '';
  const properties: FormProperty[] =
    (element as FlowElementEnum & { formProperties?: FormProperty[] }).formProperties ?? [];

  const commit = (next: FormProperty[], label: string) =>
    execute(updateFormPropertiesCommand(elementId, next, label));

  const patchRow = (index: number, patch: Partial<FormProperty>, label: string) => {
    const next = properties.map((entry, entryIndex) =>
      entryIndex === index ? { ...entry, ...patch } : entry,
    );
    commit(next, label);
  };

  return (
    <section data-property-group="form-properties">
      <h2>Form properties</h2>
      {properties.length === 0 ? <p className="property-note">No form properties.</p> : null}
      {properties.map((property, index) => (
        <div key={index} className="advanced-row" data-form-property-index={index}>
          <TextRow
            property={`formPropertyId-${index}`}
            label="Id"
            value={property.id ?? ''}
            onCommit={(draft) =>
              patchRow(index, { id: draft.trim() || null }, 'Edit form property id')
            }
          />
          <TextRow
            property={`formPropertyName-${index}`}
            label="Name"
            value={property.name ?? ''}
            onCommit={(draft) =>
              patchRow(index, { name: draft.trim() || null }, 'Edit form property name')
            }
          />
          <SelectRow
            property={`formPropertyType-${index}`}
            label="Type"
            value={property.type ?? 'string'}
            includeNone={false}
            options={FORM_PROPERTY_TYPES}
            onCommit={(value) => patchRow(index, { type: value }, 'Edit form property type')}
          />
          <TextRow
            property={`formPropertyVariable-${index}`}
            label="Variable"
            value={property.variable ?? ''}
            onCommit={(draft) =>
              patchRow(index, { variable: draft.trim() || null }, 'Edit form property variable')
            }
          />
          {property.type === 'date' ? (
            <TextRow
              property={`formPropertyDatePattern-${index}`}
              label="Date pattern, e.g. dd-MM-yyyy hh:mm"
              value={property.datePattern ?? ''}
              onCommit={(draft) =>
                patchRow(
                  index,
                  { datePattern: draft.trim() || null },
                  'Edit form property date pattern',
                )
              }
            />
          ) : null}
          {property.type === 'enum' ? (
            <p className="property-note">
              {property.formValues.length === 0
                ? 'No enum values on this field.'
                : `Enum values: ${property.formValues
                    .map((value) => value.id ?? value.name ?? '')
                    .filter(Boolean)
                    .join(', ')}`}
            </p>
          ) : null}
          <CheckboxRow
            property={`formPropertyRequired-${index}`}
            label="Required"
            checked={Boolean(property.required)}
            onCommit={(checked) =>
              patchRow(index, { required: checked }, 'Edit form property required')
            }
          />
          <CheckboxRow
            property={`formPropertyReadable-${index}`}
            label="Readable"
            checked={Boolean(property.readable)}
            onCommit={(checked) =>
              patchRow(index, { readable: checked }, 'Edit form property readable')
            }
          />
          <CheckboxRow
            property={`formPropertyWriteable-${index}`}
            label="Writeable"
            checked={Boolean(property.writeable)}
            onCommit={(checked) =>
              patchRow(index, { writeable: checked }, 'Edit form property writeable')
            }
          />
          <button
            type="button"
            className="quiet-action is-danger"
            aria-label={`Remove form property ${index + 1}`}
            onClick={() =>
              commit(
                properties.filter((_, entryIndex) => entryIndex !== index),
                'Remove form property',
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
        onClick={() => {
          const used = new Set(properties.map((entry) => entry.id).filter(Boolean) as string[]);
          let counter = 1;
          let id = `formProperty${counter}`;
          while (used.has(id)) {
            counter += 1;
            id = `formProperty${counter}`;
          }
          commit([...properties, createEmptyFormProperty(id)], 'Add form property');
        }}
      >
        + Add form property
      </button>
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

function CheckboxRow({
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

function SelectRow({
  includeNone = true,
  label,
  onCommit,
  options,
  property,
  value,
}: {
  includeNone?: boolean;
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
        {includeNone ? <option value="">None</option> : null}
        {options.map(([optionValue, optionLabel]) => (
          <option key={optionValue} value={optionValue}>
            {optionLabel}
          </option>
        ))}
      </select>
    </div>
  );
}
