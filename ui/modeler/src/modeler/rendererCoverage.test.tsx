import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';

import { BpmnCanvas } from './BpmnCanvas';

describe('BPMN renderer family coverage', () => {
  it('renders complex gateways, annotations, groups, and directed associations from canonical JSON', () => {
    const html = renderToStaticMarkup(<BpmnCanvas />);

    expect(html).toContain('element-complexGateway');
    expect(html).toContain('data-element-id="approvalNote"');
    expect(html).toContain('Two approvals');
    expect(html).toContain('data-element-id="approvalGroup"');
    expect(html).toContain('approvalCategory');
    expect(html).toContain('marker-end="url(#association-arrow)"');
  });
});
