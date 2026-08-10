import { renderToStaticMarkup } from 'react-dom/server';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it } from 'vitest';

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
});
