import type { BpmnEditorDocument } from '../generated/editor-protocol';
import { documentArtifacts, documentElements } from './diagramModel';

/**
 * Every id owned by the diagram: processes, flow elements (recursive), data
 * objects, artifacts, pools, and lanes. BPMN ids are document-scoped, so a
 * rename or a process id edit must be unique against all of them.
 */
export function collectModelIds(document: BpmnEditorDocument): Set<string> {
  const ids = new Set<string>();
  const add = (id: string | null | undefined) => {
    if (id) ids.add(id);
  };
  for (const process of document.model.processes) {
    add(process.id);
    for (const dataObject of process.dataObjects ?? []) add(dataObject.id);
    for (const lane of process.lanes ?? []) add(lane.id);
  }
  for (const element of documentElements(document)) add(element.id);
  for (const artifact of documentArtifacts(document)) add(artifact.id);
  for (const pool of document.model.pools) add(pool.id);
  return ids;
}

export function validateElementId(
  document: BpmnEditorDocument,
  currentId: string | null,
  nextId: string,
): string | null {
  const trimmed = nextId.trim();
  if (!trimmed) return 'ID is required';
  if (/\s/.test(trimmed)) return 'ID must not contain whitespace';
  if (trimmed === currentId) return null;
  if (collectModelIds(document).has(trimmed)) return `ID "${trimmed}" is already used`;
  return null;
}

/** Optional numeric field (e.g. user task priority): empty clears, otherwise digits. */
export function validateNumericValue(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  if (!/^-?\d+$/.test(trimmed)) return 'Enter a whole number';
  return null;
}

/**
 * Condition expressions stay unparsed UEL text. Empty clears the condition;
 * non-empty text only gets a basic bracket-balance check.
 */
export function validateConditionExpression(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const pairs: Record<string, string> = { ')': '(', ']': '[', '}': '{' };
  const stack: string[] = [];
  for (const char of trimmed) {
    if (char === '(' || char === '[' || char === '{') {
      stack.push(char);
    } else if (char === ')' || char === ']' || char === '}') {
      if (stack.pop() !== pairs[char]) return 'Brackets are not balanced';
    }
  }
  return stack.length === 0 ? null : 'Brackets are not balanced';
}
