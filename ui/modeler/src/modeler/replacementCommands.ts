import type { Draft } from 'immer';

import type { FlowElementEnum, Task } from '../generated/editor-protocol';
import type { ModelerCommand } from './commands';
import { locateCanonicalElement, normalizeModelInvariants } from './modelInvariants';

export const TASK_FAMILY_TYPES = [
  'task',
  'userTask',
  'serviceTask',
  'scriptTask',
  'manualTask',
  'receiveTask',
  'sendTask',
  'businessRuleTask',
] as const;

export type TaskFamilyType = (typeof TASK_FAMILY_TYPES)[number];

type TaskFamilyElement = Extract<FlowElementEnum, { elementType: TaskFamilyType }>;

export type TaskReplacementErrorCode = 'element-not-found' | 'not-task-family-element';

export class TaskReplacementError extends Error {
  readonly actualElementType: string | null;
  readonly code: TaskReplacementErrorCode;
  readonly targetId: string;

  constructor(
    code: TaskReplacementErrorCode,
    targetId: string,
    message: string,
    actualElementType: string | null = null,
  ) {
    super(message);
    this.name = 'TaskReplacementError';
    this.code = code;
    this.targetId = targetId;
    this.actualElementType = actualElementType;
  }
}

/**
 * Replaces one canonical task with another BPMN task-family type while retaining
 * only the BaseElement, FlowNode, and Activity contract shared by every member.
 */
export function replaceTaskTypeCommand(
  elementId: string,
  targetType: TaskFamilyType,
): ModelerCommand {
  return {
    label: `Replace ${elementId} with ${targetType}`,
    apply(document) {
      const located = locateCanonicalElement(document, elementId);
      if (!located) {
        throw new TaskReplacementError(
          'element-not-found',
          elementId,
          `Cannot replace missing element ${elementId}`,
        );
      }
      if (located.kind !== 'flowElement' || !isTaskFamilyElement(located.element)) {
        const actualElementType =
          located.kind === 'flowElement' ? located.element.elementType : 'valuedDataObject';
        throw new TaskReplacementError(
          'not-task-family-element',
          elementId,
          `Element ${elementId} (${actualElementType}) is not in the replaceable task family`,
          actualElementType,
        );
      }
      if (located.element.elementType === targetType) return;

      const elements = located.owner.flowElements;
      const index = elements?.findIndex((candidate) => candidate === located.element) ?? -1;
      if (!elements || index < 0) {
        throw new TaskReplacementError(
          'element-not-found',
          elementId,
          `Canonical owner no longer contains element ${elementId}`,
        );
      }

      elements[index] = createTaskFamilyElement(targetType, commonTaskFields(located.element));
      normalizeModelInvariants(document);
    },
  };
}

function isTaskFamilyElement(element: Draft<FlowElementEnum>): element is Draft<TaskFamilyElement> {
  return TASK_FAMILY_TYPES.some((elementType) => elementType === element.elementType);
}

function commonTaskFields(element: Draft<TaskFamilyElement>): Task {
  return {
    asynchronous: element.asynchronous,
    asynchronousLeave: element.asynchronousLeave,
    asynchronousLeaveExclusive: element.asynchronousLeaveExclusive,
    asynchronousLeaveNotExclusive: element.asynchronousLeaveNotExclusive,
    attributes: element.attributes,
    boundaryEvents: element.boundaryEvents,
    dataInputAssociations: element.dataInputAssociations,
    dataOutputAssociations: element.dataOutputAssociations,
    defaultFlow: element.defaultFlow,
    documentation: element.documentation,
    exclusive: element.exclusive,
    executionListeners: element.executionListeners,
    extensionElements: element.extensionElements,
    failedJobRetryTimeCycleValue: element.failedJobRetryTimeCycleValue,
    fieldExtensions: element.fieldExtensions,
    forCompensation: element.forCompensation,
    id: element.id,
    incomingFlows: element.incomingFlows,
    isForCompensation: element.isForCompensation,
    loopCharacteristics: element.loopCharacteristics,
    mapExceptions: element.mapExceptions,
    name: element.name,
    notExclusive: element.notExclusive,
    outgoingFlows: element.outgoingFlows,
    xmlColumnNumber: element.xmlColumnNumber,
    xmlRowNumber: element.xmlRowNumber,
  };
}

function createTaskFamilyElement(targetType: TaskFamilyType, common: Task): TaskFamilyElement {
  switch (targetType) {
    case 'task':
      return { ...common, elementType: 'task' };
    case 'userTask':
      return {
        ...common,
        assignee: null,
        businessCalendarName: null,
        candidateGroups: [],
        candidateUsers: [],
        category: null,
        dueDate: null,
        extended: false,
        extensionId: null,
        formKey: null,
        formProperties: [],
        owner: null,
        priority: null,
        sameDeployment: true,
        skipExpression: null,
        taskCompleterVariableName: null,
        taskIdVariableName: null,
        taskListeners: [],
        validateFormFields: null,
        elementType: 'userTask',
      };
    case 'serviceTask':
      return {
        ...common,
        doNotIncludeVariables: false,
        eventInParameters: [],
        eventOutParameters: [],
        eventType: null,
        extended: false,
        extensionId: null,
        formKey: null,
        httpRequestHandler: null,
        httpResponseHandler: null,
        implementation: null,
        implementationType: null,
        inParameters: [],
        outParameters: [],
        parallelInSameTransaction: null,
        resultVariableName: null,
        sendSynchronously: false,
        skipExpression: null,
        storeResultVariableAsTransient: false,
        topic: null,
        triggerEventType: null,
        triggerable: false,
        type: null,
        useLocalScopeForResultVariable: false,
        validateFormFields: null,
        elementType: 'serviceTask',
      };
    case 'scriptTask':
      return {
        ...common,
        autoStoreVariables: false,
        doNotIncludeVariables: false,
        inParameters: [],
        outParameters: [],
        resultVariable: null,
        script: null,
        scriptFormat: null,
        skipExpression: null,
        elementType: 'scriptTask',
      };
    case 'manualTask':
      return { ...common, elementType: 'manualTask' };
    case 'receiveTask':
      return {
        ...common,
        messageRef: null,
        skipExpression: null,
        elementType: 'receiveTask',
      };
    case 'sendTask':
      return {
        ...common,
        doNotIncludeVariables: false,
        eventInParameters: [],
        eventOutParameters: [],
        eventType: null,
        extended: false,
        extensionId: null,
        formKey: null,
        httpRequestHandler: null,
        httpResponseHandler: null,
        implementation: null,
        implementationType: null,
        inParameters: [],
        operationRef: null,
        outParameters: [],
        parallelInSameTransaction: null,
        resultVariableName: null,
        sendSynchronously: false,
        skipExpression: null,
        storeResultVariableAsTransient: false,
        topic: null,
        triggerEventType: null,
        triggerable: false,
        type: null,
        useLocalScopeForResultVariable: false,
        validateFormFields: null,
        elementType: 'sendTask',
      };
    case 'businessRuleTask':
      return {
        ...common,
        className: null,
        decisionRef: null,
        exclude: false,
        inputVariables: [],
        resultVariableName: null,
        ruleNames: [],
        elementType: 'businessRuleTask',
      };
  }
}
