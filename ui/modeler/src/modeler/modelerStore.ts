import { create } from 'zustand';
import { immer } from 'zustand/middleware/immer';

import type { BpmnEditorDocument } from '../generated/editor-protocol';
import { sampleDocument } from './sampleDocument';

const MIN_ZOOM = 0.35;
const MAX_ZOOM = 2.5;

interface ViewportState {
  x: number;
  y: number;
  zoom: number;
}

interface ModelerState {
  document: BpmnEditorDocument;
  viewport: ViewportState;
  selectedElementId: string | null;
  setDocument: (document: BpmnEditorDocument) => void;
  selectElement: (elementId: string | null) => void;
  panBy: (deltaX: number, deltaY: number) => void;
  zoomBy: (factor: number) => void;
  resetViewport: () => void;
}

const initialViewport: ViewportState = { x: 16, y: 18, zoom: 0.82 };

export const useModelerStore = create<ModelerState>()(
  immer((set) => ({
    document: sampleDocument,
    viewport: initialViewport,
    selectedElementId: 'review',
    setDocument: (document) =>
      set((state) => {
        state.document = document;
        state.selectedElementId = null;
      }),
    selectElement: (elementId) =>
      set((state) => {
        state.selectedElementId = elementId;
      }),
    panBy: (deltaX, deltaY) =>
      set((state) => {
        state.viewport.x += deltaX;
        state.viewport.y += deltaY;
      }),
    zoomBy: (factor) =>
      set((state) => {
        state.viewport.zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, state.viewport.zoom * factor));
      }),
    resetViewport: () =>
      set((state) => {
        state.viewport = initialViewport;
      }),
  })),
);
