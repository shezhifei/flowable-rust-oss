import { describe, expect, it } from 'vitest';

import { canConnectSequenceFlow, validateSequenceFlowConnection } from './connectionRules';

describe('sequence-flow connection rules', () => {
  it('allows ordinary flow nodes owned by the same semantic container', () => {
    expect(
      canConnectSequenceFlow({ source: 'startEvent', target: 'userTask', sameSemanticOwner: true }),
    ).toBe(true);
    expect(
      canConnectSequenceFlow({
        source: 'boundaryEvent',
        target: 'exclusiveGateway',
        sameSemanticOwner: true,
      }),
    ).toBe(true);
  });

  it('rejects connections that cross process or subprocess ownership', () => {
    expect(
      validateSequenceFlowConnection({
        source: 'userTask',
        target: 'serviceTask',
        sameSemanticOwner: false,
      }),
    ).toEqual({ valid: false, reason: 'different-semantic-owner' });
  });

  it('enforces event directionality', () => {
    expect(
      validateSequenceFlowConnection({
        source: 'endEvent',
        target: 'userTask',
        sameSemanticOwner: true,
      }),
    ).toEqual({ valid: false, reason: 'end-event-has-no-outgoing' });
    expect(
      validateSequenceFlowConnection({
        source: 'userTask',
        target: 'startEvent',
        sameSemanticOwner: true,
      }),
    ).toEqual({ valid: false, reason: 'start-event-has-no-incoming' });
    expect(
      validateSequenceFlowConnection({
        source: 'userTask',
        target: 'boundaryEvent',
        sameSemanticOwner: true,
      }),
    ).toEqual({ valid: false, reason: 'boundary-event-has-no-incoming' });
    expect(
      validateSequenceFlowConnection({
        source: 'eventSubProcess',
        target: 'endEvent',
        sameSemanticOwner: true,
      }),
    ).toEqual({ valid: false, reason: 'event-subprocess-has-no-sequence-flows' });
  });

  it.each(['valuedDataObject', 'dataStore', 'textAnnotation', 'group', 'sequenceFlow'] as const)(
    'rejects %s as a source endpoint',
    (source) => {
      expect(
        validateSequenceFlowConnection({ source, target: 'userTask', sameSemanticOwner: true }),
      ).toEqual({ valid: false, reason: 'source-is-not-a-flow-node' });
    },
  );

  it.each(['valuedDataObject', 'dataStore', 'association', 'pool', 'lane'] as const)(
    'rejects %s as a target endpoint',
    (target) => {
      expect(
        validateSequenceFlowConnection({ source: 'userTask', target, sameSemanticOwner: true }),
      ).toEqual({ valid: false, reason: 'target-is-not-a-flow-node' });
    },
  );
});
