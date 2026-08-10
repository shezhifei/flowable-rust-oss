import type { Draft } from 'immer';

import type {
  BpmnEditorDocument,
  FlowElementEnum,
  GraphicInfo,
} from '../generated/editor-protocol';
import type { ModelerCommand } from './commands';
import {
  validateSequenceFlowConnection,
  type SequenceFlowRejectionReason,
} from './connectionRules';
import { routeManhattan } from './manhattanRouter';
import { locateCanonicalElement, normalizeModelInvariants } from './modelInvariants';

export type ConnectionRejectionReason =
  | SequenceFlowRejectionReason
  | 'duplicate-id'
  | 'missing-di'
  | 'missing-endpoint'
  | 'source-or-target-is-not-flow-element';

export type ConnectionEligibility =
  | {
      valid: true;
      waypoints: GraphicInfo[];
    }
  | {
      valid: false;
      reason: ConnectionRejectionReason;
    };

export function sequenceFlowEligibility(
  document: Draft<BpmnEditorDocument>,
  sourceId: string,
  targetId: string,
): ConnectionEligibility {
  const source = locateCanonicalElement(document, sourceId);
  const target = locateCanonicalElement(document, targetId);
  if (!source || !target) return { valid: false, reason: 'missing-endpoint' };
  if (source.kind !== 'flowElement' || target.kind !== 'flowElement') {
    return { valid: false, reason: 'source-or-target-is-not-flow-element' };
  }

  const semantic = validateSequenceFlowConnection({
    sameSemanticOwner: source.owner === target.owner,
    source: source.element.elementType,
    target: target.element.elementType,
  });
  if (!semantic.valid) return semantic;

  const sourceBounds = document.model.locationMap[sourceId];
  const targetBounds = document.model.locationMap[targetId];
  if (!sourceBounds || !targetBounds) return { valid: false, reason: 'missing-di' };
  return {
    valid: true,
    waypoints: routeManhattan(sourceBounds, targetBounds).map(graphicPoint),
  };
}

export function createSequenceFlowCommand(
  flowId: string,
  sourceId: string,
  targetId: string,
): ModelerCommand {
  return {
    label: `Connect ${sourceId} to ${targetId}`,
    apply(document) {
      if (hasElementId(document, flowId)) return;
      const eligibility = sequenceFlowEligibility(document, sourceId, targetId);
      if (!eligibility.valid) return;
      const source = locateCanonicalElement(document, sourceId);
      if (!source || source.kind !== 'flowElement') return;

      const flow = sequenceFlow(flowId, sourceId, targetId, eligibility.waypoints);
      source.owner.flowElements ??= [];
      source.owner.flowElements.push(flow);
      document.model.flowLocationMap[flowId] = structuredClone(eligibility.waypoints);
      document.model.edgeMap[flowId] = {
        id: `BPMNEdge_${flowId}`,
        waypoints: structuredClone(eligibility.waypoints),
      };
      normalizeModelInvariants(document);
    },
  };
}

export function reconnectSequenceFlowCommand(
  flowId: string,
  sourceId: string,
  targetId: string,
): ModelerCommand {
  return {
    label: `Reconnect ${flowId}`,
    apply(document) {
      const located = locateCanonicalElement(document, flowId);
      if (
        !located ||
        located.kind !== 'flowElement' ||
        located.element.elementType !== 'sequenceFlow'
      ) {
        return;
      }
      const eligibility = sequenceFlowEligibility(document, sourceId, targetId);
      if (!eligibility.valid) return;
      const source = locateCanonicalElement(document, sourceId);
      if (!source || source.kind !== 'flowElement' || source.owner !== located.owner) return;

      located.element.sourceRef = sourceId;
      located.element.targetRef = targetId;
      located.element.waypoints = structuredClone(eligibility.waypoints);
      document.model.flowLocationMap[flowId] = structuredClone(eligibility.waypoints);
      document.model.edgeMap[flowId] = {
        ...(document.model.edgeMap[flowId] ?? { id: `BPMNEdge_${flowId}`, waypoints: [] }),
        waypoints: structuredClone(eligibility.waypoints),
      };
      normalizeModelInvariants(document);
    },
  };
}

export function setSequenceFlowConditionCommand(
  flowId: string,
  conditionExpression: string | null,
  conditionLanguage: string | null = null,
): ModelerCommand {
  return {
    label: `Set condition on ${flowId}`,
    apply(document) {
      const located = locateCanonicalElement(document, flowId);
      if (
        !located ||
        located.kind !== 'flowElement' ||
        located.element.elementType !== 'sequenceFlow'
      ) {
        return;
      }
      located.element.conditionExpression = conditionExpression;
      located.element.conditionLanguage = conditionLanguage;
      normalizeModelInvariants(document);
    },
  };
}

export function nextSequenceFlowId(document: BpmnEditorDocument, prefix = 'modeler-flow') {
  let suffix = 1;
  while (hasElementId(document, `${prefix}-${suffix}`)) suffix += 1;
  return `${prefix}-${suffix}`;
}

function hasElementId(document: Draft<BpmnEditorDocument>, id: string) {
  return document.model.processes.some((process) => Boolean(process.flowElementMap?.[id]));
}

function sequenceFlow(
  id: string,
  sourceRef: string,
  targetRef: string,
  waypoints: GraphicInfo[],
): Draft<FlowElementEnum> {
  return {
    elementType: 'sequenceFlow',
    id,
    xmlRowNumber: 0,
    xmlColumnNumber: 0,
    extensionElements: {},
    attributes: {},
    name: null,
    documentation: null,
    executionListeners: [],
    conditionExpression: null,
    conditionLanguage: null,
    skipExpression: null,
    sourceRef,
    targetRef,
    waypoints: structuredClone(waypoints),
  };
}

function graphicPoint(point: { x: number; y: number }): GraphicInfo {
  return {
    x: point.x,
    y: point.y,
    width: 0,
    height: 0,
    rotation: 0,
    expanded: false,
    xmlRowNumber: 0,
    xmlColumnNumber: 0,
  };
}
