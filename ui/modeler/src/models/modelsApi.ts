import { ModelerApiError } from '../modeler/modelerApi';

type FetchLike = typeof fetch;

/** Model kinds the modeler management page can route to an editor. */
export type ModelKind = 'bpmn' | 'dmn' | 'form' | 'unknown';

export interface RepositoryModelSummary {
  id: string;
  name: string | null;
  key: string;
  category: string | null;
  version: number;
  lastUpdateTime: string | null;
  createTime: string | null;
}

interface PagedModels {
  data: RepositoryModelSummary[];
  total: number;
}

export async function listModels(fetcher: FetchLike = fetch): Promise<RepositoryModelSummary[]> {
  const response = await fetcher('/repository/models?size=1000', {
    credentials: 'same-origin',
    headers: { Accept: 'application/json' },
  });
  if (!response.ok) throw await apiError(response, 'Unable to list models');
  const page = (await response.json()) as PagedModels;
  return page.data;
}

export async function createModel(
  draft: { name: string; key: string },
  fetcher: FetchLike = fetch,
): Promise<RepositoryModelSummary> {
  const response = await fetcher('/repository/models', {
    method: 'POST',
    credentials: 'same-origin',
    headers: { Accept: 'application/json', 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: draft.name, key: draft.key, version: 1 }),
  });
  if (!response.ok) throw await apiError(response, `Unable to create model '${draft.key}'`);
  return (await response.json()) as RepositoryModelSummary;
}

/**
 * The clone endpoint lives on the modeler app rather than the repository API and
 * answers with the Java `ModelRepresentation` shape — a different set of fields
 * from `RepositoryModelSummary`, hence its own type. An empty body lets the
 * server derive the `-copy` key and ` (copy)` name; a duplicate key comes back
 * as a 409 carrying the server's message.
 */
export interface ClonedModel {
  id: string;
  name: string | null;
  key: string;
  version: number;
}

export async function cloneModel(
  modelId: string,
  body: { name?: string; key?: string } = {},
  fetcher: FetchLike = fetch,
): Promise<ClonedModel> {
  const response = await fetcher(`/modeler-app/rest/models/${encodeURIComponent(modelId)}/clone`, {
    method: 'POST',
    credentials: 'same-origin',
    headers: { Accept: 'application/json', 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!response.ok) throw await apiError(response, `Unable to clone model '${modelId}'`);
  return (await response.json()) as ClonedModel;
}

export async function deleteModel(modelId: string, fetcher: FetchLike = fetch): Promise<void> {
  const response = await fetcher(`/repository/models/${encodeURIComponent(modelId)}`, {
    method: 'DELETE',
    credentials: 'same-origin',
  });
  if (!response.ok) throw await apiError(response, `Unable to delete model '${modelId}'`);
}

export async function getModelSource(
  modelId: string,
  fetcher: FetchLike = fetch,
): Promise<{ contentType: string; text: string }> {
  const response = await fetcher(`/repository/models/${encodeURIComponent(modelId)}/source`, {
    credentials: 'same-origin',
  });
  if (!response.ok) throw await apiError(response, `Unable to read the source of '${modelId}'`);
  return {
    contentType: response.headers.get('content-type') ?? 'application/octet-stream',
    text: await response.text(),
  };
}

export async function updateModelSource(
  modelId: string,
  contentType: string,
  source: string,
  fetcher: FetchLike = fetch,
): Promise<void> {
  const response = await fetcher(`/repository/models/${encodeURIComponent(modelId)}/source`, {
    method: 'PUT',
    credentials: 'same-origin',
    headers: { 'Content-Type': contentType },
    body: source,
  });
  if (!response.ok) throw await apiError(response, `Unable to write the source of '${modelId}'`);
}

/** Deploys a BPMN model through the Java-compatible multipart endpoint. */
export async function deployBpmnModel(
  name: string,
  resourceName: string,
  source: string,
  fetcher: FetchLike = fetch,
): Promise<void> {
  const form = new FormData();
  form.append('file', new Blob([source], { type: 'application/xml' }), resourceName);
  const response = await fetcher(
    `/repository/deployments?deploymentName=${encodeURIComponent(name)}`,
    { method: 'POST', credentials: 'same-origin', body: form },
  );
  if (!response.ok) throw await apiError(response, `Unable to deploy '${name}'`);
}

/** Deploys a DMN or form model through its engine repository endpoint. */
export async function deployDefinitionModel(
  kind: 'dmn' | 'form',
  name: string,
  resourceName: string,
  source: string,
  fetcher: FetchLike = fetch,
): Promise<void> {
  const base = kind === 'dmn' ? '/dmn-repository/deployments' : '/form-repository/deployments';
  const response = await fetcher(base, {
    method: 'POST',
    credentials: 'same-origin',
    headers: { Accept: 'application/json', 'Content-Type': 'application/json' },
    body: JSON.stringify({
      name,
      resourceName,
      resource: kind === 'form' ? deployableFormSource(source) : source,
    }),
  });
  if (!response.ok) throw await apiError(response, `Unable to deploy '${name}'`);
}

/**
 * The stored form source is a `FormEditorDocument` envelope; the form
 * repository deploys only the bare Flowable form model, so unwrap on publish.
 * Sources that are already bare (or unparseable) pass through untouched and
 * let the server report the problem.
 */
function deployableFormSource(source: string): string {
  try {
    const parsed = JSON.parse(source) as Record<string, unknown>;
    if (parsed && typeof parsed === 'object' && 'model' in parsed && 'schemaVersion' in parsed) {
      return JSON.stringify(parsed.model);
    }
    return source;
  } catch {
    return source;
  }
}

/**
 * Detects the model kind from its stored source, mirroring
 * `detect_model_kind` in `flowable-ui-rest`: JSON sources are forms, DMN
 * namespaces are decision tables, everything else XML is BPMN. The REST model
 * list carries no model type, so the management page sniffs per model.
 */
export function detectModelKind(sourceText: string): ModelKind {
  const trimmed = sourceText.trimStart();
  if (!trimmed) return 'unknown';
  if (trimmed.startsWith('{')) return 'form';
  const lowered = trimmed.toLowerCase();
  if (lowered.includes('spec/dmn') || lowered.includes('<decision')) return 'dmn';
  if (trimmed.startsWith('<')) return 'bpmn';
  return 'unknown';
}

export function editorPath(kind: ModelKind, modelId: string): string | null {
  switch (kind) {
    case 'bpmn':
      return `/models/${modelId}/bpmn`;
    case 'dmn':
      return `/models/${modelId}/dmn`;
    case 'form':
      return `/models/${modelId}/form`;
    default:
      return null;
  }
}

/** Minimal starter sources written right after creating a model. */
export function stubSource(kind: Exclude<ModelKind, 'unknown'>, key: string, name: string): string {
  switch (kind) {
    case 'bpmn':
      return `<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" xmlns:omgdc="http://www.omg.org/spec/DD/20100524/DC" xmlns:omgdi="http://www.omg.org/spec/DD/20100524/DI" targetNamespace="https://flowable.org/modeler">
  <process id="${escapeXml(key)}" name="${escapeXml(name)}" isExecutable="true">
    <startEvent id="start"/>
    <endEvent id="end"/>
    <sequenceFlow id="flow1" sourceRef="start" targetRef="end"/>
  </process>
  <BPMNDiagram>
    <BPMNPlane bpmnElement="${escapeXml(key)}">
      <BPMNShape bpmnElement="start"><omgdc:Bounds x="130" y="100" width="30" height="30"/></BPMNShape>
      <BPMNShape bpmnElement="end"><omgdc:Bounds x="260" y="100" width="28" height="28"/></BPMNShape>
      <BPMNEdge bpmnElement="flow1"><omgdi:waypoint x="160" y="115"/><omgdi:waypoint x="260" y="114"/></BPMNEdge>
    </BPMNPlane>
  </BPMNDiagram>
</definitions>`;
    case 'dmn':
      return `<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/" id="${escapeXml(key)}Definitions" name="${escapeXml(name)}" namespace="https://flowable.org/modeler">
  <decision id="${escapeXml(key)}" name="${escapeXml(name)}">
    <decisionTable id="${escapeXml(key)}Table" hitPolicy="FIRST">
      <output id="${escapeXml(key)}Output" name="result" typeRef="string"/>
      <rule id="${escapeXml(key)}Rule1"><outputEntry id="${escapeXml(key)}Rule1Output"><text>""</text></outputEntry></rule>
    </decisionTable>
  </decision>
</definitions>`;
    case 'form':
      return JSON.stringify(
        {
          schemaVersion: '1.0',
          model: { key, name, description: null, fields: [], outcomes: [] },
        },
        null,
        2,
      );
  }
}

export function stubContentType(kind: Exclude<ModelKind, 'unknown'>): string {
  return kind === 'form' ? 'application/json' : 'application/xml';
}

export function resourceNameFor(kind: ModelKind, key: string): string {
  switch (kind) {
    case 'bpmn':
      return `${key}.bpmn20.xml`;
    case 'dmn':
      return `${key}.dmn`;
    case 'form':
      // The form repository only accepts `.form` resources (Java parity);
      // `.form.json` is rejected at deploy time.
      return `${key}.form`;
    default:
      return key;
  }
}

/** Maps an imported file extension to a model kind. */
export function kindFromFileName(fileName: string): Exclude<ModelKind, 'unknown'> | null {
  const lowered = fileName.toLowerCase();
  if (lowered.endsWith('.bpmn20.xml') || lowered.endsWith('.bpmn')) return 'bpmn';
  if (lowered.endsWith('.dmn') || lowered.endsWith('.dmn.xml')) return 'dmn';
  if (lowered.endsWith('.form') || lowered.endsWith('.form.json') || lowered.endsWith('.json')) {
    return 'form';
  }
  return null;
}

/**
 * Imported `.form` files may carry the bare Flowable form model; the editor
 * boundary expects the `FormEditorDocument` wrapper, so wrap when needed.
 */
export function normalizeFormSource(text: string, key: string, name: string): string {
  try {
    const parsed = JSON.parse(text) as Record<string, unknown>;
    if (parsed && typeof parsed === 'object' && 'model' in parsed && 'schemaVersion' in parsed) {
      return text;
    }
    return JSON.stringify(
      { schemaVersion: '1.0', model: { key, name, fields: [], ...parsed } },
      null,
      2,
    );
  } catch {
    return JSON.stringify(
      { schemaVersion: '1.0', model: { key, name, fields: [], outcomes: [] } },
      null,
      2,
    );
  }
}

function escapeXml(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&apos;');
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
