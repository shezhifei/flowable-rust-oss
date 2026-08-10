import { applyPatches, enablePatches, produceWithPatches, type Patch } from 'immer';
import { createStore, type StoreApi } from 'zustand/vanilla';

import type { FormEditorDocument } from '../generated/editor-protocol';
import type { FormCommand } from './commands';

enablePatches();

const MAX_HISTORY = 100;

export interface FormHistoryEntry {
  label: string;
  patches: Patch[];
  inversePatches: Patch[];
}

export interface FormEditorState {
  document: FormEditorDocument;
  selectedFieldId: string | null;
  undoStack: FormHistoryEntry[];
  redoStack: FormHistoryEntry[];
  setDocument: (document: FormEditorDocument) => void;
  selectField: (fieldId: string | null) => void;
  execute: (command: FormCommand) => void;
  undo: () => void;
  redo: () => void;
}

export type FormEditorStore = StoreApi<FormEditorState>;

/** Creates an isolated form designer store for one open document. */
export function createFormEditorStore(initialDocument: FormEditorDocument): FormEditorStore {
  return createStore<FormEditorState>()((set, get) => ({
    document: initialDocument,
    selectedFieldId: null,
    undoStack: [],
    redoStack: [],
    setDocument: (document) =>
      set({ document, selectedFieldId: null, undoStack: [], redoStack: [] }),
    selectField: (selectedFieldId) => set({ selectedFieldId }),
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
