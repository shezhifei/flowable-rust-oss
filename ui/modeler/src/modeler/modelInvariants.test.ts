import { produce } from 'immer';
import { describe, expect, it } from 'vitest';

import { createPaletteElement } from './elementFactory';
import { moveElementCommand } from './commands';
import { locateCanonicalElement, normalizeModelInvariants } from './modelInvariants';
import { sampleDocument } from './sampleDocument';

describe('model invariants', () => {
  it('recursively rebuilds canonical lookup and relationship mirrors', () => {
    const document = structuredClone(sampleDocument);
    const process = required(document.model.processes[0]);
    const subprocess = createPaletteElement('subprocess', 'nested');
    if (subprocess.elementType !== 'subProcess') throw new Error('expected a subprocess');
    const child = createPaletteElement('task', 'nested-task');
    subprocess.flowElements = [child];
    subprocess.flowElementMap = {};
    const nestedArtifact = structuredClone(required(process.artifacts?.[0]));
    nestedArtifact.id = 'nested-note';
    subprocess.artifacts = [nestedArtifact];
    subprocess.artifactMap = {};
    process.flowElements?.push(subprocess);
    process.flowElementMap = {};
    process.artifactMap = {};
    required(process.lanes?.[0]).flowReferences = ['review', 'missing', 'review', 'decision'];
    document.model.mainProcess = null;

    let locatedOwnerId: string | null | undefined;
    const normalized = produce(document, (draft) => {
      normalizeModelInvariants(draft);
      locatedOwnerId = locateCanonicalElement(draft, 'nested-task')?.ownerId;
    });

    const normalizedProcess = required(normalized.model.processes[0]);
    const normalizedSubprocess = normalizedProcess.flowElements?.find(
      (element) => element.elementType === 'subProcess' && element.id === 'nested',
    );
    if (!normalizedSubprocess || normalizedSubprocess.elementType !== 'subProcess') {
      throw new Error('normalized subprocess was not found');
    }
    expect(locatedOwnerId).toBe('nested');
    expect(normalizedSubprocess.flowElementMap['nested-task']).toMatchObject({
      elementType: 'userTask',
    });
    expect(normalizedProcess.flowElementMap?.['nested-task']).toMatchObject({
      elementType: 'userTask',
    });
    expect(normalizedSubprocess.artifactMap['nested-note']).toMatchObject({
      artifactType: 'textAnnotation',
    });
    expect(normalizedProcess.artifactMap?.['nested-note']).toMatchObject({
      artifactType: 'textAnnotation',
    });
    expect(required(normalizedProcess.lanes?.[0]).flowReferences).toEqual(['review', 'decision']);

    const review = required(
      normalizedProcess.flowElements?.find((element) => element.id === 'review'),
    );
    if (review.elementType !== 'userTask') throw new Error('expected review to be a user task');
    expect(review.incomingFlows.map((flow) => flow.id)).toEqual(['requestFlow']);
    expect(review.outgoingFlows.map((flow) => flow.id)).toEqual(['decisionFlow']);
    expect(review.boundaryEvents?.map((event) => event.id)).toEqual(['reviewTimer']);

    expect(normalized.model.mainProcess?.flowElementMap?.['nested-task']).toMatchObject({
      elementType: 'userTask',
    });
    expect(normalized.model.mainProcess).not.toBe(normalizedProcess);
    expect(normalizedProcess.flowElementMap?.review).not.toBe(review);
  });

  it('gives the first valid lane sole ownership of each flow node', () => {
    const document = structuredClone(sampleDocument);
    const process = required(document.model.processes[0]);
    required(process.lanes?.[0]).flowReferences = ['review', 'decision'];
    required(process.lanes?.[1]).flowReferences = ['review', 'notify', 'unknown'];

    const normalized = produce(document, normalizeModelInvariants);

    expect(required(normalized.model.processes[0]?.lanes?.[0]).flowReferences).toEqual([
      'review',
      'decision',
    ]);
    expect(required(normalized.model.processes[0]?.lanes?.[1]).flowReferences).toEqual(['notify']);
  });

  it('repairs derived mirrors after an existing movement command', () => {
    const document = structuredClone(sampleDocument);
    const process = required(document.model.processes[0]);
    process.flowElementMap = {};
    process.artifactMap = {};
    document.model.mainProcess = null;

    const moved = produce(document, (draft) => {
      moveElementCommand('review', 20, 20).apply(draft);
    });

    expect(moved.model.processes[0]?.flowElementMap?.review).toMatchObject({
      elementType: 'userTask',
    });
    expect(moved.model.processes[0]?.artifactMap?.approvalLink).toMatchObject({
      artifactType: 'association',
    });
    expect(moved.model.mainProcess?.flowElementMap?.review).toMatchObject({
      elementType: 'userTask',
    });
  });
});

function required<T>(value: T | undefined): T {
  if (value === undefined) throw new Error('expected fixture value to exist');
  return value;
}
