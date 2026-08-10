import { describe, expect, it, vi } from 'vitest';

import type { DmnEditorDocument, FormEditorDocument } from '../generated/editor-protocol';
import { sampleDocument } from './sampleDocument';
import {
  loadBpmnDocument,
  loadDmnDocument,
  loadFormDocument,
  ModelerApiError,
  saveBpmnDocument,
  saveDmnDocument,
  saveFormDocument,
} from './modelerApi';

const dmnDocument: DmnEditorDocument = {
  schemaVersion: '1.0',
  model: {
    id: 'leaveDefinitions',
    decisions: [
      {
        id: 'leaveDecision',
        decisionTable: { id: 'leaveTable', hitPolicy: 'FIRST' },
      },
    ],
  },
};

const formDocument: FormEditorDocument = {
  schemaVersion: '1.0',
  model: {
    key: 'leaveForm',
    name: 'Leave request',
    fields: [{ fieldType: 'BaseField', id: 'reason', type: 'text', name: 'Reason' }],
    outcomes: [{ id: 'submit', name: 'Submit' }],
  },
};

describe('modeler API client', () => {
  it('loads a canonical BPMN document through the cookie-authenticated UI endpoint', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify(sampleDocument), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }),
    );

    await expect(loadBpmnDocument('leave/request', fetcher)).resolves.toEqual(sampleDocument);
    expect(fetcher).toHaveBeenCalledWith(
      '/modeler-app/rest/models/leave%2Frequest/editor/bpmn-json',
      expect.objectContaining({ credentials: 'same-origin' }),
    );
  });

  it('saves and reloads the server-normalized document', async () => {
    const normalized = structuredClone(sampleDocument);
    normalized.model.processes[0]!.name = 'Server normalized';
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
      .mockResolvedValueOnce(
        new Response(JSON.stringify(normalized), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }),
      );

    await expect(saveBpmnDocument('leave', sampleDocument, fetcher)).resolves.toEqual(normalized);
    expect(fetcher).toHaveBeenNthCalledWith(
      1,
      '/modeler-app/rest/models/leave/editor/bpmn-json',
      expect.objectContaining({ method: 'PUT', body: JSON.stringify(sampleDocument) }),
    );
    expect(fetcher).toHaveBeenNthCalledWith(
      2,
      '/modeler-app/rest/models/leave/editor/bpmn-json',
      expect.objectContaining({ credentials: 'same-origin' }),
    );
  });

  it('surfaces the UI error body and response status', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ message: 'Model is invalid' }), {
        status: 400,
        headers: { 'Content-Type': 'application/json' },
      }),
    );

    const error = await loadBpmnDocument('broken', fetcher).catch((reason: unknown) => reason);
    expect(error).toBeInstanceOf(ModelerApiError);
    expect(error).toMatchObject({ message: 'Model is invalid', status: 400 });
  });

  it('loads a DMN document through the dmn-json endpoint', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify(dmnDocument), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }),
    );

    await expect(loadDmnDocument('leave/dmn', fetcher)).resolves.toEqual(dmnDocument);
    expect(fetcher).toHaveBeenCalledWith(
      '/modeler-app/rest/models/leave%2Fdmn/editor/dmn-json',
      expect.objectContaining({ credentials: 'same-origin' }),
    );
  });

  it('saves a DMN document and reloads the server-normalized document', async () => {
    const normalized = structuredClone(dmnDocument);
    normalized.model.name = 'Server normalized';
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
      .mockResolvedValueOnce(
        new Response(JSON.stringify(normalized), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }),
      );

    await expect(saveDmnDocument('leave', dmnDocument, fetcher)).resolves.toEqual(normalized);
    expect(fetcher).toHaveBeenNthCalledWith(
      1,
      '/modeler-app/rest/models/leave/editor/dmn-json',
      expect.objectContaining({ method: 'PUT', body: JSON.stringify(dmnDocument) }),
    );
    expect(fetcher).toHaveBeenNthCalledWith(
      2,
      '/modeler-app/rest/models/leave/editor/dmn-json',
      expect.objectContaining({ credentials: 'same-origin' }),
    );
  });

  it('loads a form document through the form-json endpoint', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify(formDocument), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }),
    );

    await expect(loadFormDocument('leave/form', fetcher)).resolves.toEqual(formDocument);
    expect(fetcher).toHaveBeenCalledWith(
      '/modeler-app/rest/form-models/leave%2Fform/editor/form-json',
      expect.objectContaining({ credentials: 'same-origin' }),
    );
  });

  it('saves a form document and reloads the server-normalized document', async () => {
    const normalized = structuredClone(formDocument);
    normalized.model.name = 'Server normalized form';
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(new Response(null, { status: 204 }))
      .mockResolvedValueOnce(
        new Response(JSON.stringify(normalized), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }),
      );

    await expect(saveFormDocument('leave', formDocument, fetcher)).resolves.toEqual(normalized);
    expect(fetcher).toHaveBeenNthCalledWith(
      1,
      '/modeler-app/rest/form-models/leave/editor/form-json',
      expect.objectContaining({ method: 'PUT', body: JSON.stringify(formDocument) }),
    );
    expect(fetcher).toHaveBeenNthCalledWith(
      2,
      '/modeler-app/rest/form-models/leave/editor/form-json',
      expect.objectContaining({ credentials: 'same-origin' }),
    );
  });
});
