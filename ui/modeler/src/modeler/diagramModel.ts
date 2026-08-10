import type {
  ArtifactEnum,
  BpmnEditorDocument,
  FlowElementEnum,
} from '../generated/editor-protocol';

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
