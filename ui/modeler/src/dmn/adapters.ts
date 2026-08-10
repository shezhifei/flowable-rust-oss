import type { Decision, DecisionService, DmnEditorDocument } from '../generated/editor-protocol';

/**
 * Table-level values presented by the UI. `key` is an adapter name for the
 * canonical `Decision.id`; it is never persisted as a second field.
 */
export interface DecisionTableProperties {
  definitionId: string | null;
  definitionName: string | null;
  definitionNamespace: string | null;
  key: string;
  name: string | null;
  tableId: string;
}

export function decisionById(document: DmnEditorDocument, decisionId: string): Decision | null {
  return document.model.decisions?.find((decision) => decision.id === decisionId) ?? null;
}

export function readDecisionTableProperties(
  document: DmnEditorDocument,
  decisionId: string,
): DecisionTableProperties | null {
  const decision = decisionById(document, decisionId);
  if (!decision) return null;
  return {
    definitionId: document.model.id ?? null,
    definitionName: document.model.name ?? null,
    definitionNamespace: document.model.namespace ?? null,
    key: decision.id,
    name: decision.name ?? null,
    tableId: decision.decisionTable.id,
  };
}

/** Returns canonical decision-service objects for a read-only DRD summary. */
export function decisionServicesFor(
  document: DmnEditorDocument,
  decisionId: string,
): readonly DecisionService[] {
  return (document.model.decisionServices ?? []).filter(
    (service) =>
      service.outputDecisions?.includes(decisionId) === true ||
      service.requiredDecisions?.includes(decisionId) === true,
  );
}
