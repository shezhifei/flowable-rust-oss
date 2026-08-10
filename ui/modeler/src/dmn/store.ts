import { applyPatches, enablePatches, produceWithPatches, type Patch } from 'immer';
import { createStore, type StoreApi } from 'zustand/vanilla';

import type { DmnEditorDocument } from '../generated/editor-protocol';
import type { DmnCommand } from './commands';

enablePatches();

const MAX_HISTORY = 100;

export interface DmnHistoryEntry {
  label: string;
  patches: Patch[];
  inversePatches: Patch[];
}

export interface DmnEditorState {
  document: DmnEditorDocument;
  selectedDecisionId: string | null;
  undoStack: DmnHistoryEntry[];
  redoStack: DmnHistoryEntry[];
  setDocument: (document: DmnEditorDocument) => void;
  selectDecision: (decisionId: string | null) => void;
  execute: (command: DmnCommand) => void;
  undo: () => void;
  redo: () => void;
}

export type DmnEditorStore = StoreApi<DmnEditorState>;

/** Creates an isolated canonical DMN editor store for one open document. */
export function createDmnEditorStore(initialDocument: DmnEditorDocument): DmnEditorStore {
  return createStore<DmnEditorState>()((set, get) => ({
    document: initialDocument,
    selectedDecisionId: initialDocument.model.decisions?.[0]?.id ?? null,
    undoStack: [],
    redoStack: [],
    setDocument: (document) =>
      set({
        document,
        selectedDecisionId: document.model.decisions?.[0]?.id ?? null,
        undoStack: [],
        redoStack: [],
      }),
    selectDecision: (selectedDecisionId) => set({ selectedDecisionId }),
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
  }));
}
