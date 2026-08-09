import type { BpmnEditorDocument, FlowElementEnum } from '../generated/editor-protocol';

export function documentElements(document: BpmnEditorDocument): FlowElementEnum[] {
  return document.model.processes.flatMap((process) => flattenElements(process.flowElements ?? []));
}

export function flattenElements(elements: FlowElementEnum[]): FlowElementEnum[] {
  return elements.flatMap((element) => [element, ...flattenElements(nestedElements(element))]);
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
