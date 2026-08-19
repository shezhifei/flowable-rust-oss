import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';

import { PropertiesPanel } from './PropertiesPanel';
import { sampleDocument } from './sampleDocument';

function renderPanel(selectedElementIds: string[] = []) {
  return renderToStaticMarkup(
    <PropertiesPanel
      panelState={{ document: structuredClone(sampleDocument), selectedElementIds }}
    />,
  );
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

  it('notes diagram elements that are not editable yet', () => {
    const html = renderPanel(['leavePool']);
    expect(html).toContain('data-panel-state="unsupported"');
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
