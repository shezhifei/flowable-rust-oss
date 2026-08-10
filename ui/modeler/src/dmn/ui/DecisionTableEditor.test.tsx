import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { createDmnEditorStore } from '../index';
import { DecisionTableEditor } from './DecisionTableEditor';
import { sampleDmnDocument } from './dmnSampleDocument';

function renderTable(selection: Parameters<typeof DecisionTableEditor>[0]['selection'] = null) {
  const store = createDmnEditorStore(sampleDmnDocument());
  return renderToStaticMarkup(
    <DecisionTableEditor
      decisionId="leaveDecision"
      onSelect={() => undefined}
      selection={selection}
      store={store}
    />,
  );
}

describe('decision table editor rendering', () => {
  it('renders grouped input and output headers with the hit policy badge', () => {
    const html = renderTable();

    expect(html).toContain('decision-table');
    expect(html).toContain('hit-policy-badge');
    expect(html).toContain('FIRST');
    expect(html).toContain('dmn-group-input');
    expect(html).toContain('dmn-group-output');
    expect(html).toContain('aria-label="Add input column"');
    expect(html).toContain('aria-label="Add output column"');
  });

  it('renders column labels with expression and type metadata', () => {
    const html = renderTable();

    expect(html).toContain('Leave days');
    expect(html).toContain('leaveDays : integer');
    expect(html).toContain('Role');
    expect(html).toContain('Status');
    expect(html).toContain('status : string');
    expect(html).toContain('aria-label="Input column 1"');
    expect(html).toContain('aria-label="Output column 2"');
  });

  it('renders rule rows with cell values and row controls', () => {
    const html = renderTable();

    expect(html).toContain('value="&gt; 5"');
    expect(html).toContain('value="&quot;manager&quot;"');
    expect(html).toContain('value="&quot;APPROVED&quot;"');
    expect(html).toContain('value="&quot;REVIEW&quot;"');
    expect(html).toContain('aria-label="input cell 1:1"');
    expect(html).toContain('aria-label="output cell 2:2"');
    expect(html).toContain('aria-label="Move rule 2 up"');
    expect(html).toContain('aria-label="Delete rule 1"');
    expect(html).toContain('+ Add rule');
  });

  it('marks the selected column header', () => {
    const html = renderTable({ kind: 'input', index: 1 });

    expect(html).toContain('dmn-column dmn-column-input is-selected');
    expect(html).toContain('aria-pressed="true"');
  });

  it('disables boundary move controls for the first and last rule', () => {
    const html = renderTable();

    expect(html).toMatch(/aria-label="Move rule 1 up"[^>]*disabled/);
    expect(html).toMatch(/aria-label="Move rule 2 down"[^>]*disabled/);
  });
});
