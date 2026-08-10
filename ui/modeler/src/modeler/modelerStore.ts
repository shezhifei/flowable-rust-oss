import { applyPatches, enablePatches, produceWithPatches, type Patch } from 'immer';
import { create } from 'zustand';

import type { BpmnEditorDocument } from '../generated/editor-protocol';
import {
  copySelection as createClipboardSlice,
  pasteClipboardCommand,
  type BpmnClipboardSlice,
} from './clipboardCommands';
import type { ModelerCommand } from './commands';
import { sampleDocument } from './sampleDocument';

enablePatches();

const MIN_ZOOM = 0.35;
const MAX_ZOOM = 2.5;
const MAX_FIT_ZOOM = 1.05;
const MAX_HISTORY = 100;
const CANVAS_WIDTH = 1400;
const CANVAS_HEIGHT = 620;
const FIT_PADDING = 42;

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

export type EditorTool = 'pointer' | 'hand' | 'connect';

interface ModelerState {
  document: BpmnEditorDocument;
  viewport: ViewportState;
  tool: EditorTool;
  selectedElementIds: string[];
  selectedElementId: string | null;
  clipboard: BpmnClipboardSlice | null;
  undoStack: HistoryEntry[];
  redoStack: HistoryEntry[];
  setDocument: (document: BpmnEditorDocument) => void;
  execute: (command: ModelerCommand) => void;
  undo: () => void;
  redo: () => void;
  selectElement: (elementId: string | null, additive?: boolean) => void;
  selectElements: (elementIds: string[]) => void;
  setTool: (tool: EditorTool) => void;
  copySelection: () => void;
  pasteClipboard: () => void;
  panBy: (deltaX: number, deltaY: number) => void;
  zoomBy: (factor: number) => void;
  fitToModel: () => void;
  resetViewport: () => void;
}

declare global {
  interface Window {
    __FLOWABLE_MODELER_TEST__?: {
      setDocument: (document: BpmnEditorDocument) => void;
      getDocument: () => BpmnEditorDocument;
    };
  }
}

const initialViewport: ViewportState = { x: 16, y: 18, zoom: 0.82 };

export const useModelerStore = create<ModelerState>((set, get) => ({
  document: sampleDocument,
  viewport: { ...initialViewport },
  tool: 'pointer',
  selectedElementIds: ['review'],
  selectedElementId: 'review',
  clipboard: null,
  undoStack: [],
  redoStack: [],
  setDocument: (document) =>
    set({
      document,
      selectedElementIds: [],
      selectedElementId: null,
      undoStack: [],
      redoStack: [],
    }),
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
  selectElement: (elementId, additive = false) =>
    set((state) => {
      if (!elementId) return { selectedElementIds: [], selectedElementId: null };
      if (!additive) return { selectedElementIds: [elementId], selectedElementId: elementId };
      const selected = state.selectedElementIds.includes(elementId)
        ? state.selectedElementIds.filter((id) => id !== elementId)
        : [...state.selectedElementIds, elementId];
      return {
        selectedElementIds: selected,
        selectedElementId: selected.at(-1) ?? null,
      };
    }),
  selectElements: (elementIds) => {
    const selectedElementIds = [...new Set(elementIds)];
    set({ selectedElementIds, selectedElementId: selectedElementIds.at(-1) ?? null });
  },
  setTool: (tool) => set({ tool }),
  copySelection: () => {
    const state = get();
    const clipboard = createClipboardSlice(state.document, state.selectedElementIds);
    if (clipboard) set({ clipboard });
  },
  pasteClipboard: () => {
    const clipboard = get().clipboard;
    if (!clipboard) return;
    get().execute(pasteClipboardCommand(clipboard));
  },
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
  fitToModel: () =>
    set((state) => {
      const bounds = modelBounds(state.document);
      if (!bounds) return { viewport: { ...initialViewport } };
      const width = Math.max(1, bounds.maxX - bounds.minX);
      const height = Math.max(1, bounds.maxY - bounds.minY);
      const zoom = Math.min(
        MAX_FIT_ZOOM,
        Math.max(
          MIN_ZOOM,
          Math.min(
            (CANVAS_WIDTH - FIT_PADDING * 2) / width,
            (CANVAS_HEIGHT - FIT_PADDING * 2) / height,
          ),
        ),
      );
      return {
        viewport: {
          zoom,
          x: FIT_PADDING - bounds.minX * zoom,
          y: FIT_PADDING - bounds.minY * zoom,
        },
      };
    }),
  resetViewport: () => set({ viewport: { ...initialViewport } }),
}));

if (import.meta.env.MODE === 'e2e' && typeof window !== 'undefined') {
  window.__FLOWABLE_MODELER_TEST__ = {
    setDocument: (document) => {
      useModelerStore.getState().setDocument(document);
      useModelerStore.getState().fitToModel();
    },
    getDocument: () => useModelerStore.getState().document,
  };
}

function modelBounds(document: BpmnEditorDocument) {
  const points = [
    ...Object.values(document.model.locationMap).flatMap((bounds) => [
      { x: bounds.x, y: bounds.y },
      { x: bounds.x + bounds.width, y: bounds.y + bounds.height },
    ]),
    ...Object.values(document.model.flowLocationMap).flatMap((waypoints) =>
      waypoints.map((point) => ({ x: point.x, y: point.y })),
    ),
  ];
  if (points.length === 0) return null;
  return points.reduce(
    (bounds, point) => ({
      minX: Math.min(bounds.minX, point.x),
      minY: Math.min(bounds.minY, point.y),
      maxX: Math.max(bounds.maxX, point.x),
      maxY: Math.max(bounds.maxY, point.y),
    }),
    {
      minX: Number.POSITIVE_INFINITY,
      minY: Number.POSITIVE_INFINITY,
      maxX: Number.NEGATIVE_INFINITY,
      maxY: Number.NEGATIVE_INFINITY,
    },
  );
}
