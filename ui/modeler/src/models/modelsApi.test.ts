import { describe, expect, it, vi } from 'vitest';

import {
  cloneModel,
  createModel,
  deleteModel,
  deployBpmnModel,
  deployDefinitionModel,
  detectModelKind,
  editorPath,
  kindFromFileName,
  listModels,
  normalizeFormSource,
  resourceNameFor,
  stubContentType,
  stubSource,
  updateModelSource,
} from './modelsApi';

describe('models API client', () => {
  it('lists repository models', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(
        JSON.stringify({
          data: [
            {
              id: 'm1',
              name: 'Leave',
              key: 'leave',
              category: null,
              version: 1,
              lastUpdateTime: null,
              createTime: null,
            },
          ],
          total: 1,
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      ),
    );

    await expect(listModels(fetcher)).resolves.toEqual([
      {
        id: 'm1',
        name: 'Leave',
        key: 'leave',
        category: null,
        version: 1,
        lastUpdateTime: null,
        createTime: null,
      },
    ]);
    expect(fetcher).toHaveBeenCalledWith(
      '/repository/models?size=1000',
      expect.objectContaining({ credentials: 'same-origin' }),
    );
  });

  it('creates, updates source, and deletes models', async () => {
    const createFetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(
        JSON.stringify({
          id: 'm2',
          name: 'New form',
          key: 'newForm',
          category: null,
          version: 1,
          lastUpdateTime: null,
          createTime: null,
        }),
        { status: 201, headers: { 'Content-Type': 'application/json' } },
      ),
    );
    await expect(createModel({ name: 'New form', key: 'newForm' }, createFetcher)).resolves.toMatchObject(
      { id: 'm2', key: 'newForm' },
    );

    const sourceFetcher = vi.fn<typeof fetch>().mockResolvedValue(new Response(null, { status: 204 }));
    await updateModelSource('m2', 'application/json', '{"schemaVersion":"1.0"}', sourceFetcher);
    expect(sourceFetcher).toHaveBeenCalledWith(
      '/repository/models/m2/source',
      expect.objectContaining({ method: 'PUT' }),
    );

    const deleteFetcher = vi.fn<typeof fetch>().mockResolvedValue(new Response(null, { status: 204 }));
    await deleteModel('m2', deleteFetcher);
    expect(deleteFetcher).toHaveBeenCalledWith(
      '/repository/models/m2',
      expect.objectContaining({ method: 'DELETE' }),
    );
  });

  it('deploys BPMN through multipart and DMN/form through definition endpoints', async () => {
    const bpmnFetcher = vi.fn<typeof fetch>().mockResolvedValue(new Response(null, { status: 201 }));
    await deployBpmnModel('Leave', 'leave.bpmn20.xml', '<definitions/>', bpmnFetcher);
    expect(bpmnFetcher).toHaveBeenCalledWith(
      '/repository/deployments?deploymentName=Leave',
      expect.objectContaining({ method: 'POST' }),
    );

    const dmnFetcher = vi.fn<typeof fetch>().mockResolvedValue(new Response(null, { status: 201 }));
    await deployDefinitionModel('dmn', 'Rules', 'rules.dmn', '<definitions/>', dmnFetcher);
    expect(dmnFetcher).toHaveBeenCalledWith(
      '/dmn-repository/deployments',
      expect.objectContaining({ method: 'POST' }),
    );

    const formFetcher = vi.fn<typeof fetch>().mockResolvedValue(new Response(null, { status: 201 }));
    await deployDefinitionModel('form', 'Form', 'form.form', '{}', formFetcher);
    expect(formFetcher).toHaveBeenCalledWith(
      '/form-repository/deployments',
      expect.objectContaining({ method: 'POST' }),
    );
  });

  it('clones a model through the modeler clone endpoint', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ id: 'm2', name: 'Leave (copy)', key: 'leave-copy', version: 1 }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }),
    );

    await expect(cloneModel('m1', {}, fetcher)).resolves.toMatchObject({
      id: 'm2',
      key: 'leave-copy',
    });
    expect(fetcher).toHaveBeenCalledWith(
      '/modeler-app/rest/models/m1/clone',
      expect.objectContaining({ method: 'POST', credentials: 'same-origin' }),
    );
    // An empty body lets the server derive the `-copy` key and ` (copy)` name.
    expect(JSON.parse((fetcher.mock.calls[0]![1] as RequestInit).body as string)).toEqual({});
  });

  it('surfaces the duplicate-key conflict from a clone', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ message: 'Provided model key already exists: leave-copy' }), {
        status: 409,
        headers: { 'Content-Type': 'application/json' },
      }),
    );

    await expect(cloneModel('m1', { key: 'leave-copy' }, fetcher)).rejects.toThrow(
      'Provided model key already exists: leave-copy',
    );
    expect(JSON.parse((fetcher.mock.calls[0]![1] as RequestInit).body as string)).toEqual({
      key: 'leave-copy',
    });
  });

  it('unwraps the FormEditorDocument envelope when publishing a form', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(new Response(null, { status: 201 }));
    const editorDocument = JSON.stringify({
      schemaVersion: '1.0',
      model: { key: 'leave', name: 'Leave form', fields: [], outcomes: [] },
    });
    await deployDefinitionModel('form', 'Leave form', 'leave.form', editorDocument, fetcher);
    const body = JSON.parse((fetcher.mock.calls[0]![1] as RequestInit).body as string) as {
      resourceName: string;
      resource: string;
    };
    expect(body.resourceName).toBe('leave.form');
    expect(JSON.parse(body.resource)).toEqual({
      key: 'leave',
      name: 'Leave form',
      fields: [],
      outcomes: [],
    });

    const bareFetcher = vi.fn<typeof fetch>().mockResolvedValue(new Response(null, { status: 201 }));
    const bare = JSON.stringify({ key: 'leave', name: 'Leave form', fields: [] });
    await deployDefinitionModel('form', 'Leave form', 'leave.form', bare, bareFetcher);
    const bareBody = JSON.parse((bareFetcher.mock.calls[0]![1] as RequestInit).body as string) as {
      resource: string;
    };
    expect(bareBody.resource).toBe(bare);
  });
});

describe('models API helpers', () => {
  it('detects model kind from source text and file names', () => {
    expect(detectModelKind('{"key":"f"}')).toBe('form');
    expect(detectModelKind('<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/">')).toBe(
      'dmn',
    );
    expect(detectModelKind('<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">')).toBe(
      'bpmn',
    );
    expect(detectModelKind('')).toBe('unknown');

    expect(kindFromFileName('a.bpmn20.xml')).toBe('bpmn');
    expect(kindFromFileName('a.dmn')).toBe('dmn');
    expect(kindFromFileName('a.form.json')).toBe('form');
    expect(kindFromFileName('a.txt')).toBeNull();
  });

  it('builds editor paths, stubs, and resource names', () => {
    expect(editorPath('bpmn', '1')).toBe('/models/1/bpmn');
    expect(editorPath('dmn', '1')).toBe('/models/1/dmn');
    expect(editorPath('form', '1')).toBe('/models/1/form');
    expect(editorPath('unknown', '1')).toBeNull();

    expect(stubContentType('form')).toBe('application/json');
    expect(stubContentType('bpmn')).toBe('application/xml');
    expect(resourceNameFor('bpmn', 'leave')).toBe('leave.bpmn20.xml');
    expect(resourceNameFor('dmn', 'leave')).toBe('leave.dmn');
    expect(resourceNameFor('form', 'leave')).toBe('leave.form');

    expect(stubSource('form', 'leave', 'Leave')).toContain('"key": "leave"');
    expect(stubSource('bpmn', 'leave', 'Leave')).toContain('process id="leave"');
    expect(stubSource('dmn', 'leave', 'Leave')).toContain('decision id="leave"');
  });

  it('wraps bare form models into FormEditorDocument envelopes', () => {
    const bare = JSON.stringify({ key: 'k', name: 'N', fields: [] });
    const wrapped = normalizeFormSource(bare, 'fallbackKey', 'Fallback');
    expect(JSON.parse(wrapped)).toMatchObject({
      schemaVersion: '1.0',
      model: { key: 'k', name: 'N', fields: [] },
    });

    const already = JSON.stringify({
      schemaVersion: '1.0',
      model: { key: 'k', name: 'N', fields: [] },
    });
    expect(normalizeFormSource(already, 'x', 'y')).toBe(already);
  });
});
