import type { Draft } from 'immer';

import type {
  ArtifactEnum,
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
import type { ModelerCommand } from './commands';
import { findDiagramShape } from './diagramModel';
import { locateCanonicalElement, normalizeModelInvariants } from './modelInvariants';
import { collectModelIds, validateElementId } from './propertyValidation';

export type PropertyCommandErrorCode =
  'duplicate-element-id' | 'invalid-element-id' | 'missing-element' | 'missing-process';

export class PropertyCommandError extends Error {
  readonly code: PropertyCommandErrorCode;
  readonly targetId: string;

  constructor(code: PropertyCommandErrorCode, targetId: string, message: string) {
    super(message);
    this.name = 'PropertyCommandError';
    this.code = code;
    this.targetId = targetId;
  }
}

/**
 * Applies already-validated property values to one flow element, data object,
 * pool, lane, or artifact. Unknown ids abort the command so nothing
 * half-writes.
 */
export function updateElementPropertiesCommand(
  elementId: string,
  properties: Record<string, unknown>,
  label?: string,
): ModelerCommand {
  return {
    label: label ?? `Edit ${elementId} properties`,
    apply(document) {
      const target = locateEditableShape(document, elementId);
      if (!target) {
        throw new PropertyCommandError(
          'missing-element',
          elementId,
          `${elementId} is not part of this document`,
        );
      }
      Object.assign(target, properties);
      normalizeModelInvariants(document);
    },
  };
}

/**
 * The mutable object behind an id, across every collection the canvas can
 * select from. `locateCanonicalElement` covers the process tree; pools, lanes,
 * and artifacts are reachable only through the document-level collections.
 */
function locateEditableShape(
  document: Draft<BpmnEditorDocument>,
  elementId: string,
): Record<string, unknown> | null {
  const located = locateCanonicalElement(document, elementId);
  if (located) return located.element as Record<string, unknown>;
  const shape = findDiagramShape(document, elementId);
  if (!shape) return null;
  switch (shape.kind) {
    case 'pool':
      return shape.pool as Record<string, unknown>;
    case 'lane':
      return shape.lane as Record<string, unknown>;
    case 'artifact':
      return shape.artifact as Record<string, unknown>;
    default:
      return shape.element as Record<string, unknown>;
  }
}

/**
 * Renames an element id and rewires every diagram-owned reference to it:
 * DI maps, sequence flow endpoints, boundary attachments, default flows,
 * lane memberships, message flows, and association endpoints. Pools, lanes,
 * and artifacts rename through the same graph.
 */
export function renameElementIdCommand(elementId: string, nextId: string): ModelerCommand {
  return {
    label: `Rename ${elementId} to ${nextId}`,
    apply(document) {
      const validationError = validateElementId(document, elementId, nextId);
      if (validationError) {
        throw new PropertyCommandError(
          collectModelIds(document).has(nextId.trim()) && nextId.trim() !== elementId
            ? 'duplicate-element-id'
            : 'invalid-element-id',
          elementId,
          validationError,
        );
      }
      const trimmed = nextId.trim();
      if (trimmed === elementId) return;
      const target = locateEditableShape(document, elementId);
      if (!target) {
        throw new PropertyCommandError(
          'missing-element',
          elementId,
          `${elementId} is not part of this document`,
        );
      }
      target.id = trimmed;

      moveKeyedEntry(document.model.locationMap, elementId, trimmed);
      moveKeyedEntry(document.model.labelLocationMap, elementId, trimmed);
      moveKeyedEntry(document.model.flowLocationMap, elementId, trimmed);
      moveKeyedEntry(document.model.edgeMap, elementId, trimmed);

      for (const process of document.model.processes) {
        rewireFlowElementReferences(process.flowElements ?? [], elementId, trimmed);
        for (const lane of process.lanes ?? []) {
          lane.flowReferences = lane.flowReferences.map((reference) =>
            reference === elementId ? trimmed : reference,
          );
        }
        rewireArtifactReferences(process.artifacts ?? [], elementId, trimmed);
      }
      for (const flow of Object.values(document.model.messageFlows)) {
        if (flow.sourceRef === elementId) flow.sourceRef = trimmed;
        if (flow.targetRef === elementId) flow.targetRef = trimmed;
      }
      rewireArtifactReferences(document.model.globalArtifacts, elementId, trimmed);
      normalizeModelInvariants(document);
    },
  };
}

export interface ProcessPropertyUpdate {
  documentation?: string | null;
  id?: string;
  name?: string | null;
}

/**
 * Edits one process. `processId` names the target so a multi-pool document can
 * reach every participant's process; without it the command edits the main
 * process, which is what the no-selection panel shows. A pool `processRef`
 * follows a process id rename.
 */
export function updateProcessPropertiesCommand(
  properties: ProcessPropertyUpdate,
  processId?: string | null,
): ModelerCommand {
  const summary = properties.id ?? processId ?? 'process';
  return {
    label: `Edit ${summary} process properties`,
    apply(document) {
      const process = processId
        ? document.model.processes.find((candidate) => candidate.id === processId)
        : document.model.processes[0];
      if (!process) {
        throw new PropertyCommandError(
          'missing-process',
          summary,
          processId ? `the document has no process '${processId}'` : 'the document has no process',
        );
      }
      if (properties.id !== undefined) {
        const validationError = validateElementId(document, process.id ?? null, properties.id);
        if (validationError) {
          throw new PropertyCommandError(
            collectModelIds(document).has(properties.id.trim())
              ? 'duplicate-element-id'
              : 'invalid-element-id',
            process.id ?? summary,
            validationError,
          );
        }
        const previousId = process.id ?? null;
        process.id = properties.id.trim();
        for (const pool of document.model.pools) {
          if (pool.processRef === previousId) pool.processRef = process.id;
        }
      }
      if (properties.name !== undefined) process.name = properties.name;
      if (properties.documentation !== undefined) process.documentation = properties.documentation;
      normalizeModelInvariants(document);
    },
  };
}

/** Replaces the document-level signal definitions (process/event refs pick from this list). */
export function updateModelSignalsCommand(signals: Signal[]): ModelerCommand {
  return {
    label: 'Edit signal definitions',
    apply(document) {
      document.model.signals = signals;
      normalizeModelInvariants(document);
    },
  };
}

/** Replaces the document-level message definitions. */
export function updateModelMessagesCommand(messages: Message[]): ModelerCommand {
  return {
    label: 'Edit message definitions',
    apply(document) {
      document.model.messages = messages;
      normalizeModelInvariants(document);
    },
  };
}

/** Replaces the document-level escalation definitions. */
export function updateModelEscalationsCommand(escalations: Escalation[]): ModelerCommand {
  return {
    label: 'Edit escalation definitions',
    apply(document) {
      document.model.escalations = escalations;
      normalizeModelInvariants(document);
    },
  };
}

/** Event definition types the panel can attach a reference to. */
export type ReferencableDefinitionType =
  | 'errorEventDefinition'
  | 'escalationEventDefinition'
  | 'messageEventDefinition'
  | 'signalEventDefinition';

/** Definition types that carry a literal code beside their catalog reference. */
export type CodedDefinitionType = 'errorEventDefinition' | 'escalationEventDefinition';

const REFERENCE_FIELDS: Record<ReferencableDefinitionType, string> = {
  errorEventDefinition: 'errorRef',
  escalationEventDefinition: 'escalationRef',
  messageEventDefinition: 'messageRef',
  signalEventDefinition: 'signalRef',
};

const CODE_FIELDS: Record<CodedDefinitionType, string> = {
  errorEventDefinition: 'errorCode',
  escalationEventDefinition: 'escalationCode',
};

/**
 * Applies `patch` to the element's first event definition of `definitionType`,
 * creating one from `seed` when the event has none. Shared by every
 * event-definition command so find-or-create and the two failure modes —
 * unknown element, element that cannot hold definitions — stay in one place.
 */
function patchEventDefinition(
  document: Draft<BpmnEditorDocument>,
  elementId: string,
  definitionType: EventDefinitionEnum['eventDefinitionType'],
  patch: Record<string, unknown>,
  seed: Record<string, unknown> = {},
) {
  const located = locateCanonicalElement(document, elementId);
  if (!located) {
    throw new PropertyCommandError(
      'missing-element',
      elementId,
      `${elementId} is not part of this document`,
    );
  }
  const element = located.element as Draft<FlowElementEnum> & {
    eventDefinitions?: Draft<EventDefinitionEnum>[];
  };
  if (!('eventDefinitions' in element)) {
    throw new PropertyCommandError(
      'missing-element',
      elementId,
      `${elementId} does not carry event definitions`,
    );
  }
  const definitions = (element.eventDefinitions ??= []);
  const existing = definitions.find(
    (candidate) => candidate.eventDefinitionType === definitionType,
  );
  if (existing) {
    Object.assign(existing, patch);
  } else {
    definitions.push({
      eventDefinitionType: definitionType,
      id: `${elementId}_${definitionType}`,
      attributes: {},
      extensionElements: {},
      xmlColumnNumber: 0,
      xmlRowNumber: 0,
      ...seed,
      ...patch,
    } as Draft<EventDefinitionEnum>);
  }
  normalizeModelInvariants(document);
}

/**
 * Points an event definition at a catalog entry — signal, message, error or
 * escalation. Creates the definition entry when none exists yet so the panel can
 * seed a reference without a separate create step, and reuses the existing one
 * otherwise rather than stacking duplicates of the same type.
 *
 * A `null` ref clears the reference but keeps the definition, so an error event
 * stays an error event while its code is what identifies it.
 */
export function updateEventDefinitionRefCommand(
  elementId: string,
  definitionType: ReferencableDefinitionType,
  ref: string | null,
): ModelerCommand {
  const field = REFERENCE_FIELDS[definitionType];
  return {
    label: `Edit ${field} on ${elementId}`,
    apply(document) {
      patchEventDefinition(
        document,
        elementId,
        definitionType,
        { [field]: ref },
        definitionType in CODE_FIELDS ? { [CODE_FIELDS[definitionType as CodedDefinitionType]]: null } : {},
      );
    },
  };
}

/**
 * Sets the literal `errorCode` / `escalationCode` on an event definition. Codes
 * are independent of the catalog reference: Flowable matches a thrown error by
 * code, so a boundary event can carry one without any `<error>` declaration.
 */
export function updateEventDefinitionCodeCommand(
  elementId: string,
  definitionType: CodedDefinitionType,
  code: string | null,
): ModelerCommand {
  const field = CODE_FIELDS[definitionType];
  return {
    label: `Edit ${field} on ${elementId}`,
    apply(document) {
      patchEventDefinition(
        document,
        elementId,
        definitionType,
        { [field]: code },
        { [REFERENCE_FIELDS[definitionType]]: null },
      );
    },
  };
}

/** Element types that carry an inline form; Java allows it on these two only. */
const FORM_PROPERTY_TYPES = new Set<FlowElementEnum['elementType']>(['startEvent', 'userTask']);

/**
 * Replaces an element's inline form definition wholesale. The panel edits the
 * list as a unit — add, remove and reorder are all one list write — so a single
 * replace keeps every row change a single undo step.
 */
export function updateFormPropertiesCommand(
  elementId: string,
  next: FormProperty[],
  label?: string,
): ModelerCommand {
  return {
    label: label ?? `Edit form properties on ${elementId}`,
    apply(document) {
      const located = locateCanonicalElement(document, elementId);
      if (!located) {
        throw new PropertyCommandError(
          'missing-element',
          elementId,
          `${elementId} is not part of this document`,
        );
      }
      if (located.kind !== 'flowElement' || !FORM_PROPERTY_TYPES.has(located.element.elementType)) {
        throw new PropertyCommandError(
          'missing-element',
          elementId,
          `${elementId} does not carry form properties`,
        );
      }
      (
        located.element as Draft<FlowElementEnum> & { formProperties?: Draft<FormProperty>[] }
      ).formProperties = next as Draft<FormProperty>[];
      normalizeModelInvariants(document);
    },
  };
}

/** Fields a timer editor may write. Absent keys are left untouched. */
export type TimerDefinitionFields = {
  calendarName?: string | null;
  endDate?: string | null;
  timeCycle?: string | null;
  timeDate?: string | null;
  timeDuration?: string | null;
};

/** The three mutually exclusive timer kinds; BPMN allows at most one. */
const TIMER_KIND_FIELDS = ['timeDate', 'timeCycle', 'timeDuration'] as const;

/**
 * Writes timer fields onto an event's `timerEventDefinition`, creating the
 * definition when the event has none yet.
 *
 * `timeDate`, `timeCycle` and `timeDuration` are mutually exclusive in BPMN, so
 * naming any one of them clears the other two — including when the value is
 * `null`, which leaves the timer unconfigured rather than falling back to a
 * stale kind. `calendarName` and `endDate` apply to whichever kind is set and
 * never disturb it.
 */
export function updateTimerDefinitionCommand(
  elementId: string,
  fields: TimerDefinitionFields,
): ModelerCommand {
  const kind = TIMER_KIND_FIELDS.find((field) => field in fields);
  const patch: Record<string, string | null> = {};
  if (kind) {
    for (const field of TIMER_KIND_FIELDS) {
      patch[field] = field === kind ? (fields[kind] ?? null) : null;
    }
  }
  if ('calendarName' in fields) patch.calendarName = fields.calendarName ?? null;
  if ('endDate' in fields) patch.endDate = fields.endDate ?? null;

  return {
    label: `Edit timer on ${elementId}`,
    apply(document) {
      patchEventDefinition(document, elementId, 'timerEventDefinition', patch, {
        timeDate: null,
        timeCycle: null,
        timeDuration: null,
        calendarName: null,
        endDate: null,
      });
    },
  };
}

/** Empty multi-instance characteristics used when enabling the MI group. */
export function createEmptyLoopCharacteristics(
  sequential = false,
): MultiInstanceLoopCharacteristics {
  return {
    attributes: {},
    extensionElements: {},
    sequential,
    noWaitStatesAsyncLeave: false,
    collectionString: null,
    elementVariable: null,
    completionCondition: null,
    loopCardinality: null,
    xmlColumnNumber: 0,
    xmlRowNumber: 0,
  };
}

export function createEmptyListener(event: string): FlowableListener {
  return {
    attributes: {},
    extensionElements: {},
    event,
    implementation: '',
    implementationType: 'class',
    xmlColumnNumber: 0,
    xmlRowNumber: 0,
  };
}

export function createEmptyFieldExtension(): FieldExtension {
  return {
    attributes: {},
    extensionElements: {},
    fieldName: '',
    stringValue: null,
    expression: null,
    xmlColumnNumber: 0,
    xmlRowNumber: 0,
  };
}

export function createEmptyIOParameter(): IOParameter {
  return {
    attributes: {},
    extensionElements: {},
    source: '',
    target: '',
    transient: false,
    xmlColumnNumber: 0,
    xmlRowNumber: 0,
  };
}

export function createEmptySignal(id: string): Signal {
  return {
    attributes: {},
    extensionElements: {},
    id,
    name: id,
    scope: 'global',
    xmlColumnNumber: 0,
    xmlRowNumber: 0,
  };
}

export function createEmptyMessage(id: string): Message {
  return {
    attributes: {},
    extensionElements: {},
    id,
    name: id,
    xmlColumnNumber: 0,
    xmlRowNumber: 0,
  };
}

/**
 * A new escalation for the document catalog. `escalationCode` is what Flowable
 * matches at runtime, so it defaults to the id rather than staying empty.
 */
export function createEmptyEscalation(id: string): Escalation {
  return {
    attributes: {},
    escalationCode: id,
    extensionElements: {},
    id,
    name: id,
    xmlColumnNumber: 0,
    xmlRowNumber: 0,
  };
}

/**
 * A new form field. `readable` and `writeable` default to true to match the BPMN
 * defaults — the XML attributes only ever appear to turn them off — and `name`
 * defaults to the id so a fresh row renders with a label instead of blank.
 */
export function createEmptyFormProperty(id: string): FormProperty {
  return {
    attributes: {},
    datePattern: null,
    defaultExpression: null,
    expression: null,
    extensionElements: {},
    formValues: [],
    id,
    name: id,
    readable: true,
    required: false,
    type: 'string',
    variable: null,
    writeable: true,
    xmlColumnNumber: 0,
    xmlRowNumber: 0,
  };
}

function moveKeyedEntry<T>(map: Record<string, T>, from: string, to: string) {
  const entry = map[from];
  if (entry === undefined) return;
  map[to] = entry;
  delete map[from];
}

function rewireFlowElementReferences(elements: Draft<FlowElementEnum>[], from: string, to: string) {
  for (const element of elements) {
    if (element.elementType === 'sequenceFlow') {
      if (element.sourceRef === from) element.sourceRef = to;
      if (element.targetRef === from) element.targetRef = to;
    }
    if (element.elementType === 'boundaryEvent' && element.attachedToRefId === from) {
      element.attachedToRefId = to;
    }
    if ('defaultFlow' in element && element.defaultFlow === from) {
      element.defaultFlow = to;
    }
    switch (element.elementType) {
      case 'subProcess':
      case 'transaction':
      case 'eventSubProcess':
      case 'adhocSubProcess':
        rewireFlowElementReferences(element.flowElements ?? [], from, to);
        rewireArtifactReferences(element.artifacts ?? [], from, to);
        break;
      default:
        break;
    }
  }
}

function rewireArtifactReferences(artifacts: Draft<ArtifactEnum>[], from: string, to: string) {
  for (const artifact of artifacts) {
    if (artifact.artifactType !== 'association') continue;
    if (artifact.sourceRef === from) artifact.sourceRef = to;
    if (artifact.targetRef === from) artifact.targetRef = to;
  }
}
