import { applyPatches, enablePatches, produceWithPatches, type Patch } from 'immer';
import { create } from 'zustand';

import type { BpmnEditorDocument } from '../generated/editor-protocol';
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
  fitToModel: () => void;
  resetViewport: () => void;
}

declare global {
  interface Window {
    __FLOWABLE_MODELER_TEST__?: {
      setDocument: (document: BpmnEditorDocument) => void;
    };
  }
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
