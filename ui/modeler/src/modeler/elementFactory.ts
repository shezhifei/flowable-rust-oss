import type { FlowElementEnum } from '../generated/editor-protocol';

export type PaletteElementKind = 'event' | 'task' | 'gateway' | 'subprocess' | 'data';

const baseElement = (id: string) => ({
  id,
  xmlRowNumber: 0,
  xmlColumnNumber: 0,
  extensionElements: {},
  attributes: {},
});

const flowNode = (id: string, name: string) => ({
  ...baseElement(id),
  name,
  documentation: null,
  executionListeners: [],
  asynchronous: false,
  asynchronousLeave: false,
  notExclusive: false,
  asynchronousLeaveNotExclusive: false,
  exclusive: true,
  asynchronousLeaveExclusive: false,
  incomingFlows: [],
  outgoingFlows: [],
});

const activity = (id: string, name: string) => ({
  ...flowNode(id, name),
  failedJobRetryTimeCycleValue: null,
  defaultFlow: null,
  isForCompensation: false,
  forCompensation: false,
  loopCharacteristics: null,
  dataInputAssociations: [],
  dataOutputAssociations: [],
  mapExceptions: [],
  boundaryEvents: [],
  fieldExtensions: [],
});

export function createPaletteElement(kind: PaletteElementKind, id: string): FlowElementEnum {
  switch (kind) {
    case 'event':
      return {
        elementType: 'startEvent',
        ...flowNode(id, 'Start event'),
        eventDefinitions: [],
        initiator: null,
        formKey: null,
        sameDeployment: true,
        interrupting: true,
      };
    case 'task':
      return {
        elementType: 'userTask',
        ...activity(id, 'User task'),
        assignee: null,
        owner: null,
        priority: null,
        category: null,
        formKey: null,
        dueDate: null,
        businessCalendarName: null,
        candidateUsers: [],
        candidateGroups: [],
        formProperties: [],
        taskListeners: [],
        skipExpression: null,
        extended: false,
        extensionId: null,
        sameDeployment: true,
        validateFormFields: null,
        taskIdVariableName: null,
        taskCompleterVariableName: null,
      };
    case 'gateway':
      return {
        elementType: 'exclusiveGateway',
        ...flowNode(id, 'Gateway'),
        defaultFlow: null,
      };
    case 'subprocess':
      return {
        elementType: 'subProcess',
        ...activity(id, 'Subprocess'),
        flowElements: [],
        flowElementMap: {},
        artifacts: [],
        artifactMap: {},
        dataObjects: [],
        triggeredByEvent: false,
      };
    case 'data':
      return {
        elementType: 'valuedDataObject',
        ...baseElement(id),
        name: 'Data object',
        documentation: null,
        executionListeners: [],
        itemSubjectRef: {
          ...baseElement(`${id}Item`),
          structureRef: 'xsd:string',
          itemKind: null,
          isCollection: false,
        },
        type: 'string',
        dataObjectRef: null,
      };
  }
}

export function defaultElementSize(kind: PaletteElementKind) {
  switch (kind) {
    case 'event':
      return { width: 42, height: 42 };
    case 'gateway':
      return { width: 66, height: 66 };
    case 'subprocess':
      return { width: 260, height: 170 };
    case 'data':
      return { width: 46, height: 62 };
    case 'task':
      return { width: 156, height: 100 };
  }
}
