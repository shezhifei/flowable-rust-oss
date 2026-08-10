import type { Draft } from 'immer';

import type { ArtifactEnum, FlowElementEnum } from '../generated/editor-protocol';
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
