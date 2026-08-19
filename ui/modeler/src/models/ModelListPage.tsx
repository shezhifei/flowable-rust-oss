import { useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import {
  cloneModel,
  createModel,
  deleteModel,
  deployBpmnModel,
  deployDefinitionModel,
  detectModelKind,
  editorPath,
  getModelSource,
  kindFromFileName,
  listModels,
  normalizeFormSource,
  resourceNameFor,
  stubContentType,
  stubSource,
  updateModelSource,
  type ModelKind,
  type RepositoryModelSummary,
} from './modelsApi';

export interface ModelEntry extends RepositoryModelSummary {
  kind: ModelKind;
}

type PageState =
  | { state: 'loading' }
  | { state: 'ready'; models: ModelEntry[] }
  | { state: 'error'; message: string };

const KIND_LABELS: Record<ModelKind, string> = {
  bpmn: 'BPMN',
  dmn: 'DMN',
  form: 'Form',
  unknown: 'Unknown',
};

/**
 * Model management entry page: lists repository models with their detected
 * kind, and offers create / delete / import / publish actions that route into
 * the BPMN, DMN, and form editors.
 */
export function ModelListPage() {
  const navigate = useNavigate();
  const [page, setPage] = useState<PageState>({ state: 'loading' });
  const [notice, setNotice] = useState<{ kind: 'error' | 'status'; message: string } | null>(null);
  const [creating, setCreating] = useState<Exclude<ModelKind, 'unknown'> | null>(null);
  const importInput = useRef<HTMLInputElement>(null);

  const refresh = async () => {
    setPage({ state: 'loading' });
    try {
      const models = await listModels();
      const entries = await Promise.all(
        models.map(async (model) => {
          try {
            const source = await getModelSource(model.id);
            return { ...model, kind: detectModelKind(source.text) };
          } catch {
            return { ...model, kind: 'unknown' as ModelKind };
          }
        }),
      );
      setPage({ state: 'ready', models: entries });
    } catch (error) {
      setPage({
        state: 'error',
        message: error instanceof Error ? error.message : 'Unable to list models',
      });
    }
  };

  useEffect(() => {
    void refresh();
  }, []);

  const reportError = (error: unknown, fallback: string) => {
    setNotice({ kind: 'error', message: error instanceof Error ? error.message : fallback });
  };

  const openModel = (entry: ModelEntry) => {
    const path = editorPath(entry.kind, entry.id);
    if (path) void navigate(path);
  };

  const removeModel = async (entry: ModelEntry) => {
    setNotice(null);
    try {
      await deleteModel(entry.id);
      setNotice({ kind: 'status', message: `Deleted '${entry.name ?? entry.key}'` });
      await refresh();
    } catch (error) {
      reportError(error, `Unable to delete '${entry.key}'`);
    }
  };

  /**
   * Server-side duplicate: the stored bytes are copied verbatim and the server
   * derives the `-copy` key and ` (copy)` name, so an empty body is enough. A
   * duplicate key answers 409 and lands in the error notice like any other
   * failure.
   */
  const duplicateModel = async (entry: ModelEntry) => {
    setNotice(null);
    try {
      const clone = await cloneModel(entry.id);
      setNotice({
        kind: 'status',
        message: `Cloned '${entry.name ?? entry.key}' to '${clone.key}'`,
      });
      await refresh();
    } catch (error) {
      reportError(error, `Unable to clone '${entry.key}'`);
    }
  };

  const publishModel = async (entry: ModelEntry) => {
    setNotice(null);
    try {
      const source = await getModelSource(entry.id);
      const name = entry.name ?? entry.key;
      const resourceName = resourceNameFor(entry.kind, entry.key);
      if (entry.kind === 'bpmn') {
        await deployBpmnModel(name, resourceName, source.text);
      } else if (entry.kind === 'dmn' || entry.kind === 'form') {
        await deployDefinitionModel(entry.kind, name, resourceName, source.text);
      } else {
        throw new Error('Only BPMN, DMN, and form models can be published');
      }
      setNotice({ kind: 'status', message: `Published '${name}'` });
    } catch (error) {
      reportError(error, `Unable to publish '${entry.key}'`);
    }
  };

  const importFile = async (file: File) => {
    setNotice(null);
    const kind = kindFromFileName(file.name);
    if (!kind) {
      setNotice({
        kind: 'error',
        message: `Unsupported file '${file.name}'; expected .bpmn20.xml, .dmn, or .form`,
      });
      return;
    }
    try {
      const text = await file.text();
      const key = slugify(file.name.replace(/\.[^.]+(\.xml|\.json)?$/i, '')) || `imported-${kind}`;
      const model = await createModel({ name: file.name, key });
      const source = kind === 'form' ? normalizeFormSource(text, key, file.name) : text;
      await updateModelSource(model.id, stubContentType(kind), source);
      void navigate(editorPath(kind, model.id)!);
    } catch (error) {
      reportError(error, `Unable to import '${file.name}'`);
    }
  };

  return (
    <main className="modeler-shell">
      <header className="modeler-topbar">
        <a className="wordmark" href="/modeler-app/" aria-label="Flowable Modeler home">
          <span className="wordmark-mark" aria-hidden="true">
            F
          </span>
          <span>
            <strong>Flowable</strong>
            <small>Modeler</small>
          </span>
        </a>
        <div className="model-title-block">
          <span className="document-kind">Models</span>
          <strong>Model repository</strong>
        </div>
        <div className="topbar-actions">
          <button type="button" className="quiet-button" onClick={() => importInput.current?.click()}>
            Import
          </button>
          <input
            ref={importInput}
            type="file"
            accept=".bpmn20.xml,.bpmn,.dmn,.dmn.xml,.form,.form.json,.json"
            className="visually-hidden"
            aria-label="Import model file"
            onChange={(event) => {
              const file = event.target.files?.[0];
              event.target.value = '';
              if (file) void importFile(file);
            }}
          />
          <button type="button" className="avatar-button" aria-label="Account menu">
            AD
          </button>
        </div>
      </header>

      {notice ? (
        <div
          className={notice.kind === 'error' ? 'modeler-notice is-error' : 'modeler-notice'}
          role={notice.kind === 'error' ? 'alert' : 'status'}
        >
          {notice.message}
        </div>
      ) : null}

      <div className="model-list-layout">
        <section className="model-list-panel" aria-label="Model list">
          <div className="model-list-header">
            <h1>Models</h1>
            <div className="model-list-create">
              {(['bpmn', 'dmn', 'form'] as const).map((kind) => (
                <button
                  key={kind}
                  type="button"
                  className="quiet-action"
                  onClick={() => setCreating(kind)}
                >
                  + {KIND_LABELS[kind]}
                </button>
              ))}
            </div>
          </div>

          {creating ? (
            <CreateModelForm
              kind={creating}
              onCancel={() => setCreating(null)}
              onCreated={(modelId, kind) => {
                void navigate(editorPath(kind, modelId)!);
              }}
              onError={(error) => reportError(error, 'Unable to create the model')}
            />
          ) : null}

          {page.state === 'loading' ? <p className="model-list-hint">Loading models…</p> : null}
          {page.state === 'error' ? (
            <p className="model-list-hint" role="alert">
              {page.message}
            </p>
          ) : null}
          {page.state === 'ready' && page.models.length === 0 ? (
            <p className="model-list-hint">
              No models yet. Create a BPMN, DMN, or form model to get started.
            </p>
          ) : null}
          {page.state === 'ready' && page.models.length > 0 ? (
            <table className="model-list-table" aria-label="Model list table">
              <thead>
                <tr>
                  <th scope="col">Thumbnail</th>
                  <th scope="col">Name</th>
                  <th scope="col">Type</th>
                  <th scope="col">Key</th>
                  <th scope="col">Last updated</th>
                  <th scope="col">Actions</th>
                </tr>
              </thead>
              <tbody>
                {page.models.map((entry) => (
                  <tr key={entry.id} data-model-id={entry.id}>
                    <td className="model-list-thumb-cell">
                      {entry.kind === 'bpmn' ? (
                        <img
                          className="model-list-thumb"
                          src={`/modeler-app/rest/models/${encodeURIComponent(entry.id)}/thumbnail`}
                          alt=""
                          width={72}
                          height={40}
                          loading="lazy"
                        />
                      ) : (
                        <span className="model-list-thumb is-placeholder" aria-hidden="true" />
                      )}
                    </td>
                    <td>
                      <button
                        type="button"
                        className="model-list-open"
                        disabled={!editorPath(entry.kind, entry.id)}
                        onClick={() => openModel(entry)}
                      >
                        {entry.name ?? entry.key}
                      </button>
                    </td>
                    <td>
                      <span className={`model-kind-badge is-${entry.kind}`}>
                        {KIND_LABELS[entry.kind]}
                      </span>
                    </td>
                    <td className="model-list-key">{entry.key}</td>
                    <td>{formatTimestamp(entry.lastUpdateTime)}</td>
                    <td>
                      <ModelRowActions
                        entry={entry}
                        onClone={() => void duplicateModel(entry)}
                        onDelete={() => void removeModel(entry)}
                        onPublish={() => void publishModel(entry)}
                      />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : null}
        </section>
      </div>
    </main>
  );
}

/**
 * The per-row action buttons. Every label names the model because the row
 * repeats them: three bare "Publish" buttons on a page are indistinguishable to
 * a screen reader. Publish needs a known kind to pick a deployment endpoint;
 * clone and delete work on the stored bytes and stay available regardless.
 */
export function ModelRowActions({
  entry,
  onClone,
  onDelete,
  onPublish,
}: {
  entry: ModelEntry;
  onClone: () => void;
  onDelete: () => void;
  onPublish: () => void;
}) {
  const label = entry.name ?? entry.key;
  return (
    <span className="model-list-actions">
      <button
        type="button"
        className="quiet-action"
        aria-label={`Publish model ${label}`}
        disabled={entry.kind === 'unknown'}
        onClick={onPublish}
      >
        Publish
      </button>
      <button
        type="button"
        className="quiet-action"
        aria-label={`Clone model ${label}`}
        onClick={onClone}
      >
        Clone
      </button>
      <button
        type="button"
        className="quiet-action is-danger"
        aria-label={`Delete model ${label}`}
        onClick={onDelete}
      >
        Delete
      </button>
    </span>
  );
}

function CreateModelForm({
  kind,
  onCancel,
  onCreated,
  onError,
}: {
  kind: Exclude<ModelKind, 'unknown'>;
  onCancel: () => void;
  onCreated: (modelId: string, kind: Exclude<ModelKind, 'unknown'>) => void;
  onError: (error: unknown) => void;
}) {
  const [name, setName] = useState('');
  const [key, setKey] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    const trimmedName = name.trim();
    const trimmedKey = key.trim() || slugify(trimmedName);
    if (!trimmedName) {
      setError('Name must not be blank');
      return;
    }
    if (!trimmedKey) {
      setError('Key must not be blank');
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const model = await createModel({ name: trimmedName, key: trimmedKey });
      await updateModelSource(model.id, stubContentType(kind), stubSource(kind, trimmedKey, trimmedName));
      onCreated(model.id, kind);
    } catch (createError) {
      setBusy(false);
      onError(createError);
    }
  };

  return (
    <form
      className="model-create-form"
      aria-label={`Create ${KIND_LABELS[kind]} model`}
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <span className={`model-kind-badge is-${kind}`}>{KIND_LABELS[kind]}</span>
      <input
        type="text"
        aria-label="Model name"
        placeholder="Name"
        value={name}
        onChange={(event) => setName(event.target.value)}
      />
      <input
        type="text"
        aria-label="Model key"
        placeholder="Key (derived from name)"
        value={key}
        onChange={(event) => setKey(event.target.value)}
      />
      <button type="submit" className="primary-button" disabled={busy}>
        {busy ? 'Creating…' : 'Create'}
      </button>
      <button type="button" className="quiet-button" onClick={onCancel}>
        Cancel
      </button>
      {error ? (
        <span className="property-error" role="alert">
          {error}
        </span>
      ) : null}
    </form>
  );
}

function slugify(value: string): string {
  return value
    .trim()
    .replace(/[^a-zA-Z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .toLowerCase();
}

function formatTimestamp(value: string | null): string {
  if (!value) return '—';
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}
