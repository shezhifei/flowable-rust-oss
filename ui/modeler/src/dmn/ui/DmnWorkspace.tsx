import { useEffect, useState } from 'react';
import { Link, useParams } from 'react-router-dom';
import { useStore } from 'zustand';

import { loadDmnDocument, saveDmnDocument } from '../../modeler/modelerApi';
import { createDmnEditorStore, type DmnEditorStore } from '../index';
import { DecisionTableEditor, type DmnTableSelection } from './DecisionTableEditor';
import { DmnPropertiesPanel } from './DmnPropertiesPanel';

type PersistenceState =
  | { state: 'idle' }
  | { state: 'loading' | 'saving' }
  | { state: 'saved'; message: string }
  | { state: 'error'; message: string };

export function DmnWorkspace() {
  const { modelId } = useParams<{ modelId: string }>();
  const [store, setStore] = useState<DmnEditorStore | null>(null);
  const [persistence, setPersistence] = useState<PersistenceState>({ state: 'loading' });

  useEffect(() => {
    if (!modelId) return;
    let active = true;
    void loadDmnDocument(modelId)
      .then((loaded) => {
        if (!active) return;
        setStore(createDmnEditorStore(loaded));
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
      const normalized = await saveDmnDocument(modelId, store.getState().document);
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
        <DmnWorkspaceBody
          key={modelId ?? 'dmn'}
          persistence={persistence}
          save={() => void save()}
          store={store}
        />
      ) : (
        <header className="modeler-topbar">
          <Wordmark />
          <div className="model-title-block">
            <span className="document-kind">DMN 1.3</span>
            <strong>Loading decision table…</strong>
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

function DmnWorkspaceBody({
  persistence,
  save,
  store,
}: {
  persistence: PersistenceState;
  save: () => void;
  store: DmnEditorStore;
}) {
  const document = useStore(store, (state) => state.document);
  const selectedDecisionId = useStore(store, (state) => state.selectedDecisionId);
  const undoStack = useStore(store, (state) => state.undoStack);
  const redoStack = useStore(store, (state) => state.redoStack);
  const [selection, setSelection] = useState<DmnTableSelection>(null);

  const decisions = document.model.decisions ?? [];
  const decisionId = selectedDecisionId ?? decisions[0]?.id ?? null;
  const decision = decisions.find((candidate) => candidate.id === decisionId) ?? null;

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
          <span className="document-kind">DMN 1.3</span>
          <strong>{decision?.name ?? decision?.id ?? 'Untitled decision'}</strong>
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
            disabled={persistence.state === 'loading' || persistence.state === 'saving'}
            onClick={save}
          >
            {persistence.state === 'saving' ? 'Saving…' : 'Save'}
          </button>
          <button type="button" className="primary-button">
            Publish
          </button>
          <button type="button" className="avatar-button" aria-label="Account menu">
            AD
          </button>
        </div>
      </header>

      <div className="modeler-layout">
        <aside className="palette-panel" aria-label="Decision list">
          <div className="panel-kicker">Decisions</div>
          <div className="palette-list">
            {decisions.map((candidate) => (
              <button
                key={candidate.id}
                type="button"
                className={candidate.id === decisionId ? 'is-active' : undefined}
                aria-pressed={candidate.id === decisionId}
                onClick={() => {
                  store.getState().selectDecision(candidate.id);
                  setSelection(null);
                }}
              >
                <span aria-hidden="true">▤</span>
                {candidate.name ?? candidate.id}
              </button>
            ))}
          </div>
          <div className="palette-hint">
            Click a column header to edit it, or a cell to edit its expression.
          </div>
        </aside>

        <section className="canvas-workspace" aria-label="Decision table editor workspace">
          <div className="canvas-toolbar" role="toolbar" aria-label="Decision table controls">
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
          {decisionId ? (
            <DecisionTableEditor
              decisionId={decisionId}
              onSelect={setSelection}
              selection={selection}
              store={store}
            />
          ) : (
            <div className="dmn-table-missing">This document does not contain a decision.</div>
          )}
          <div className="canvas-statusbar">
            <span>
              <i className="status-dot" /> Protocol {document.schemaVersion}
            </span>
            <span>{decisions.length} decisions</span>
            <span>{decision?.decisionTable.rules?.length ?? 0} rules</span>
          </div>
        </section>

        {decisionId ? (
          <DmnPropertiesPanel
            decisionId={decisionId}
            onSelect={setSelection}
            selection={selection}
            store={store}
          />
        ) : (
          <aside className="properties-panel" aria-label="Decision table properties" />
        )}
      </div>
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
