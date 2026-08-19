import type { Draft } from 'immer';

import type {
  ArtifactEnum,
  EventDefinitionEnum,
  FieldExtension,
  FlowableListener,
  FlowElementEnum,
  IOParameter,
  Message,
  MultiInstanceLoopCharacteristics,
  Signal,
} from '../generated/editor-protocol';
import type { ModelerCommand } from './commands';
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
 * Applies already-validated property values to one flow element or data
 * object. Unknown element ids abort the command so nothing half-writes.
 */
export function updateElementPropertiesCommand(
  elementId: string,
  properties: Record<string, unknown>,
  label?: string,
): ModelerCommand {
  return {
    label: label ?? `Edit ${elementId} properties`,
    apply(document) {
      const located = locateCanonicalElement(document, elementId);
      if (!located) {
        throw new PropertyCommandError(
          'missing-element',
          elementId,
          `${elementId} is not part of this document`,
        );
      }
      Object.assign(located.element, properties);
      normalizeModelInvariants(document);
    },
  };
}

/**
 * Renames an element id and rewires every diagram-owned reference to it:
 * DI maps, sequence flow endpoints, boundary attachments, default flows,
 * lane memberships, message flows, and association endpoints.
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
      const located = locateCanonicalElement(document, elementId);
      if (!located) {
        throw new PropertyCommandError(
          'missing-element',
          elementId,
          `${elementId} is not part of this document`,
        );
      }
      located.element.id = trimmed;

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

/** Edits the main process (no-selection target). A pool processRef follows a process id rename. */
export function updateProcessPropertiesCommand(properties: ProcessPropertyUpdate): ModelerCommand {
  const summary = properties.id ?? 'process';
  return {
    label: `Edit ${summary} process properties`,
    apply(document) {
      const process = document.model.processes[0];
      if (!process) {
        throw new PropertyCommandError('missing-process', summary, 'the document has no process');
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

/**
 * Sets signalRef or messageRef on the first matching event definition of an
 * event element. Creates a definition entry when none exists yet so the panel
 * can seed a reference without a separate create step.
 */
export function updateEventDefinitionRefCommand(
  elementId: string,
  definitionType: 'signalEventDefinition' | 'messageEventDefinition',
  ref: string | null,
): ModelerCommand {
  const field = definitionType === 'signalEventDefinition' ? 'signalRef' : 'messageRef';
  return {
    label: `Edit ${field} on ${elementId}`,
    apply(document) {
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
      if (!existing) {
        definitions.push({
          eventDefinitionType: definitionType,
          id: `${elementId}_${definitionType}`,
          attributes: {},
          extensionElements: {},
          xmlColumnNumber: 0,
          xmlRowNumber: 0,
          [field]: ref,
        } as Draft<EventDefinitionEnum>);
      } else {
        Object.assign(existing, { [field]: ref });
      }
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
  return {
    label: `Edit timer on ${elementId}`,
    apply(document) {
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
        (candidate) => candidate.eventDefinitionType === 'timerEventDefinition',
      );

      const patch: Record<string, string | null> = {};
      if (kind) {
        for (const field of TIMER_KIND_FIELDS) {
          patch[field] = field === kind ? (fields[kind] ?? null) : null;
        }
      }
      if ('calendarName' in fields) patch.calendarName = fields.calendarName ?? null;
      if ('endDate' in fields) patch.endDate = fields.endDate ?? null;

      if (existing) {
        Object.assign(existing, patch);
      } else {
        definitions.push({
          eventDefinitionType: 'timerEventDefinition',
          id: `${elementId}_timerEventDefinition`,
          attributes: {},
          extensionElements: {},
          xmlColumnNumber: 0,
          xmlRowNumber: 0,
          timeDate: null,
          timeCycle: null,
          timeDuration: null,
          calendarName: null,
          endDate: null,
          ...patch,
        } as Draft<EventDefinitionEnum>);
      }
      normalizeModelInvariants(document);
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
