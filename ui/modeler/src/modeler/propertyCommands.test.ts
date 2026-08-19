import { beforeEach, describe, expect, it } from 'vitest';

import { useModelerStore } from './modelerStore';
import {
  createEmptyFieldExtension,
  createEmptyIOParameter,
  createEmptyListener,
  createEmptyLoopCharacteristics,
  createEmptyMessage,
  createEmptySignal,
  renameElementIdCommand,
  updateElementPropertiesCommand,
  updateEventDefinitionRefCommand,
  updateModelMessagesCommand,
  updateModelSignalsCommand,
  updateProcessPropertiesCommand,
  updateTimerDefinitionCommand,
} from './propertyCommands';
import { sampleDocument } from './sampleDocument';

function resetStore() {
  useModelerStore.getState().setDocument(structuredClone(sampleDocument));
}

function state() {
  return useModelerStore.getState();
}

function flowElement(id: string) {
  const element = state().document.model.processes[0]?.flowElementMap?.[id];
  if (!element) throw new Error(`${id} is missing`);
  return element;
}

describe('element property updates', () => {
  beforeEach(resetStore);

  it('writes user task properties through the command stack with undo/redo', () => {
    state().execute(
      updateElementPropertiesCommand(
        'review',
        { assignee: 'kermit', candidateGroups: ['management', 'hr'], priority: '75' },
        'Edit assignee',
      ),
    );

    expect(state().undoStack.at(-1)?.label).toBe('Edit assignee');
    expect(flowElement('review')).toMatchObject({
      assignee: 'kermit',
      candidateGroups: ['management', 'hr'],
      priority: '75',
    });

    state().undo();
    expect(flowElement('review')).toMatchObject({
      assignee: null,
      candidateGroups: ['managers'],
      priority: '50',
    });

    state().redo();
    expect(flowElement('review')).toMatchObject({ assignee: 'kermit', priority: '75' });
  });

  it('clears nullable values and list values', () => {
    state().execute(
      updateElementPropertiesCommand('review', { formKey: null, candidateGroups: [] }),
    );
    expect(flowElement('review')).toMatchObject({ formKey: null, candidateGroups: [] });
    state().undo();
    expect(flowElement('review')).toMatchObject({ formKey: 'leaveRequest' });
  });

  it('writes service task implementation fields and sequence flow conditions', () => {
    state().execute(
      updateElementPropertiesCommand('notify', {
        implementationType: 'class',
        implementation: 'org.flowable.NotifyDelegate',
        resultVariableName: 'notifyResult',
      }),
    );
    expect(flowElement('notify')).toMatchObject({
      implementationType: 'class',
      implementation: 'org.flowable.NotifyDelegate',
      resultVariableName: 'notifyResult',
    });

    state().execute(
      updateElementPropertiesCommand('approvedFlow', {
        conditionExpression: '${approved == true}',
      }),
    );
    expect(flowElement('approvedFlow')).toMatchObject({
      conditionExpression: '${approved == true}',
    });

    state().undo();
    state().undo();
    expect(flowElement('notify')).toMatchObject({ implementationType: 'delegateExpression' });
    expect(flowElement('approvedFlow')).toMatchObject({ conditionExpression: '${approved}' });
  });

  it('rejects writes to unknown elements without touching the document', () => {
    expect(() =>
      state().execute(updateElementPropertiesCommand('ghost', { name: 'Ghost' })),
    ).toThrowError(expect.objectContaining({ code: 'missing-element' }));
    expect(state().undoStack).toHaveLength(0);
  });
});

describe('element id rename', () => {
  beforeEach(resetStore);

  it('rewires DI maps, flow endpoints, attachments, and lane memberships', () => {
    state().execute(renameElementIdCommand('review', 'audit'));

    const { model } = state().document;
    expect(model.locationMap.audit).toBeDefined();
    expect(model.locationMap.review).toBeUndefined();
    expect(flowElement('requestFlow')).toMatchObject({ targetRef: 'audit' });
    expect(flowElement('decisionFlow')).toMatchObject({ sourceRef: 'audit' });
    expect(flowElement('reviewTimer')).toMatchObject({ attachedToRefId: 'audit' });
    expect(model.processes[0]?.lanes?.[0]?.flowReferences).toContain('audit');
    expect(model.processes[0]?.lanes?.[0]?.flowReferences).not.toContain('review');
    expect(model.processes[0]?.flowElementMap?.review).toBeUndefined();

    state().undo();
    expect(flowElement('requestFlow')).toMatchObject({ targetRef: 'review' });
    expect(state().document.model.locationMap.review).toBeDefined();
    expect(state().document.model.processes[0]?.lanes?.[0]?.flowReferences).toContain('review');

    state().redo();
    expect(state().document.model.locationMap.audit).toBeDefined();
  });

  it('moves sequence flow waypoints and label bounds to the new id', () => {
    state().execute(renameElementIdCommand('approvedFlow', 'confirmedFlow'));

    const { model } = state().document;
    expect(model.flowLocationMap.confirmedFlow).toHaveLength(4);
    expect(model.flowLocationMap.approvedFlow).toBeUndefined();
    expect(model.labelLocationMap.confirmedFlow).toBeDefined();
    expect(model.labelLocationMap.approvedFlow).toBeUndefined();
  });

  it('rewires association endpoints through artifacts', () => {
    state().execute(renameElementIdCommand('decision', 'verdict'));
    const association = state().document.model.processes[0]?.artifactMap?.approvalLink;
    expect(association).toMatchObject({ targetRef: 'verdict' });
  });

  it('refuses duplicate, blank, and whitespace ids', () => {
    expect(() => state().execute(renameElementIdCommand('review', 'notify'))).toThrowError(
      expect.objectContaining({ code: 'duplicate-element-id' }),
    );
    expect(() => state().execute(renameElementIdCommand('review', 'approvalNote'))).toThrowError(
      expect.objectContaining({ code: 'duplicate-element-id' }),
    );
    expect(() => state().execute(renameElementIdCommand('review', '  '))).toThrowError(
      expect.objectContaining({ code: 'invalid-element-id' }),
    );
    expect(() => state().execute(renameElementIdCommand('review', 'has space'))).toThrowError(
      expect.objectContaining({ code: 'invalid-element-id' }),
    );
    expect(state().undoStack).toHaveLength(0);
    expect(state().document.model.locationMap.review).toBeDefined();
  });

  it('ignores a rename to the same id', () => {
    state().execute(renameElementIdCommand('review', 'review'));
    expect(state().undoStack).toHaveLength(0);
  });
});

describe('process property updates', () => {
  beforeEach(resetStore);

  it('edits the main process name and documentation with undo', () => {
    state().execute(
      updateProcessPropertiesCommand({ name: 'Leave approval v2', documentation: null }),
    );
    expect(state().document.model.processes[0]).toMatchObject({
      name: 'Leave approval v2',
      documentation: null,
    });

    state().undo();
    expect(state().document.model.processes[0]).toMatchObject({
      name: 'Leave approval',
      documentation: 'A representative Flowable process rendered from the editor protocol.',
    });
  });

  it('renames the process id and updates the pool process reference', () => {
    state().execute(updateProcessPropertiesCommand({ id: 'leaveApprovalProcess' }));
    expect(state().document.model.processes[0]?.id).toBe('leaveApprovalProcess');
    expect(state().document.model.pools[0]?.processRef).toBe('leaveApprovalProcess');

    state().undo();
    expect(state().document.model.processes[0]?.id).toBe('leaveProcess');
    expect(state().document.model.pools[0]?.processRef).toBe('leaveProcess');
  });

  it('rejects a process id that collides with any diagram id', () => {
    expect(() => state().execute(updateProcessPropertiesCommand({ id: 'review' }))).toThrowError(
      expect.objectContaining({ code: 'duplicate-element-id' }),
    );
    expect(state().undoStack).toHaveLength(0);
  });
});

describe('phase-2 advanced property commands', () => {
  beforeEach(resetStore);

  it('writes multi-instance characteristics through the command stack', () => {
    const loop = createEmptyLoopCharacteristics(true);
    loop.collectionString = '${assignees}';
    loop.elementVariable = 'assignee';
    loop.completionCondition = '${nrOfCompletedInstances == 1}';
    state().execute(
      updateElementPropertiesCommand(
        'review',
        { loopCharacteristics: loop },
        'Enable multi-instance',
      ),
    );

    expect(flowElement('review')).toMatchObject({
      loopCharacteristics: {
        sequential: true,
        collectionString: '${assignees}',
        elementVariable: 'assignee',
        completionCondition: '${nrOfCompletedInstances == 1}',
      },
    });
    state().undo();
    expect(flowElement('review')).toMatchObject({ loopCharacteristics: null });
  });

  it('writes task and execution listeners', () => {
    const taskListener = createEmptyListener('create');
    taskListener.implementationType = 'class';
    taskListener.implementation = 'org.flowable.TaskListener';
    state().execute(
      updateElementPropertiesCommand('review', { taskListeners: [taskListener] }, 'Add task listener'),
    );
    expect(flowElement('review')).toMatchObject({
      taskListeners: [
        {
          event: 'create',
          implementationType: 'class',
          implementation: 'org.flowable.TaskListener',
        },
      ],
    });

    const executionListener = createEmptyListener('start');
    executionListener.implementationType = 'expression';
    executionListener.implementation = '${logExecution}';
    state().execute(
      updateElementPropertiesCommand(
        'notify',
        { executionListeners: [executionListener] },
        'Add execution listener',
      ),
    );
    expect(flowElement('notify')).toMatchObject({
      executionListeners: [
        {
          event: 'start',
          implementationType: 'expression',
          implementation: '${logExecution}',
        },
      ],
    });
  });

  it('writes service task field injection entries', () => {
    const field = createEmptyFieldExtension();
    field.fieldName = 'endpoint';
    field.stringValue = 'https://example.test';
    state().execute(
      updateElementPropertiesCommand('notify', { fieldExtensions: [field] }, 'Add field'),
    );
    expect(flowElement('notify')).toMatchObject({
      fieldExtensions: [{ fieldName: 'endpoint', stringValue: 'https://example.test' }],
    });
    state().undo();
    expect(flowElement('notify')).toMatchObject({ fieldExtensions: [] });
  });

  it('writes call activity calledElement and in/out parameters', () => {
    // Seed a call activity into the sample document for this case.
    const document = structuredClone(sampleDocument);
    const process = document.model.processes[0]!;
    const callActivity = {
      elementType: 'callActivity' as const,
      id: 'childCall',
      name: 'Call child',
      documentation: null,
      xmlRowNumber: 0,
      xmlColumnNumber: 0,
      extensionElements: {},
      attributes: {},
      executionListeners: [],
      asynchronous: false,
      asynchronousLeave: false,
      notExclusive: false,
      asynchronousLeaveNotExclusive: false,
      exclusive: true,
      asynchronousLeaveExclusive: false,
      incomingFlows: [],
      outgoingFlows: [],
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
      calledElement: null,
      calledElementType: null,
      calledElementBinding: null,
      businessKey: null,
      inheritBusinessKey: false,
      inheritVariables: false,
      sameDeployment: true,
      fallbackToDefaultTenant: null,
      processInstanceName: null,
      processInstanceIdVariableName: null,
      completeAsync: false,
      useLocalScopeForOutParameters: false,
      inParameters: [],
      outParameters: [],
    };
    process.flowElements = [...(process.flowElements ?? []), callActivity];
    process.flowElementMap = {
      ...(process.flowElementMap ?? {}),
      childCall: callActivity,
    };
    state().setDocument(document);

    const inParameter = createEmptyIOParameter();
    inParameter.source = 'parentVar';
    inParameter.target = 'childVar';
    state().execute(
      updateElementPropertiesCommand(
        'childCall',
        {
          calledElement: 'childProcess',
          inParameters: [inParameter],
          outParameters: [],
        },
        'Edit call activity',
      ),
    );
    expect(flowElement('childCall')).toMatchObject({
      calledElement: 'childProcess',
      inParameters: [{ source: 'parentVar', target: 'childVar' }],
    });
  });

  it('manages document signal/message definitions and event refs', () => {
    state().execute(updateModelSignalsCommand([createEmptySignal('escalationSignal')]));
    expect(state().document.model.signals).toEqual([
      expect.objectContaining({ id: 'escalationSignal', name: 'escalationSignal' }),
    ]);

    state().execute(updateModelMessagesCommand([createEmptyMessage('startMessage')]));
    expect(state().document.model.messages).toEqual([
      expect.objectContaining({ id: 'startMessage', name: 'startMessage' }),
    ]);

    state().execute(
      updateEventDefinitionRefCommand('start', 'messageEventDefinition', 'startMessage'),
    );
    const start = flowElement('start');
    if (start.elementType !== 'startEvent') throw new Error('expected start event');
    expect(start.eventDefinitions).toEqual([
      expect.objectContaining({
        eventDefinitionType: 'messageEventDefinition',
        messageRef: 'startMessage',
      }),
    ]);

    state().undo();
    expect(flowElement('start')).toMatchObject({ eventDefinitions: [] });
  });

  it('keeps exactly one timer definition field set', () => {
    state().execute(updateTimerDefinitionCommand('reviewTimer', { timeCycle: 'R3/PT10M' }));
    const timer = flowElement('reviewTimer');
    if (timer.elementType !== 'boundaryEvent') throw new Error('expected boundary event');
    expect(timer.eventDefinitions).toEqual([
      expect.objectContaining({
        eventDefinitionType: 'timerEventDefinition',
        timeCycle: 'R3/PT10M',
        timeDuration: null,
        timeDate: null,
      }),
    ]);

    state().undo();
    expect(flowElement('reviewTimer')).toMatchObject({
      eventDefinitions: [expect.objectContaining({ timeDuration: 'PT48H', timeCycle: null })],
    });
  });

  it('writes calendarName and endDate without disturbing the timer kind', () => {
    state().execute(
      updateTimerDefinitionCommand('reviewTimer', {
        calendarName: 'businessCalendar',
        endDate: '2026-09-01T00:00:00Z',
      }),
    );
    const timer = flowElement('reviewTimer');
    if (timer.elementType !== 'boundaryEvent') throw new Error('expected boundary event');
    expect(timer.eventDefinitions).toEqual([
      expect.objectContaining({
        timeDuration: 'PT48H',
        calendarName: 'businessCalendar',
        endDate: '2026-09-01T00:00:00Z',
      }),
    ]);
  });

  it('creates a timer definition on an event that has none', () => {
    state().execute(updateTimerDefinitionCommand('start', { timeDate: '2026-12-24T09:00:00Z' }));
    const start = flowElement('start');
    if (start.elementType !== 'startEvent') throw new Error('expected start event');
    expect(start.eventDefinitions).toEqual([
      expect.objectContaining({
        eventDefinitionType: 'timerEventDefinition',
        id: 'start_timerEventDefinition',
        timeDate: '2026-12-24T09:00:00Z',
        timeDuration: null,
        timeCycle: null,
      }),
    ]);
  });

  it('clears the timer kind when the value is emptied', () => {
    state().execute(updateTimerDefinitionCommand('reviewTimer', { timeDuration: null }));
    const timer = flowElement('reviewTimer');
    if (timer.elementType !== 'boundaryEvent') throw new Error('expected boundary event');
    expect(timer.eventDefinitions).toEqual([
      expect.objectContaining({ timeDuration: null, timeDate: null, timeCycle: null }),
    ]);
  });

  it('refuses timer edits on elements that cannot carry event definitions', () => {
    expect(() => state().execute(updateTimerDefinitionCommand('review', { timeCycle: 'R/PT1H' })))
      .toThrow(/does not carry event definitions/);
    expect(() => state().execute(updateTimerDefinitionCommand('nope', { timeCycle: 'R/PT1H' })))
      .toThrow(/is not part of this document/);
  });
});
