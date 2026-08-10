import type { ArtifactEnum, FlowElementEnum } from '../generated/editor-protocol';

export type SequenceFlowEndpointKind =
  | FlowElementEnum['elementType']
  | ArtifactEnum['artifactType']
  | 'dataStore'
  | 'lane'
  | 'messageFlow'
  | 'pool';

export type SequenceFlowRejectionReason =
  | 'different-semantic-owner'
  | 'end-event-has-no-outgoing'
  | 'event-subprocess-has-no-sequence-flows'
  | 'source-is-not-a-flow-node'
  | 'start-event-has-no-incoming'
  | 'boundary-event-has-no-incoming'
  | 'target-is-not-a-flow-node';

export type SequenceFlowConnectionResult =
  { valid: true } | { valid: false; reason: SequenceFlowRejectionReason };

export interface SequenceFlowConnection {
  sameSemanticOwner: boolean;
  source: SequenceFlowEndpointKind;
  target: SequenceFlowEndpointKind;
}

const flowNodeKinds: ReadonlySet<SequenceFlowEndpointKind> = new Set([
  'task',
  'userTask',
  'serviceTask',
  'caseServiceTask',
  'sendTask',
  'scriptTask',
  'manualTask',
  'receiveTask',
  'businessRuleTask',
  'startEvent',
  'endEvent',
  'exclusiveGateway',
  'parallelGateway',
  'inclusiveGateway',
  'eventBasedGateway',
  'complexGateway',
  'intermediateCatchEvent',
  'intermediateThrowEvent',
  'subProcess',
  'transaction',
  'eventSubProcess',
  'adhocSubProcess',
  'callActivity',
  'boundaryEvent',
]);

/** Validates only sequence-flow endpoint semantics; ownership is resolved by the caller. */
export function validateSequenceFlowConnection(
  connection: SequenceFlowConnection,
): SequenceFlowConnectionResult {
  const { sameSemanticOwner, source, target } = connection;

  if (!sameSemanticOwner) return reject('different-semantic-owner');
  if (!flowNodeKinds.has(source)) return reject('source-is-not-a-flow-node');
  if (!flowNodeKinds.has(target)) return reject('target-is-not-a-flow-node');
  if (source === 'eventSubProcess' || target === 'eventSubProcess') {
    return reject('event-subprocess-has-no-sequence-flows');
  }
  if (source === 'endEvent') return reject('end-event-has-no-outgoing');
  if (target === 'startEvent') return reject('start-event-has-no-incoming');
  if (target === 'boundaryEvent') return reject('boundary-event-has-no-incoming');
  return { valid: true };
}

export function canConnectSequenceFlow(connection: SequenceFlowConnection): boolean {
  return validateSequenceFlowConnection(connection).valid;
}

function reject(reason: SequenceFlowRejectionReason): SequenceFlowConnectionResult {
  return { valid: false, reason };
}
