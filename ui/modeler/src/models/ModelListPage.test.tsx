import { renderToStaticMarkup } from 'react-dom/server';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it } from 'vitest';

import { ModelListPage, ModelRowActions, type ModelEntry } from './ModelListPage';

const entry: ModelEntry = {
  id: 'm1',
  name: 'Leave',
  key: 'leave',
  category: null,
  version: 1,
  lastUpdateTime: null,
  createTime: null,
  kind: 'bpmn',
};

/** The rendered `<button …>` open tag carrying `label` as its aria-label. */
function buttonTag(html: string, label: string): string {
  const match = new RegExp(`<button[^>]*aria-label="${label}"[^>]*>`).exec(html);
  if (!match) throw new Error(`no button labelled '${label}' in ${html}`);
  return match[0];
}

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

describe('ModelRowActions', () => {
  const noop = () => {};

  it('offers clone beside publish and delete', () => {
    const html = renderToStaticMarkup(
      <ModelRowActions entry={entry} onClone={noop} onDelete={noop} onPublish={noop} />,
    );

    expect(html).toContain('Clone');
    expect(html).toContain('Publish');
    expect(html).toContain('Delete');
    expect(buttonTag(html, 'Clone model Leave')).not.toContain('disabled');
  });

  it('keeps clone available for a model of unknown kind', () => {
    // Clone copies the stored bytes server-side, so unlike publish it does not
    // depend on the kind sniffed from the source.
    const html = renderToStaticMarkup(
      <ModelRowActions
        entry={{ ...entry, kind: 'unknown' }}
        onClone={noop}
        onDelete={noop}
        onPublish={noop}
      />,
    );

    expect(buttonTag(html, 'Clone model Leave')).not.toContain('disabled');
    expect(buttonTag(html, 'Publish model Leave')).toContain('disabled');
  });

  it('labels the actions by key when the model has no name', () => {
    const html = renderToStaticMarkup(
      <ModelRowActions
        entry={{ ...entry, name: null }}
        onClone={noop}
        onDelete={noop}
        onPublish={noop}
      />,
    );

    expect(buttonTag(html, 'Clone model leave')).toBeTruthy();
    expect(buttonTag(html, 'Delete model leave')).toBeTruthy();
  });
});
