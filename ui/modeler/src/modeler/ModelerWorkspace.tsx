import { useEffect, useState } from 'react';
import { Link, useParams } from 'react-router-dom';

import { BpmnCanvas } from './BpmnCanvas';
import { deleteElementsCommand } from './commands';
import { BPMN_PALETTE_MIME, createAtPointCommand, nextPaletteElementId } from './creationCommands';
import { documentElements } from './diagramModel';
import type { CanonicalPaletteElementKind } from './elementFactory';
import { useModelerStore } from './modelerStore';
import { loadBpmnDocument, saveBpmnDocument } from './modelerApi';
import { PropertiesPanel } from './PropertiesPanel';

const palette = [
  ['Start', '○', 'start'],
  ['End', '◉', 'end'],
  ['User task', '▢', 'userTask'],
  ['Gateway', '◇', 'exclusiveGateway'],
  ['Subprocess', '▣', 'subprocess'],
  ['Timer boundary', '◷', 'boundaryTimer'],
  ['Data', '⌑', 'data'],
] as const;

/**
 * Real repository model ids load and save through the editor endpoint.
 * The reserved `sample` id keeps the in-memory demo document offline.
 */
function isPersistableModelId(modelId: string | undefined): modelId is string {
  return Boolean(modelId && modelId !== 'sample');
}

export function ModelerWorkspace() {
  const { modelId } = useParams<{ modelId: string }>();
  const document = useModelerStore((state) => state.document);
  const viewport = useModelerStore((state) => state.viewport);
  const tool = useModelerStore((state) => state.tool);
  const selectedElementIds = useModelerStore((state) => state.selectedElementIds);
  const selectedElementId = useModelerStore((state) => state.selectedElementId);
  const setTool = useModelerStore((state) => state.setTool);
  const zoomBy = useModelerStore((state) => state.zoomBy);
  const fitToModel = useModelerStore((state) => state.fitToModel);
  const undoStack = useModelerStore((state) => state.undoStack);
  const redoStack = useModelerStore((state) => state.redoStack);
  const undo = useModelerStore((state) => state.undo);
  const redo = useModelerStore((state) => state.redo);
  const execute = useModelerStore((state) => state.execute);
  const selectElement = useModelerStore((state) => state.selectElement);
  const setDocument = useModelerStore((state) => state.setDocument);
  const copySelection = useModelerStore((state) => state.copySelection);
  const pasteClipboard = useModelerStore((state) => state.pasteClipboard);
  const [persistence, setPersistence] = useState<
    | { state: 'idle' }
    | { state: 'loading' | 'saving' }
    | { state: 'saved'; message: string }
    | { state: 'error'; message: string }
  >(isPersistableModelId(modelId) ? { state: 'loading' } : { state: 'idle' });
  const process = document.model.processes[0];
  const elements = documentElements(document);
  const canPersist = isPersistableModelId(modelId);

  useEffect(() => {
    if (!isPersistableModelId(modelId)) return;
    let active = true;
    void loadBpmnDocument(modelId)
      .then((loaded) => {
        if (!active) return;
        setDocument(loaded);
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
  }, [modelId, setDocument]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const target = event.target;
      if (
        target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement ||
        (target instanceof HTMLElement && target.isContentEditable)
      ) {
        return;
      }
      if ((event.key === 'Delete' || event.key === 'Backspace') && selectedElementIds.length) {
        event.preventDefault();
        execute(deleteElementsCommand(selectedElementIds));
        selectElement(null);
        return;
      }
      if (!(event.ctrlKey || event.metaKey)) return;
      if (event.key.toLowerCase() === 'c') {
        event.preventDefault();
        copySelection();
      } else if (event.key.toLowerCase() === 'v') {
        event.preventDefault();
        pasteClipboard();
      } else if (event.key.toLowerCase() === 'z') {
        event.preventDefault();
        if (event.shiftKey) redo();
        else undo();
      } else if (event.key.toLowerCase() === 'y') {
        event.preventDefault();
        redo();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [copySelection, execute, pasteClipboard, redo, selectElement, selectedElementIds, undo]);

  const createElement = (kind: CanonicalPaletteElementKind) => {
    const id = nextPaletteElementId(document, kind);
    const creationIndex = elements.filter((item) => item.id?.startsWith(`modeler-${kind}`)).length;
    const point = paletteClickPoint(document, kind, creationIndex, selectedElementId);
    execute(createAtPointCommand(kind, id, point));
    if (useModelerStore.getState().document.model.locationMap[id]) selectElement(id);
  };

  const save = async () => {
    if (!canPersist || !modelId || persistence.state === 'saving') return;
    setPersistence({ state: 'saving' });
    try {
      const normalized = await saveBpmnDocument(modelId, useModelerStore.getState().document);
      setDocument(normalized);
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
          <span className="document-kind">BPMN 2.0</span>
          <strong>{process?.name ?? 'Untitled process'}</strong>
          <span className="save-state">
            <i aria-hidden="true" /> {undoStack.length ? 'Local changes' : 'Local draft ready'}
          </span>
        </div>
        <div className="topbar-actions">
          <Link className="quiet-button" to="/">
            Back to list
          </Link>
          {canPersist ? (
            <button
              type="button"
              className="quiet-button"
              disabled={persistence.state === 'loading' || persistence.state === 'saving'}
              onClick={() => void save()}
            >
              {persistence.state === 'saving' ? 'Saving…' : 'Save'}
            </button>
          ) : null}
          <button type="button" className="quiet-button">
            Validate
          </button>
          <button type="button" className="primary-button">
            Publish
          </button>
          <button type="button" className="avatar-button" aria-label="Account menu">
            AD
          </button>
        </div>
      </header>

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

      <div className="modeler-layout">
        <aside className="palette-panel" aria-label="BPMN element palette">
          <div className="panel-kicker">Elements</div>
          <div className="palette-list">
            {palette.map(([label, glyph, kind]) => (
              <button
                key={label}
                type="button"
                title={`Create ${label.toLowerCase()}`}
                draggable
                data-palette-kind={kind}
                onDragStart={(event) => {
                  event.dataTransfer.effectAllowed = 'copy';
                  event.dataTransfer.setData(BPMN_PALETTE_MIME, kind);
                }}
                onClick={() => createElement(kind)}
              >
                <span aria-hidden="true">{glyph}</span>
                {label}
              </button>
            ))}
          </div>
          <div className="palette-hint">
            Click for a guided placement, or drag an element onto the canvas.
          </div>
        </aside>

        <section className="canvas-workspace" aria-label="Process editor workspace">
          <div className="canvas-toolbar" role="toolbar" aria-label="Canvas controls">
            <div className="tool-cluster">
              <button
                type="button"
                aria-label="Pointer tool"
                className={tool === 'pointer' ? 'is-active' : undefined}
                onClick={() => setTool('pointer')}
              >
                ↖
              </button>
              <button
                type="button"
                aria-label="Hand tool"
                className={tool === 'hand' ? 'is-active' : undefined}
                onClick={() => setTool('hand')}
              >
                ✥
              </button>
              <span className="tool-divider" />
              <button
                type="button"
                aria-label="Undo"
                title={undoStack.at(-1)?.label}
                disabled={undoStack.length === 0}
                onClick={undo}
              >
                ↶
              </button>
              <button
                type="button"
                aria-label="Redo"
                title={redoStack.at(-1)?.label}
                disabled={redoStack.length === 0}
                onClick={redo}
              >
                ↷
              </button>
              <button
                type="button"
                aria-label="Delete selection"
                disabled={selectedElementIds.length === 0}
                onClick={() => {
                  if (!selectedElementIds.length) return;
                  execute(deleteElementsCommand(selectedElementIds));
                  selectElement(null);
                }}
              >
                ⌫
              </button>
            </div>
            <div className="tool-cluster zoom-controls">
              <button type="button" aria-label="Zoom out" onClick={() => zoomBy(0.9)}>
                −
              </button>
              <output aria-label="Zoom level">{Math.round(viewport.zoom * 100)}%</output>
              <button type="button" aria-label="Zoom in" onClick={() => zoomBy(1.1)}>
                +
              </button>
              <button type="button" onClick={fitToModel}>
                Fit
              </button>
            </div>
          </div>
          <BpmnCanvas />
          <div className="canvas-statusbar">
            <span>
              <i className="status-dot" /> Protocol {document.schemaVersion}
            </span>
            <span>
              {elements.filter((element) => element.elementType !== 'sequenceFlow').length} elements
            </span>
            <span>{Object.keys(document.model.locationMap).length} DI bounds</span>
          </div>
        </section>

        <PropertiesPanel />
      </div>
    </main>
  );
}

function paletteClickPoint(
  document: ReturnType<typeof useModelerStore.getState>['document'],
  kind: CanonicalPaletteElementKind,
  creationIndex: number,
  selectedElementId: string | null,
) {
  if (kind === 'boundaryTimer' && selectedElementId) {
    const selectedBounds = document.model.locationMap[selectedElementId];
    if (selectedBounds) {
      return {
        x: selectedBounds.x + selectedBounds.width,
        y: selectedBounds.y + selectedBounds.height / 2,
      };
    }
  }
  const pool = document.model.pools
    .map((candidate) => (candidate.id ? document.model.locationMap[candidate.id] : undefined))
    .find((bounds) => bounds !== undefined);
  const origin = pool ? { x: pool.x + 120, y: pool.y + 100 } : { x: 240, y: 220 };
  return {
    x: origin.x + (creationIndex % 4) * 170,
    y: origin.y + Math.floor(creationIndex / 4) * 120,
  };
}
