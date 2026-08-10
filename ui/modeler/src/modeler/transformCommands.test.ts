import { beforeEach, describe, expect, it } from 'vitest';

import { useModelerStore } from './modelerStore';
import { sampleDocument } from './sampleDocument';
import {
  addBendpointCommand,
  moveBendpointCommand,
  moveElementsCommand,
  removeBendpointCommand,
  resizeElementCommand,
  TransformCommandError,
} from './transformCommands';

describe('atomic multi-element movement', () => {
  beforeEach(resetStore);

  it('snaps one delta and moves a container, descendants, boundary DI, and routes once', () => {
    const originalDecisionFlow = structuredClone(requiredRoute('decisionFlow'));
    const originalRequestFlow = structuredClone(requiredRoute('requestFlow'));

    useModelerStore.getState().execute(moveElementsCommand(['approvalGroup'], 13, 17));

    let state = useModelerStore.getState();
    expect(state.document.model.locationMap.approvalGroup).toMatchObject({ x: 292, y: 132 });
    expect(state.document.model.locationMap.review).toMatchObject({ x: 314, y: 155 });
    expect(state.document.model.locationMap.decision).toMatchObject({ x: 564, y: 173 });
    expect(state.document.model.locationMap.reviewTimer).toMatchObject({ x: 420, y: 234 });
    expect(state.document.model.flowLocationMap.decisionFlow).toEqual(
      originalDecisionFlow.map((point) => ({ ...point, x: point.x + 10, y: point.y + 20 })),
    );
    expect(requiredRoute('requestFlow')[0]).toEqual(originalRequestFlow[0]);
    expect(requiredRoute('requestFlow').at(-1)).toMatchObject({ x: 314, y: 205 });
    expect(state.document.model.edgeMap.decisionFlow?.waypoints).toEqual(
      state.document.model.flowLocationMap.decisionFlow,
    );
    expect(state.undoStack).toHaveLength(1);

    useModelerStore.getState().undo();
    state = useModelerStore.getState();
    expect(state.document.model.locationMap.approvalGroup).toMatchObject({ x: 282, y: 112 });
    expect(state.document.model.flowLocationMap.decisionFlow).toEqual(originalDecisionFlow);

    useModelerStore.getState().redo();
    state = useModelerStore.getState();
    expect(state.document.model.locationMap.review).toMatchObject({ x: 314, y: 155 });
    expect(state.document.model.flowLocationMap.decisionFlow).toEqual(
      originalDecisionFlow.map((point) => ({ ...point, x: point.x + 10, y: point.y + 20 })),
    );
  });
});

describe('container resizing', () => {
  beforeEach(resetStore);

  it('snaps and clamps a supported container without scaling its children', () => {
    const reviewBefore = structuredClone(
      useModelerStore.getState().document.model.locationMap.review,
    );
    useModelerStore.getState().execute(resizeElementCommand('approvalGroup', 53, 73));

    expect(useModelerStore.getState().document.model.locationMap.approvalGroup).toMatchObject({
      x: 282,
      y: 112,
      width: 80,
      height: 70,
    });
    expect(useModelerStore.getState().document.model.locationMap.review).toEqual(reviewBefore);

    useModelerStore.getState().undo();
    expect(useModelerStore.getState().document.model.locationMap.approvalGroup).toMatchObject({
      width: 370,
      height: 158,
    });
    useModelerStore.getState().redo();
    expect(useModelerStore.getState().document.model.locationMap.approvalGroup).toMatchObject({
      width: 80,
      height: 70,
    });
  });

  it('rejects non-container targets with a typed error and no history entry', () => {
    expect(() =>
      useModelerStore.getState().execute(resizeElementCommand('review', 200, 120)),
    ).toThrowError(
      expect.objectContaining<Partial<TransformCommandError>>({
        code: 'unsupported-resize-target',
        targetId: 'review',
      }),
    );
    expect(useModelerStore.getState().undoStack).toHaveLength(0);
    expect(useModelerStore.getState().document.model.locationMap.review).toMatchObject({
      width: 156,
      height: 100,
    });
  });
});

describe('bendpoint commands', () => {
  beforeEach(resetStore);

  it('adds, moves, and removes snapped bendpoints while synchronizing edge DI', () => {
    useModelerStore.getState().execute(addBendpointCommand('approvedFlow', 0, { x: 641, y: 201 }));
    expect(requiredRoute('approvedFlow')[1]).toMatchObject({ x: 640, y: 200 });
    expect(useModelerStore.getState().document.model.edgeMap.approvedFlow?.waypoints).toEqual(
      requiredRoute('approvedFlow'),
    );

    useModelerStore.getState().execute(moveBendpointCommand('approvedFlow', 1, { x: 651, y: 211 }));
    expect(requiredRoute('approvedFlow')[1]).toMatchObject({ x: 650, y: 210 });

    useModelerStore.getState().execute(removeBendpointCommand('approvedFlow', 1));
    expect(requiredRoute('approvedFlow')).toHaveLength(4);
    expect(useModelerStore.getState().document.model.edgeMap.approvedFlow?.waypoints).toEqual(
      requiredRoute('approvedFlow'),
    );

    useModelerStore.getState().undo();
    expect(requiredRoute('approvedFlow')).toHaveLength(5);
    expect(requiredRoute('approvedFlow')[1]).toMatchObject({ x: 650, y: 210 });
    useModelerStore.getState().redo();
    expect(requiredRoute('approvedFlow')).toHaveLength(4);
  });

  it('rejects anchor movement and missing routes with typed errors', () => {
    expect(() =>
      useModelerStore.getState().execute(moveBendpointCommand('approvedFlow', 0, { x: 1, y: 1 })),
    ).toThrowError(expect.objectContaining({ code: 'invalid-bendpoint-index' }));
    expect(() =>
      useModelerStore.getState().execute(addBendpointCommand('missingFlow', 0, { x: 1, y: 1 })),
    ).toThrowError(expect.objectContaining({ code: 'missing-route' }));
    expect(useModelerStore.getState().undoStack).toHaveLength(0);
  });
});

function resetStore() {
  useModelerStore.getState().setDocument(structuredClone(sampleDocument));
}

function requiredRoute(flowId: string) {
  const route = useModelerStore.getState().document.model.flowLocationMap[flowId];
  if (!route) throw new Error(`${flowId} route is missing`);
  return route;
}
