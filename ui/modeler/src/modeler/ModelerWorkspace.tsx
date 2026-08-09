import { useEffect } from 'react';

import type { FlowElementEnum } from '../generated/editor-protocol';
import { BpmnCanvas } from './BpmnCanvas';
import { createElementCommand, deleteElementsCommand } from './commands';
import { documentElements } from './diagramModel';
import {
  createPaletteElement,
  defaultElementSize,
  type PaletteElementKind,
} from './elementFactory';
import { useModelerStore } from './modelerStore';

const palette = [
  ['Event', '○', 'event'],
  ['Task', '▢', 'task'],
  ['Gateway', '◇', 'gateway'],
  ['Subprocess', '▣', 'subprocess'],
  ['Data', '⌑', 'data'],
] as const;

export function ModelerWorkspace() {
  const document = useModelerStore((state) => state.document);
  const viewport = useModelerStore((state) => state.viewport);
  const selectedElementId = useModelerStore((state) => state.selectedElementId);
  const zoomBy = useModelerStore((state) => state.zoomBy);
  const resetViewport = useModelerStore((state) => state.resetViewport);
  const undoStack = useModelerStore((state) => state.undoStack);
  const redoStack = useModelerStore((state) => state.redoStack);
  const undo = useModelerStore((state) => state.undo);
  const redo = useModelerStore((state) => state.redo);
  const execute = useModelerStore((state) => state.execute);
  const selectElement = useModelerStore((state) => state.selectElement);
  const process = document.model.processes[0];
  const elements = documentElements(document);
  const selectedElement = elements.find((element) => element.id === selectedElementId);

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
      if ((event.key === 'Delete' || event.key === 'Backspace') && selectedElementId) {
        event.preventDefault();
        execute(deleteElementsCommand([selectedElementId]));
        selectElement(null);
        return;
      }
      if (!(event.ctrlKey || event.metaKey)) return;
      if (event.key.toLowerCase() === 'z') {
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
  }, [execute, redo, selectElement, selectedElementId, undo]);

  const createElement = (kind: PaletteElementKind) => {
    const id = nextElementId(elements, kind);
    const element = createPaletteElement(kind, id);
    const size = defaultElementSize(kind);
    const creationIndex = elements.filter((item) => item.id?.startsWith(`modeler-${kind}`)).length;
    execute(
      createElementCommand(element, {
        x: 220 + (creationIndex % 4) * 190,
        y: 260 + Math.floor(creationIndex / 4) * 140,
        ...size,
        rotation: 0,
        expanded: true,
        xmlRowNumber: 0,
        xmlColumnNumber: 0,
      }),
    );
    selectElement(id);
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

      <div className="modeler-layout">
        <aside className="palette-panel" aria-label="BPMN element palette">
          <div className="panel-kicker">Elements</div>
          <div className="palette-list">
            {palette.map(([label, glyph, kind]) => (
              <button
                key={label}
                type="button"
                title={`Create ${label.toLowerCase()}`}
                onClick={() => createElement(kind)}
              >
                <span aria-hidden="true">{glyph}</span>
                {label}
              </button>
            ))}
          </div>
          <div className="palette-hint">
            Click to create. Drag-to-place is the next interaction increment.
          </div>
        </aside>

        <section className="canvas-workspace" aria-label="Process editor workspace">
          <div className="canvas-toolbar" role="toolbar" aria-label="Canvas controls">
            <div className="tool-cluster">
              <button type="button" aria-label="Pointer tool" className="is-active">
                ↖
              </button>
              <button type="button" aria-label="Hand tool">
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
                disabled={!selectedElementId}
                onClick={() => {
                  if (!selectedElementId) return;
                  execute(deleteElementsCommand([selectedElementId]));
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
              <button type="button" onClick={resetViewport}>
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

        <aside className="properties-panel" aria-label="Element properties">
          <div className="properties-heading">
            <div>
              <span className="panel-kicker">Selection</span>
              <h1>{selectedElement?.name ?? 'Process'}</h1>
            </div>
            <span className="selection-glyph" aria-hidden="true">
              {selectedElement ? elementGlyph(selectedElement) : '◎'}
            </span>
          </div>
          {selectedElement ? <ElementSummary element={selectedElement} /> : <ProcessSummary />}
        </aside>
      </div>
    </main>
  );
}

function ElementSummary({ element }: { element: FlowElementEnum }) {
  return (
    <div className="property-groups">
      <section>
        <h2>General</h2>
        <Property label="ID" value={element.id ?? '—'} code />
        <Property label="Type" value={humanize(element.elementType)} />
        <Property label="Name" value={element.name ?? '—'} />
      </section>
      <section>
        <h2>Document geometry</h2>
        <Property label="Source row" value={String(element.xmlRowNumber)} />
        <Property label="Source column" value={String(element.xmlColumnNumber)} />
      </section>
      <section className="read-only-note">
        <span>Read-only preview</span>
        <p>
          Property editing is introduced in M3. This panel already resolves the typed canonical
          element.
        </p>
      </section>
    </div>
  );
}

function ProcessSummary() {
  return (
    <div className="empty-properties">
      <span>Nothing selected</span>
      <p>Select a BPMN node to inspect its canonical JSON properties.</p>
    </div>
  );
}

function Property({
  label,
  value,
  code = false,
}: {
  label: string;
  value: string;
  code?: boolean;
}) {
  return (
    <div className="property-row">
      <span>{label}</span>
      <strong className={code ? 'code-value' : undefined}>{value}</strong>
    </div>
  );
}

function elementGlyph(element: FlowElementEnum) {
  if (element.elementType.includes('Event')) return '○';
  if (element.elementType.includes('Gateway')) return '◇';
  if (element.elementType.includes('Task')) return '▢';
  return '▣';
}

function humanize(value: string) {
  return value.replace(/([a-z])([A-Z])/g, '$1 $2').replace(/^./, (letter) => letter.toUpperCase());
}

function nextElementId(elements: FlowElementEnum[], kind: PaletteElementKind) {
  const existing = new Set(elements.flatMap((element) => (element.id ? [element.id] : [])));
  let suffix = 1;
  while (existing.has(`modeler-${kind}-${suffix}`)) suffix += 1;
  return `modeler-${kind}-${suffix}`;
}
