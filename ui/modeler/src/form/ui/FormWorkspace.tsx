import { useEffect, useMemo, useState } from 'react';
import { Link, useParams } from 'react-router-dom';
import { useStore } from 'zustand';

import { loadFormDocument, saveFormDocument } from '../../modeler/modelerApi';
import { createFormEditorStore, validateFormDocument, type FormEditorStore } from '../index';
import { FormCanvas } from './FormCanvas';
import { FormPalette } from './FormPalette';
import { FormPreview } from './FormPreview';
import { FormPropertiesPanel } from './FormPropertiesPanel';

type PersistenceState =
  | { state: 'idle' }
  | { state: 'loading' | 'saving' }
  | { state: 'saved'; message: string }
  | { state: 'error'; message: string };

export function FormWorkspace() {
  const { modelId } = useParams<{ modelId: string }>();
  const [store, setStore] = useState<FormEditorStore | null>(null);
  const [persistence, setPersistence] = useState<PersistenceState>({ state: 'loading' });

  useEffect(() => {
    if (!modelId) return;
    let active = true;
    void loadFormDocument(modelId)
      .then((loaded) => {
        if (!active) return;
        setStore(createFormEditorStore(loaded));
        setPersistence({ state: 'idle' });
      })
      .catch((error: unknown) => {
        if (!active) return;
        setPersistence({
          state: 'error',
          message: error instanceof Error ? error.message : 'Unable to load this model',
        });
      });
    return () => {
      active = false;
    };
  }, [modelId]);

  const save = async () => {
    if (!modelId || !store || persistence.state === 'saving') return;
    setPersistence({ state: 'saving' });
    try {
      const normalized = await saveFormDocument(modelId, store.getState().document);
      store.getState().setDocument(normalized);
      setPersistence({ state: 'saved', message: 'Saved and reloaded from the server' });
    } catch (error) {
      setPersistence({
        state: 'error',
        message: error instanceof Error ? error.message : 'Unable to save this model',
      });
    }
  };

  return (
    <main className="modeler-shell">
      {store ? (
        <FormWorkspaceBody
          key={modelId ?? 'form'}
          persistence={persistence}
          save={() => void save()}
          store={store}
        />
      ) : (
        <header className="modeler-topbar">
          <Wordmark />
          <div className="model-title-block">
            <span className="document-kind">Form</span>
            <strong>Loading form…</strong>
          </div>
        </header>
      )}
      {persistence.state === 'error' ? (
        <div className="modeler-notice is-error" role="alert">
          {persistence.message}
        </div>
      ) : null}
      {persistence.state === 'saved' ? (
        <div className="modeler-notice" role="status">
          {persistence.message}
        </div>
      ) : null}
    </main>
  );
}

function FormWorkspaceBody({
  persistence,
  save,
  store,
}: {
  persistence: PersistenceState;
  save: () => void;
  store: FormEditorStore;
}) {
  const document = useStore(store, (state) => state.document);
  const undoStack = useStore(store, (state) => state.undoStack);
  const redoStack = useStore(store, (state) => state.redoStack);
  const [mode, setMode] = useState<'design' | 'preview'>('design');
  const issues = useMemo(() => validateFormDocument(document), [document]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const target = event.target;
      if (
        target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement ||
        target instanceof HTMLSelectElement ||
        (target instanceof HTMLElement && target.isContentEditable)
      ) {
        return;
      }
      if (!(event.ctrlKey || event.metaKey)) return;
      if (event.key.toLowerCase() === 'z') {
        event.preventDefault();
        if (event.shiftKey) store.getState().redo();
        else store.getState().undo();
      } else if (event.key.toLowerCase() === 'y') {
        event.preventDefault();
        store.getState().redo();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [store]);

  return (
    <>
      <header className="modeler-topbar">
        <Wordmark />
        <div className="model-title-block">
          <span className="document-kind">Form</span>
          <strong>{document.model.name || 'Untitled form'}</strong>
          <span className="save-state">
            <i aria-hidden="true" /> {undoStack.length ? 'Local changes' : 'Local draft ready'}
          </span>
        </div>
        <div className="topbar-actions">
          <Link className="quiet-button" to="/">
            Back to list
          </Link>
          <button
            type="button"
            className="quiet-button"
            aria-pressed={mode === 'preview'}
            onClick={() => setMode(mode === 'design' ? 'preview' : 'design')}
          >
            {mode === 'design' ? 'Preview' : 'Design'}
          </button>
          <button
            type="button"
            className="quiet-button"
            disabled={persistence.state === 'loading' || persistence.state === 'saving'}
            onClick={save}
          >
            {persistence.state === 'saving' ? 'Saving…' : 'Save'}
          </button>
          <button type="button" className="avatar-button" aria-label="Account menu">
            AD
          </button>
        </div>
      </header>

      {mode === 'design' ? (
        <div className="modeler-layout">
          <FormPalette store={store} />
          <section className="canvas-workspace" aria-label="Form designer workspace">
            <div className="canvas-toolbar" role="toolbar" aria-label="Form designer controls">
              <div className="tool-cluster">
                <button
                  type="button"
                  aria-label="Undo"
                  title={undoStack.at(-1)?.label}
                  disabled={undoStack.length === 0}
                  onClick={() => store.getState().undo()}
                >
                  ↶
                </button>
                <button
                  type="button"
                  aria-label="Redo"
                  title={redoStack.at(-1)?.label}
                  disabled={redoStack.length === 0}
                  onClick={() => store.getState().redo()}
                >
                  ↷
                </button>
              </div>
            </div>
            <FormCanvas issues={issues} store={store} />
            <div className="canvas-statusbar">
              <span>
                <i className="status-dot" /> Protocol {document.schemaVersion}
              </span>
              <span>{document.model.fields?.length ?? 0} fields</span>
              <span>{document.model.outcomes?.length ?? 0} outcomes</span>
              <span className={issues.length ? 'status-issues' : undefined}>
                {issues.length ? `${issues.length} validation issues` : 'Valid'}
              </span>
            </div>
          </section>
          <FormPropertiesPanel issues={issues} store={store} />
        </div>
      ) : (
        <FormPreview model={document.model} />
      )}
    </>
  );
}

function Wordmark() {
  return (
    <a className="wordmark" href="/modeler-app/" aria-label="Flowable Modeler home">
      <span className="wordmark-mark" aria-hidden="true">
        F
      </span>
      <span>
        <strong>Flowable</strong>
        <small>Modeler</small>
      </span>
    </a>
  );
}
