import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { PropertiesPanel } from './PropertiesPanel';
import { useModelerStore } from './modelerStore';
import { replaceTaskTypeCommand } from './replacementCommands';
import { sampleDocument } from './sampleDocument';

function renderPanel(selectedElementIds: string[] = []) {
  return renderToStaticMarkup(
    <PropertiesPanel
      panelState={{ document: structuredClone(sampleDocument), selectedElementIds }}
    />,
  );
}

/** Two participants, cloned from the sample so the literal cannot drift. */
function renderMultiPoolPanel() {
  const document = structuredClone(sampleDocument);
  const pool = document.model.pools[0];
  const process = document.model.processes[0];
  if (!pool || !process) throw new Error('the sample document should have a pool and a process');
  document.model.pools.push({ ...structuredClone(pool), id: 'vacationPool', name: 'Vacation' });
  return renderToStaticMarkup(
    <PropertiesPanel panelState={{ document, selectedElementIds: [] }} />,
  );
}

/**
 * The sample document has no business rule task, so this converts `review`
 * with the same command the palette uses rather than hand-writing an element
 * literal that would drift from the generated type.
 */
function businessRuleDocument(decisionRef: string | null) {
  useModelerStore.getState().setDocument(structuredClone(sampleDocument));
  useModelerStore.getState().execute(replaceTaskTypeCommand('review', 'businessRuleTask'));
  const document = structuredClone(useModelerStore.getState().document);
  const task = document.model.processes[0]?.flowElements?.find(
    (element) => element.id === 'review',
  );
  if (!task || task.elementType !== 'businessRuleTask') {
    throw new Error('review should be a business rule task');
  }
  task.decisionRef = decisionRef;
  task.resultVariableName = 'decisionOutcome';
  return document;
}

describe('properties panel selection states', () => {
  it('shows process-level properties when nothing is selected', () => {
    const html = renderPanel();
    expect(html).toContain('data-panel-state="process"');
    expect(html).toContain('value="leaveProcess"');
    expect(html).toContain('value="Leave approval"');
    expect(html).toContain('A representative Flowable process rendered from the editor protocol.');
  });

  it('shows a read-only hint for multi-select', () => {
    const html = renderPanel(['review', 'notify']);
    expect(html).toContain('data-panel-state="multi-select"');
    expect(html).toContain('Multiple elements selected');
    expect(html).not.toContain('data-property="assignee"');
  });

  it('lists the participants of a multi-pool document so each process is reachable', () => {
    const html = renderMultiPoolPanel();

    expect(html).toContain('data-property-group="pools"');
    expect(html).toContain('data-pool-target="leavePool"');
    expect(html).toContain('data-pool-target="vacationPool"');
    expect(html).toContain('This document has 2 participants.');
  });

  it('keeps the single-pool panel free of the participant list', () => {
    expect(renderPanel()).not.toContain('data-property-group="pools"');
  });

  it('notes a selection that is no longer part of the document', () => {
    const html = renderPanel(['ghostShape']);
    expect(html).toContain('data-panel-state="unsupported"');
  });

  it('edits a pool and the process it points at', () => {
    const html = renderPanel(['leavePool']);

    expect(html).toContain('data-panel-state="pool"');
    expect(html).toContain('data-property-group="pool-process"');
    expect(html).toContain('value="leavePool"');
    expect(html).toContain('data-property="processId"');
    expect(html).toContain('value="leaveProcess"');
    expect(html).toContain('data-property="processDocumentation"');
  });

  it('edits a lane and reports its membership', () => {
    const html = renderPanel(['managerLane']);

    expect(html).toContain('data-panel-state="lane"');
    expect(html).toContain('value="managerLane"');
    expect(html).toContain('value="Manager"');
    expect(html).toContain('2 elements in this lane.');
  });

  it('edits a text annotation and an association', () => {
    const annotation = renderPanel(['approvalNote']);
    expect(annotation).toContain('data-panel-state="artifact"');
    expect(annotation).toContain('data-property="text"');

    const association = renderPanel(['approvalLink']);
    expect(association).toContain('data-property="associationDirection"');
  });
});

describe('properties panel element groups', () => {
  it('renders general, execution, assignment, and form groups for a user task', () => {
    const html = renderPanel(['review']);

    expect(html).toContain('data-panel-state="element"');
    expect(html).toContain('<h1>Review request</h1>');
    expect(html).toContain('data-property="id"');
    expect(html).toContain('data-property="name"');
    expect(html).toContain('data-property="documentation"');
    expect(html).toContain('data-property="asynchronous"');
    expect(html).toContain('data-property="exclusive"');

    expect(html).toContain('data-property="assignee"');
    expect(html).toContain('value="managers"');
    expect(html).toContain('value="leaveRequest"');
    expect(html).toContain('value="50"');
    expect(html).toContain('data-property="dueDate"');
    expect(html).toContain('data-property="category"');
  });

  it('renders the implementation group for a service task', () => {
    const html = renderPanel(['notify']);

    expect(html).toContain('data-property="implementationType"');
    expect(html).toContain('Delegate expression');
    expect(html).toContain('value="${notificationDelegate}"');
    expect(html).toContain('data-property="resultVariableName"');
    expect(html).not.toContain('data-property="assignee"');
  });

  it('renders the condition group for a sequence flow', () => {
    const html = renderPanel(['approvedFlow']);

    expect(html).toContain('data-property="conditionExpression"');
    expect(html).toContain('${approved}');
    expect(html).not.toContain('data-property="asynchronous"');
  });

  it('renders multi-instance and listener groups for a user task', () => {
    const html = renderPanel(['review']);
    expect(html).toContain('data-property-group="multi-instance"');
    expect(html).toContain('data-property="multiInstanceEnabled"');
    expect(html).toContain('data-property-group="task-listeners"');
    expect(html).toContain('data-property-group="execution-listeners"');
  });

  it('renders field injection for a service task', () => {
    const html = renderPanel(['notify']);
    expect(html).toContain('data-property-group="field-injection"');
  });

  it('renders global signal and message definition editors for the process', () => {
    const html = renderPanel();
    expect(html).toContain('data-property-group="signals"');
    expect(html).toContain('data-property-group="messages"');
    expect(html).toContain('+ Add signal');
    expect(html).toContain('+ Add message');
  });

  it('renders the timer group for a boundary timer event', () => {
    const html = renderPanel(['reviewTimer']);
    expect(html).toContain('data-property-group="timer-definition"');
    expect(html).toContain('data-property="timerType"');
    expect(html).toContain('data-property="timeDuration"');
    expect(html).toContain('value="PT48H"');
    expect(html).toContain('data-property="calendarName"');
    // Only the active timer kind gets an editor.
    expect(html).not.toContain('data-property="timeCycle"');
    expect(html).not.toContain('data-property="timeDate"');
  });

  it('omits the timer group for elements that cannot hold a timer', () => {
    const html = renderPanel(['review']);
    expect(html).not.toContain('data-property-group="timer-definition"');
  });

  it('renders error and escalation editors for a boundary event', () => {
    const html = renderPanel(['reviewTimer']);
    expect(html).toContain('data-property-group="error-escalation"');
    expect(html).toContain('data-property="errorRef"');
    expect(html).toContain('data-property="errorCode"');
    expect(html).toContain('data-property="escalationRef"');
    expect(html).toContain('data-property="escalationCode"');
  });

  it('omits error and escalation editors for a user task', () => {
    const html = renderPanel(['review']);
    expect(html).not.toContain('data-property-group="error-escalation"');
  });

  it('renders the global escalation catalog for the process', () => {
    const html = renderPanel();
    expect(html).toContain('data-property-group="escalations"');
    expect(html).toContain('+ Add escalation');
  });

  it('renders the form properties group for a user task', () => {
    const html = renderPanel(['review']);
    expect(html).toContain('data-property-group="form-properties"');
    expect(html).toContain('+ Add form property');
  });

  it('renders the form properties group for a start event', () => {
    const html = renderPanel(['start']);
    expect(html).toContain('data-property-group="form-properties"');
  });

  it('renders a row per existing form property', () => {
    const document = structuredClone(sampleDocument);
    const review = document.model.processes[0]?.flowElements?.find(
      (element) => element.id === 'review',
    );
    if (!review || review.elementType !== 'userTask') throw new Error('review is missing');
    review.formProperties = [
      {
        attributes: {},
        extensionElements: {},
        formValues: [],
        id: 'amount',
        name: 'Amount',
        type: 'long',
        variable: 'amount',
        readable: true,
        writeable: false,
        required: true,
        datePattern: null,
        defaultExpression: null,
        expression: null,
        xmlColumnNumber: 0,
        xmlRowNumber: 0,
      },
    ];

    const html = renderToStaticMarkup(
      <PropertiesPanel panelState={{ document, selectedElementIds: ['review'] }} />,
    );
    expect(html).toContain('data-property="formPropertyId-0"');
    expect(html).toContain('data-property="formPropertyType-0"');
    expect(html).toContain('data-property="formPropertyVariable-0"');
    expect(html).toContain('data-property="formPropertyRequired-0"');
    expect(html).toContain('data-property="formPropertyWriteable-0"');
    expect(html).toContain('value="Amount"');
  });

  it('omits the form properties group for a service task', () => {
    const html = renderPanel(['notify']);
    expect(html).not.toContain('data-property-group="form-properties"');
  });

  it('renders the decision group for a business rule task', () => {
    const html = renderToStaticMarkup(
      <PropertiesPanel
        panelState={{
          document: businessRuleDocument('leaveDecision'),
          selectedElementIds: ['review'],
        }}
      />,
    );
    expect(html).toContain('data-property-group="decision"');
    expect(html).toContain('data-property="decisionRef"');
    expect(html).toContain('value="leaveDecision"');
    expect(html).toContain('data-property="resultVariableName"');
    expect(html).toContain('value="decisionOutcome"');
  });

  it('renders an empty decision reference when the task has none', () => {
    const html = renderToStaticMarkup(
      <PropertiesPanel
        panelState={{ document: businessRuleDocument(null), selectedElementIds: ['review'] }}
      />,
    );
    expect(html).toContain('data-property="decisionRef"');
    expect(html).not.toContain('value="leaveDecision"');
  });

  it('omits the decision group for a user task', () => {
    const html = renderPanel(['review']);
    expect(html).not.toContain('data-property-group="decision"');
    expect(html).not.toContain('data-property="decisionRef"');
  });

  it('reflects the document it is given for the selected element', () => {
    const document = structuredClone(sampleDocument);
    const review = document.model.processes[0]?.flowElements?.find(
      (element) => element.id === 'review',
    );
    if (!review || review.elementType !== 'userTask') throw new Error('review is missing');
    review.formKey = 'expenseReport';

    const html = renderToStaticMarkup(
      <PropertiesPanel panelState={{ document, selectedElementIds: ['review'] }} />,
    );
    expect(html).toContain('value="expenseReport"');
  });
});
