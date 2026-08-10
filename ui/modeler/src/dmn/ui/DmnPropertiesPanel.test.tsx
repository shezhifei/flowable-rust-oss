import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { createDmnEditorStore } from '../index';
import { DmnPropertiesPanel } from './DmnPropertiesPanel';
import { sampleDmnDocument } from './dmnSampleDocument';

function renderPanel(
  selection: Parameters<typeof DmnPropertiesPanel>[0]['selection'] = null,
  document = sampleDmnDocument(),
) {
  const store = createDmnEditorStore(document);
  return renderToStaticMarkup(
    <DmnPropertiesPanel
      decisionId="leaveDecision"
      onSelect={() => undefined}
      selection={selection}
      store={store}
    />,
  );
}

describe('DMN properties panel', () => {
  it('renders the definition and decision general properties', () => {
    const html = renderPanel();

    expect(html).toContain('data-panel-state="dmn-general"');
    expect(html).toContain('value="leaveDefinitions"');
    expect(html).toContain('value="Leave definitions"');
    expect(html).toContain('data-property="key"');
    expect(html).toContain('value="leaveDecision"');
    expect(html).toContain('value="Leave approval"');
    expect(html).toContain('value="leaveTable"');
  });

  it('offers the creatable hit policies with the current one selected', () => {
    const html = renderPanel();

    expect(html).toContain('data-property="hitPolicy"');
    expect(html).toContain('<option value="FIRST" selected="">First</option>');
    expect(html).toContain('<option value="RULE_ORDER">Rule order</option>');
    expect(html).toContain('<option value="PRIORITY">Priority</option>');
    expect(html).not.toContain('COMPLETE');
    // The collect aggregator only appears for COLLECT tables.
    expect(html).not.toContain('data-property="collectOperator"');
  });

  it('keeps an imported COMPLETE hit policy visible but not re-selectable', () => {
    const document = sampleDmnDocument();
    const table = document.model.decisions?.[0]?.decisionTable;
    if (!table) throw new Error('table is missing');
    table.hitPolicy = 'COMPLETE';

    const html = renderPanel(null, document);

    expect(html).toContain('<option value="COMPLETE" selected="">Complete (imported)</option>');
  });

  it('shows the collect aggregator for COLLECT tables', () => {
    const document = sampleDmnDocument();
    const table = document.model.decisions?.[0]?.decisionTable;
    if (!table) throw new Error('table is missing');
    table.hitPolicy = 'COLLECT';

    const html = renderPanel(null, document);

    expect(html).toContain('data-property="collectOperator"');
    expect(html).toContain('<option value="SUM">SUM</option>');
  });

  it('renders the input column editor for a selected input column', () => {
    const html = renderPanel({ kind: 'input', index: 0 });

    expect(html).toContain('data-panel-state="dmn-input-column"');
    expect(html).toContain('value="Leave days"');
    expect(html).toContain('value="leaveDays"');
    expect(html).toContain('<option value="integer" selected="">integer</option>');
    expect(html).toContain('Delete input column');
  });

  it('renders the output column editor for a selected output column', () => {
    const html = renderPanel({ kind: 'output', index: 1 });

    expect(html).toContain('data-panel-state="dmn-output-column"');
    expect(html).toContain('value="reason"');
    expect(html).toContain('data-property="outputValues"');
    expect(html).toContain('Delete output column');
  });

  it('falls back to the general view when the selection is stale', () => {
    const html = renderPanel({ kind: 'output', index: 9 });

    expect(html).toContain('data-panel-state="dmn-general"');
  });
});
