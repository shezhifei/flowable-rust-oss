import { FormCommandError, type FormCommand, type FormEditorStore } from '../index';

/**
 * Executes a command against the store, converting known command errors into
 * displayable messages so property fields can surface them next to the input.
 */
export function executeFormCommand(store: FormEditorStore, command: FormCommand): string | null {
  try {
    store.getState().execute(command);
    return null;
  } catch (error) {
    if (error instanceof FormCommandError) return error.message;
    throw error;
  }
}
