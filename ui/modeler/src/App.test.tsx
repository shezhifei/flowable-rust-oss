import { renderToStaticMarkup } from 'react-dom/server';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it } from 'vitest';

import { App } from './App';

describe('App', () => {
  it('renders the model management page as the entry route', () => {
    const html = renderToStaticMarkup(
      <MemoryRouter initialEntries={['/']}>
        <App />
      </MemoryRouter>,
    );

    expect(html).toContain('Model repository');
    expect(html).toContain('Model list');
    expect(html).toContain('Loading models');
  });

  it('renders the typed BPMN workspace from the Zustand document', () => {
    const html = renderToStaticMarkup(
      <MemoryRouter initialEntries={['/models/sample/bpmn']}>
        <App />
      </MemoryRouter>,
    );

    expect(html).toContain('Leave approval');
    expect(html).toContain('BPMN process canvas');
    expect(html).toContain('data-element-id="review"');
    expect(html).toContain('Protocol 1.0');
    expect(html).toContain('Local draft ready');
    expect(html).toContain('Review request');
    expect(html).toContain('Back to list');
  });
});
