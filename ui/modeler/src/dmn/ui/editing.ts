import {
  DmnCommandError,
  editCellCommand,
  type DmnCellAddress,
  type DmnCommand,
  type DmnEditorStore,
} from '../index';
import { validateFeelExpression, validateUnaryTests } from '../feelValidation';

/**
 * Executes a command against the store, converting known command errors into
 * displayable messages so form fields can surface them next to the input.
 */
export function executeDmnCommand(store: DmnEditorStore, command: DmnCommand): string | null {
  try {
    store.getState().execute(command);
    return null;
  } catch (error) {
    if (error instanceof DmnCommandError) return error.message;
    throw error;
  }
}

/**
 * Commits a cell draft after the FEEL subset gate. Invalid drafts never reach
 * the document: the returned message keeps the cell flagged in the UI.
 */
export function commitCellText(
  store: DmnEditorStore,
  decisionId: string,
  address: DmnCellAddress,
  draft: string,
): string | null {
  const validationError =
    address.kind === 'input' ? validateUnaryTests(draft) : validateFeelExpression(draft);
  if (validationError) return validationError;
  const text = draft.trim() === '' ? null : draft.trim();
  return executeDmnCommand(store, editCellCommand(decisionId, address, { text }));
}
