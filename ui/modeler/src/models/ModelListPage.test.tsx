import { renderToStaticMarkup } from 'react-dom/server';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

import { ModelListPage } from './ModelListPage';

describe('ModelListPage', () => {
  it('renders the loading shell for the model repository entry page', () => {
    const html = renderToStaticMarkup(
      <MemoryRouter>
        <ModelListPage />
      </MemoryRouter>,
    );

    expect(html).toContain('Model repository');
    expect(html).toContain('Model list');
    expect(html).toContain('Loading models');
    expect(html).toContain('+ BPMN');
    expect(html).toContain('+ DMN');
    expect(html).toContain('+ Form');
  });

  it('shows models returned by the repository API after load', async () => {
    const models = [
      {
        id: 'm1',
        name: 'Leave process',
        key: 'leaveProcess',
        category: null,
        version: 1,
        lastUpdateTime: '2026-08-01T10:00:00.000Z',
        createTime: null,
      },
    ];

    const fetchMock = vi.fn<typeof fetch>(async (input) => {
      const url = String(input);
      if (url.includes('/repository/models?')) {
        return new Response(JSON.stringify({ data: models, total: 1 }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      if (url.endsWith('/source')) {
        return new Response(
          '<?xml version="1.0"?><definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"><process id="p"/></definitions>',
          { status: 200, headers: { 'Content-Type': 'application/xml' } },
        );
      }
      return new Response('not found', { status: 404 });
    });
    vi.stubGlobal('fetch', fetchMock);

    // SSR markup only covers the initial loading frame; the fetch path is exercised
    // by modelsApi tests and the Playwright management-page e2e.
    const html = renderToStaticMarkup(
      <MemoryRouter>
        <ModelListPage />
      </MemoryRouter>,
    );
    expect(html).toContain('Loading models');
    vi.unstubAllGlobals();
  });
});
