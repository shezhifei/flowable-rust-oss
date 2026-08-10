import { beforeEach, describe, expect, it } from 'vitest';

import type { FlowElementEnum } from '../generated/editor-protocol';
import { createPaletteElement } from './elementFactory';
import { locateCanonicalElement } from './modelInvariants';
import { useModelerStore } from './modelerStore';
import {
  replaceTaskTypeCommand,
  TaskReplacementError,
  type TaskFamilyType,
} from './replacementCommands';
import { sampleDocument } from './sampleDocument';

const replacementCycle = [
  'task',
  'serviceTask',
  'scriptTask',
  'manualTask',
  'receiveTask',
  'sendTask',
  'businessRuleTask',
  'userTask',
] as const satisfies readonly TaskFamilyType[];

type TaskFamilyElement = Extract<FlowElementEnum, { elementType: TaskFamilyType }>;

describe('task-family replacement commands', () => {
  beforeEach(() => {
    useModelerStore.getState().setDocument(structuredClone(sampleDocument));
  });

  it('cycles through all eight task types as reversible single-history commands', () => {
    for (const targetType of replacementCycle) {
      useModelerStore.getState().execute(replaceTaskTypeCommand('review', targetType));
      expect(canonicalReview().elementType).toBe(targetType);
    }

    expect(useModelerStore.getState().undoStack).toHaveLength(replacementCycle.length);
    expect(canonicalReview()).toMatchObject({ elementType: 'userTask' });

    for (let index = 0; index < replacementCycle.length; index += 1) {
      useModelerStore.getState().undo();
    }
    expect(canonicalReview()).toMatchObject({
      elementType: 'userTask',
      assignee: null,
      formKey: 'leaveRequest',
      priority: '50',
    });

    for (let index = 0; index < replacementCycle.length; index += 1) {
      useModelerStore.getState().redo();
    }
    expect(canonicalReview()).toMatchObject({
      elementType: 'userTask',
      assignee: null,
      formKey: null,
      priority: null,
    });
  });

  it('preserves the complete shared activity contract and derived relationships', () => {
    const document = structuredClone(sampleDocument);
    const review = mutableReview(document.model.processes[0]?.flowElements);
    review.attributes = { 'flowable:test': [{ name: 'test', value: 'preserved' }] };
    review.extensionElements = {
      audit: [
        {
          attributes: {},
          elementText: 'preserved',
          extensionElements: {},
          name: 'audit',
          xmlColumnNumber: 8,
          xmlRowNumber: 7,
        },
      ],
    };
    review.documentation = 'Shared documentation';
    review.executionListeners = [
      {
        attributes: {},
        event: 'start',
        extensionElements: {},
        implementation: '${auditListener}',
        implementationType: 'delegateExpression',
        xmlColumnNumber: 4,
        xmlRowNumber: 3,
      },
    ];
    review.asynchronous = true;
    review.asynchronousLeave = true;
    review.asynchronousLeaveExclusive = true;
    review.asynchronousLeaveNotExclusive = true;
    review.exclusive = false;
    review.notExclusive = true;
    review.failedJobRetryTimeCycleValue = 'R5/PT10S';
    review.isForCompensation = true;
    review.forCompensation = true;
    review.loopCharacteristics = {
      attributes: {},
      collectionString: '${reviewers}',
      elementVariable: 'reviewer',
      extensionElements: {},
      noWaitStatesAsyncLeave: true,
      sequential: true,
      xmlColumnNumber: 6,
      xmlRowNumber: 5,
    };
    review.fieldExtensions = [
      {
        attributes: {},
        extensionElements: {},
        fieldName: 'sharedField',
        stringValue: 'sharedValue',
        xmlColumnNumber: 10,
        xmlRowNumber: 9,
      },
    ];
    const originalBounds = structuredClone(document.model.locationMap.review);
    const originalLaneId = document.model.processes[0]?.lanes?.find((lane) =>
      lane.flowReferences.includes('review'),
    )?.id;
    useModelerStore.getState().setDocument(document);

    useModelerStore.getState().execute(replaceTaskTypeCommand('review', 'businessRuleTask'));

    const state = useModelerStore.getState();
    const replaced = canonicalReview();
    expect(replaced).toMatchObject({
      elementType: 'businessRuleTask',
      asynchronous: true,
      asynchronousLeave: true,
      asynchronousLeaveExclusive: true,
      asynchronousLeaveNotExclusive: true,
      documentation: 'Shared documentation',
      exclusive: false,
      failedJobRetryTimeCycleValue: 'R5/PT10S',
      forCompensation: true,
      id: 'review',
      isForCompensation: true,
      name: 'Review request',
      notExclusive: true,
      xmlColumnNumber: review.xmlColumnNumber,
      xmlRowNumber: review.xmlRowNumber,
    });
    expect(replaced.attributes).toEqual(review.attributes);
    expect(replaced.extensionElements).toEqual(review.extensionElements);
    expect(replaced.executionListeners).toEqual(review.executionListeners);
    expect(replaced.loopCharacteristics).toEqual(review.loopCharacteristics);
    expect(replaced.fieldExtensions).toEqual(review.fieldExtensions);
    expect(replaced.incomingFlows.map((flow) => flow.id)).toEqual(['requestFlow']);
    expect(replaced.outgoingFlows.map((flow) => flow.id)).toEqual(['decisionFlow']);
    expect(replaced.boundaryEvents?.map((event) => event.id)).toEqual(['reviewTimer']);
    expect(state.document.model.locationMap.review).toEqual(originalBounds);
    expect(
      state.document.model.processes[0]?.lanes?.find((lane) =>
        lane.flowReferences.includes('review'),
      )?.id,
    ).toBe(originalLaneId);
    expect(state.document.model.processes[0]?.flowElementMap?.review).toMatchObject({
      elementType: 'businessRuleTask',
    });
    expect(state.document.model.mainProcess?.flowElementMap?.review).toMatchObject({
      elementType: 'businessRuleTask',
    });
  });

  it('uses target defaults and never leaks source subtype fields', () => {
    useModelerStore.getState().execute(replaceTaskTypeCommand('review', 'serviceTask'));
    let replaced = canonicalReview();
    expect(replaced).toMatchObject({
      elementType: 'serviceTask',
      doNotIncludeVariables: false,
      eventInParameters: [],
      eventOutParameters: [],
      extended: false,
      implementation: null,
      implementationType: null,
      inParameters: [],
      outParameters: [],
      sendSynchronously: false,
      storeResultVariableAsTransient: false,
      triggerable: false,
      useLocalScopeForResultVariable: false,
    });
    expect(replaced).not.toHaveProperty('assignee');
    expect(replaced).not.toHaveProperty('candidateGroups');
    expect(replaced).not.toHaveProperty('formProperties');
    expect(replaced).not.toHaveProperty('taskListeners');

    useModelerStore.getState().execute(replaceTaskTypeCommand('review', 'scriptTask'));
    replaced = canonicalReview();
    expect(replaced).toMatchObject({
      elementType: 'scriptTask',
      autoStoreVariables: false,
      doNotIncludeVariables: false,
      inParameters: [],
      outParameters: [],
      resultVariable: null,
      script: null,
      scriptFormat: null,
    });
    expect(replaced).not.toHaveProperty('implementation');
    expect(replaced).not.toHaveProperty('triggerable');

    useModelerStore.getState().execute(replaceTaskTypeCommand('review', 'businessRuleTask'));
    replaced = canonicalReview();
    expect(replaced).toMatchObject({
      elementType: 'businessRuleTask',
      className: null,
      decisionRef: null,
      exclude: false,
      inputVariables: [],
      resultVariableName: null,
      ruleNames: [],
    });
    expect(replaced).not.toHaveProperty('script');
    expect(replaced).not.toHaveProperty('scriptFormat');
  });

  it('replaces a nested task in the same canonical owner and rebuilds all maps', () => {
    const document = structuredClone(sampleDocument);
    const nested = createPaletteElement('subprocess', 'nested-subprocess');
    if (nested.elementType !== 'subProcess') throw new Error('expected subprocess fixture');
    const innerReview = structuredClone(mutableReview(document.model.processes[0]?.flowElements));
    innerReview.id = 'inner-review';
    innerReview.incomingFlows = [];
    innerReview.outgoingFlows = [];
    innerReview.boundaryEvents = [];
    nested.flowElements = [innerReview];
    nested.flowElementMap = { 'inner-review': innerReview };
    document.model.processes[0]?.flowElements?.push(nested);
    useModelerStore.getState().setDocument(document);

    useModelerStore.getState().execute(replaceTaskTypeCommand('inner-review', 'manualTask'));

    const state = useModelerStore.getState();
    expect(locateCanonicalElement(state.document, 'inner-review')).toMatchObject({
      kind: 'flowElement',
      ownerId: 'nested-subprocess',
      element: { elementType: 'manualTask' },
    });
    const nestedAfter = locateCanonicalElement(state.document, 'nested-subprocess');
    expect(nestedAfter).toMatchObject({
      element: {
        flowElements: [{ elementType: 'manualTask', id: 'inner-review' }],
        flowElementMap: { 'inner-review': { elementType: 'manualTask' } },
      },
    });
    expect(state.document.model.processes[0]?.flowElementMap?.['inner-review']).toMatchObject({
      elementType: 'manualTask',
    });
    expect(state.document.model.mainProcess?.flowElementMap?.['inner-review']).toMatchObject({
      elementType: 'manualTask',
    });
  });

  it('rejects missing and non-task elements with typed errors and no history', () => {
    expect(() =>
      useModelerStore.getState().execute(replaceTaskTypeCommand('missing', 'serviceTask')),
    ).toThrowError(
      expect.objectContaining<Partial<TaskReplacementError>>({
        code: 'element-not-found',
        targetId: 'missing',
      }),
    );
    expect(() =>
      useModelerStore.getState().execute(replaceTaskTypeCommand('start', 'serviceTask')),
    ).toThrowError(
      expect.objectContaining<Partial<TaskReplacementError>>({
        actualElementType: 'startEvent',
        code: 'not-task-family-element',
        targetId: 'start',
      }),
    );
    expect(useModelerStore.getState().undoStack).toHaveLength(0);
    expect(canonicalElement('start')).toMatchObject({ elementType: 'startEvent' });
  });
});

function canonicalReview(): TaskFamilyElement {
  const review = canonicalElement('review');
  if (!isTaskFamilyElement(review)) {
    throw new Error('expected review task-family element');
  }
  return review;
}

function isTaskFamilyElement(element: FlowElementEnum): element is TaskFamilyElement {
  return replacementCycle.some((elementType) => elementType === element.elementType);
}

function canonicalElement(elementId: string): FlowElementEnum {
  const located = locateCanonicalElement(useModelerStore.getState().document, elementId);
  if (!located || located.kind !== 'flowElement') {
    throw new Error(`expected canonical flow element ${elementId}`);
  }
  return located.element;
}

function mutableReview(
  elements: FlowElementEnum[] | undefined,
): Extract<FlowElementEnum, { elementType: 'userTask' }> {
  const review = elements?.find((element) => element.id === 'review');
  if (!review || review.elementType !== 'userTask') throw new Error('expected review user task');
  return review;
}
