import type {
  BpmnEditorDocument,
  DmnEditorDocument,
  FormEditorDocument,
} from '../generated/editor-protocol';

type FetchLike = typeof fetch;

export class ModelerApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
    this.name = 'ModelerApiError';
  }
}

export async function loadBpmnDocument(
  modelId: string,
  fetcher: FetchLike = fetch,
): Promise<BpmnEditorDocument> {
  const response = await fetcher(editorUrl(modelId), {
    credentials: 'same-origin',
    headers: { Accept: 'application/json' },
  });
  if (!response.ok) throw await apiError(response, `Unable to load model '${modelId}'`);
  return (await response.json()) as BpmnEditorDocument;
}

export async function loadDmnDocument(
  modelId: string,
  fetcher: FetchLike = fetch,
): Promise<DmnEditorDocument> {
  const response = await fetcher(dmnEditorUrl(modelId), {
    credentials: 'same-origin',
    headers: { Accept: 'application/json' },
  });
  if (!response.ok) throw await apiError(response, `Unable to load model '${modelId}'`);
  return (await response.json()) as DmnEditorDocument;
}

export async function saveDmnDocument(
  modelId: string,
  document: DmnEditorDocument,
  fetcher: FetchLike = fetch,
): Promise<DmnEditorDocument> {
  const response = await fetcher(dmnEditorUrl(modelId), {
    method: 'PUT',
    credentials: 'same-origin',
    headers: {
      Accept: 'application/json',
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(document),
  });
  if (!response.ok) throw await apiError(response, `Unable to save model '${modelId}'`);

  // The server re-encodes JSON through the canonical Rust model and XML writer.
  // Reading it back makes that authoritative representation the next editor state.
  return loadDmnDocument(modelId, fetcher);
}

export async function saveBpmnDocument(
  modelId: string,
  document: BpmnEditorDocument,
  fetcher: FetchLike = fetch,
): Promise<BpmnEditorDocument> {
  const response = await fetcher(editorUrl(modelId), {
    method: 'PUT',
    credentials: 'same-origin',
    headers: {
      Accept: 'application/json',
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(document),
  });
  if (!response.ok) throw await apiError(response, `Unable to save model '${modelId}'`);

  // The server re-encodes JSON through the canonical Rust model and XML writer.
  // Reading it back makes that authoritative representation the next editor state.
  return loadBpmnDocument(modelId, fetcher);
}

export async function loadFormDocument(
  modelId: string,
  fetcher: FetchLike = fetch,
): Promise<FormEditorDocument> {
  const response = await fetcher(formEditorUrl(modelId), {
    credentials: 'same-origin',
    headers: { Accept: 'application/json' },
  });
  if (!response.ok) throw await apiError(response, `Unable to load model '${modelId}'`);
  return (await response.json()) as FormEditorDocument;
}

export async function saveFormDocument(
  modelId: string,
  document: FormEditorDocument,
  fetcher: FetchLike = fetch,
): Promise<FormEditorDocument> {
  const response = await fetcher(formEditorUrl(modelId), {
    method: 'PUT',
    credentials: 'same-origin',
    headers: {
      Accept: 'application/json',
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(document),
  });
  if (!response.ok) throw await apiError(response, `Unable to save model '${modelId}'`);

  // The server validates and re-encodes the form JSON through the canonical
  // Rust model. Reading it back makes that representation the editor state.
  return loadFormDocument(modelId, fetcher);
}

function editorUrl(modelId: string) {
  return `/modeler-app/rest/models/${encodeURIComponent(modelId)}/editor/bpmn-json`;
}

function dmnEditorUrl(modelId: string) {
  return `/modeler-app/rest/models/${encodeURIComponent(modelId)}/editor/dmn-json`;
}

function formEditorUrl(modelId: string) {
  return `/modeler-app/rest/form-models/${encodeURIComponent(modelId)}/editor/form-json`;
}

async function apiError(response: Response, fallback: string) {
  let message = fallback;
  try {
    const payload = (await response.json()) as { message?: unknown };
    if (typeof payload.message === 'string' && payload.message.trim()) message = payload.message;
  } catch {
    const text = await response.text().catch(() => '');
    if (text.trim()) message = text;
  }
  return new ModelerApiError(message, response.status);
}
