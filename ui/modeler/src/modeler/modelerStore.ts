import { applyPatches, enablePatches, produceWithPatches, type Patch } from 'immer';
import { create } from 'zustand';

import type { BpmnEditorDocument } from '../generated/editor-protocol';
import type { ModelerCommand } from './commands';
import { sampleDocument } from './sampleDocument';

enablePatches();

const MIN_ZOOM = 0.35;
const MAX_ZOOM = 2.5;
const MAX_HISTORY = 100;

interface ViewportState {
  x: number;
  y: number;
  zoom: number;
}

interface HistoryEntry {
  label: string;
  patches: Patch[];
  inversePatches: Patch[];
}

interface ModelerState {
  document: BpmnEditorDocument;
  viewport: ViewportState;
  selectedElementId: string | null;
  undoStack: HistoryEntry[];
  redoStack: HistoryEntry[];
  setDocument: (document: BpmnEditorDocument) => void;
  execute: (command: ModelerCommand) => void;
  undo: () => void;
  redo: () => void;
  selectElement: (elementId: string | null) => void;
  panBy: (deltaX: number, deltaY: number) => void;
  zoomBy: (factor: number) => void;
  resetViewport: () => void;
}

const initialViewport: ViewportState = { x: 16, y: 18, zoom: 0.82 };

export const useModelerStore = create<ModelerState>((set, get) => ({
  document: sampleDocument,
  viewport: { ...initialViewport },
  selectedElementId: 'review',
  undoStack: [],
  redoStack: [],
  setDocument: (document) =>
    set({ document, selectedElementId: null, undoStack: [], redoStack: [] }),
  execute: (command) => {
    const state = get();
    const [document, patches, inversePatches] = produceWithPatches(state.document, command.apply);
    if (patches.length === 0) return;
    const entry = { label: command.label, patches, inversePatches };
    set({
      document,
      undoStack: [...state.undoStack, entry].slice(-MAX_HISTORY),
      redoStack: [],
    });
  },
  undo: () => {
    const state = get();
    const entry = state.undoStack.at(-1);
    if (!entry) return;
    set({
      document: applyPatches(state.document, entry.inversePatches),
      undoStack: state.undoStack.slice(0, -1),
      redoStack: [...state.redoStack, entry],
    });
  },
  redo: () => {
    const state = get();
    const entry = state.redoStack.at(-1);
    if (!entry) return;
    set({
      document: applyPatches(state.document, entry.patches),
      undoStack: [...state.undoStack, entry].slice(-MAX_HISTORY),
      redoStack: state.redoStack.slice(0, -1),
    });
  },
  selectElement: (selectedElementId) => set({ selectedElementId }),
  panBy: (deltaX, deltaY) =>
    set((state) => ({
      viewport: { ...state.viewport, x: state.viewport.x + deltaX, y: state.viewport.y + deltaY },
    })),
  zoomBy: (factor) =>
    set((state) => ({
      viewport: {
        ...state.viewport,
        zoom: Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, state.viewport.zoom * factor)),
      },
    })),
  resetViewport: () => set({ viewport: { ...initialViewport } }),
}));
