import type {
  ArtifactEnum,
  BpmnEditorDocument,
  FlowElementEnum,
  Lane,
  Pool,
  Process,
} from '../generated/editor-protocol';

/**
 * Every kind of shape the canvas lets the author select. Pools, lanes, and
 * artifacts live outside `flowElements`, so a lookup that only walks the
 * process tree cannot reach them.
 */
export type DiagramShape =
  | { kind: 'flowElement'; element: FlowElementEnum }
  | { kind: 'pool'; pool: Pool }
  | { kind: 'lane'; lane: Lane; process: Process }
  | { kind: 'artifact'; artifact: ArtifactEnum };

/** Resolves a selected id to the shape that owns it, whatever collection it sits in. */
export function findDiagramShape(document: BpmnEditorDocument, id: string): DiagramShape | null {
  const element = documentElements(document).find((candidate) => candidate.id === id);
  if (element) return { kind: 'flowElement', element };

  const pool = document.model.pools.find((candidate) => candidate.id === id);
  if (pool) return { kind: 'pool', pool };

  for (const process of document.model.processes) {
    const lane = process.lanes?.find((candidate) => candidate.id === id);
    if (lane) return { kind: 'lane', lane, process };
  }

  const artifact = documentArtifacts(document).find((candidate) => candidate.id === id);
  if (artifact) return { kind: 'artifact', artifact };

  return null;
}

/** The process a pool points at, or null when the reference dangles. */
export function processForPool(document: BpmnEditorDocument, pool: Pool): Process | null {
  if (!pool.processRef) return null;
  return document.model.processes.find((process) => process.id === pool.processRef) ?? null;
}

export function documentElements(document: BpmnEditorDocument): FlowElementEnum[] {
  return document.model.processes.flatMap((process) => flattenElements(process.flowElements ?? []));
}

export function flattenElements(elements: FlowElementEnum[]): FlowElementEnum[] {
  return elements.flatMap((element) => [element, ...flattenElements(nestedElements(element))]);
}

export function documentArtifacts(document: BpmnEditorDocument): ArtifactEnum[] {
  return [
    ...document.model.globalArtifacts,
    ...document.model.processes.flatMap((process) => [
      ...(process.artifacts ?? []),
      ...nestedArtifacts(process.flowElements ?? []),
    ]),
  ];
}

function nestedArtifacts(elements: FlowElementEnum[]): ArtifactEnum[] {
  return elements.flatMap((element) => {
    switch (element.elementType) {
      case 'subProcess':
      case 'transaction':
      case 'eventSubProcess':
      case 'adhocSubProcess':
        return [...(element.artifacts ?? []), ...nestedArtifacts(element.flowElements ?? [])];
      default:
        return [];
    }
  });
}

function nestedElements(element: FlowElementEnum): FlowElementEnum[] {
  switch (element.elementType) {
    case 'subProcess':
    case 'transaction':
    case 'eventSubProcess':
    case 'adhocSubProcess':
      return element.flowElements ?? [];
    default:
      return [];
  }
}
