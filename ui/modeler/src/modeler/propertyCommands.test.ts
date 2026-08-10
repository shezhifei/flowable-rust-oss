import { beforeEach, describe, expect, it } from 'vitest';

import { useModelerStore } from './modelerStore';
import {
  renameElementIdCommand,
  updateElementPropertiesCommand,
  updateProcessPropertiesCommand,
} from './propertyCommands';
import { sampleDocument } from './sampleDocument';

function resetStore() {
  useModelerStore.getState().setDocument(structuredClone(sampleDocument));
}

function state() {
  return useModelerStore.getState();
}

function flowElement(id: string) {
  const element = state().document.model.processes[0]?.flowElementMap?.[id];
  if (!element) throw new Error(`${id} is missing`);
  return element;
}

describe('element property updates', () => {
  beforeEach(resetStore);

  it('writes user task properties through the command stack with undo/redo', () => {
    state().execute(
      updateElementPropertiesCommand(
        'review',
        { assignee: 'kermit', candidateGroups: ['management', 'hr'], priority: '75' },
        'Edit assignee',
      ),
    );

    expect(state().undoStack.at(-1)?.label).toBe('Edit assignee');
    expect(flowElement('review')).toMatchObject({
      assignee: 'kermit',
      candidateGroups: ['management', 'hr'],
      priority: '75',
    });

    state().undo();
    expect(flowElement('review')).toMatchObject({
      assignee: null,
      candidateGroups: ['managers'],
      priority: '50',
    });

    state().redo();
    expect(flowElement('review')).toMatchObject({ assignee: 'kermit', priority: '75' });
  });

  it('clears nullable values and list values', () => {
    state().execute(
      updateElementPropertiesCommand('review', { formKey: null, candidateGroups: [] }),
    );
    expect(flowElement('review')).toMatchObject({ formKey: null, candidateGroups: [] });
    state().undo();
    expect(flowElement('review')).toMatchObject({ formKey: 'leaveRequest' });
  });

  it('writes service task implementation fields and sequence flow conditions', () => {
    state().execute(
      updateElementPropertiesCommand('notify', {
        implementationType: 'class',
        implementation: 'org.flowable.NotifyDelegate',
        resultVariableName: 'notifyResult',
      }),
    );
    expect(flowElement('notify')).toMatchObject({
      implementationType: 'class',
      implementation: 'org.flowable.NotifyDelegate',
      resultVariableName: 'notifyResult',
    });

    state().execute(
      updateElementPropertiesCommand('approvedFlow', {
        conditionExpression: '${approved == true}',
      }),
    );
    expect(flowElement('approvedFlow')).toMatchObject({
      conditionExpression: '${approved == true}',
    });

    state().undo();
    state().undo();
    expect(flowElement('notify')).toMatchObject({ implementationType: 'delegateExpression' });
    expect(flowElement('approvedFlow')).toMatchObject({ conditionExpression: '${approved}' });
  });

  it('rejects writes to unknown elements without touching the document', () => {
    expect(() =>
      state().execute(updateElementPropertiesCommand('ghost', { name: 'Ghost' })),
    ).toThrowError(expect.objectContaining({ code: 'missing-element' }));
    expect(state().undoStack).toHaveLength(0);
  });
});

describe('element id rename', () => {
  beforeEach(resetStore);

  it('rewires DI maps, flow endpoints, attachments, and lane memberships', () => {
    state().execute(renameElementIdCommand('review', 'audit'));

    const { model } = state().document;
    expect(model.locationMap.audit).toBeDefined();
    expect(model.locationMap.review).toBeUndefined();
    expect(flowElement('requestFlow')).toMatchObject({ targetRef: 'audit' });
    expect(flowElement('decisionFlow')).toMatchObject({ sourceRef: 'audit' });
    expect(flowElement('reviewTimer')).toMatchObject({ attachedToRefId: 'audit' });
    expect(model.processes[0]?.lanes?.[0]?.flowReferences).toContain('audit');
    expect(model.processes[0]?.lanes?.[0]?.flowReferences).not.toContain('review');
    expect(model.processes[0]?.flowElementMap?.review).toBeUndefined();

    state().undo();
    expect(flowElement('requestFlow')).toMatchObject({ targetRef: 'review' });
    expect(state().document.model.locationMap.review).toBeDefined();
    expect(state().document.model.processes[0]?.lanes?.[0]?.flowReferences).toContain('review');

    state().redo();
    expect(state().document.model.locationMap.audit).toBeDefined();
  });

  it('moves sequence flow waypoints and label bounds to the new id', () => {
    state().execute(renameElementIdCommand('approvedFlow', 'confirmedFlow'));

    const { model } = state().document;
    expect(model.flowLocationMap.confirmedFlow).toHaveLength(4);
    expect(model.flowLocationMap.approvedFlow).toBeUndefined();
    expect(model.labelLocationMap.confirmedFlow).toBeDefined();
    expect(model.labelLocationMap.approvedFlow).toBeUndefined();
  });

  it('rewires association endpoints through artifacts', () => {
    state().execute(renameElementIdCommand('decision', 'verdict'));
    const association = state().document.model.processes[0]?.artifactMap?.approvalLink;
    expect(association).toMatchObject({ targetRef: 'verdict' });
  });

  it('refuses duplicate, blank, and whitespace ids', () => {
    expect(() => state().execute(renameElementIdCommand('review', 'notify'))).toThrowError(
      expect.objectContaining({ code: 'duplicate-element-id' }),
    );
    expect(() => state().execute(renameElementIdCommand('review', 'approvalNote'))).toThrowError(
      expect.objectContaining({ code: 'duplicate-element-id' }),
    );
    expect(() => state().execute(renameElementIdCommand('review', '  '))).toThrowError(
      expect.objectContaining({ code: 'invalid-element-id' }),
    );
    expect(() => state().execute(renameElementIdCommand('review', 'has space'))).toThrowError(
      expect.objectContaining({ code: 'invalid-element-id' }),
    );
    expect(state().undoStack).toHaveLength(0);
    expect(state().document.model.locationMap.review).toBeDefined();
  });

  it('ignores a rename to the same id', () => {
    state().execute(renameElementIdCommand('review', 'review'));
    expect(state().undoStack).toHaveLength(0);
  });
});

describe('process property updates', () => {
  beforeEach(resetStore);

  it('edits the main process name and documentation with undo', () => {
    state().execute(
      updateProcessPropertiesCommand({ name: 'Leave approval v2', documentation: null }),
    );
    expect(state().document.model.processes[0]).toMatchObject({
      name: 'Leave approval v2',
      documentation: null,
    });

    state().undo();
    expect(state().document.model.processes[0]).toMatchObject({
      name: 'Leave approval',
      documentation: 'A representative Flowable process rendered from the editor protocol.',
    });
  });

  it('renames the process id and updates the pool process reference', () => {
    state().execute(updateProcessPropertiesCommand({ id: 'leaveApprovalProcess' }));
    expect(state().document.model.processes[0]?.id).toBe('leaveApprovalProcess');
    expect(state().document.model.pools[0]?.processRef).toBe('leaveApprovalProcess');

    state().undo();
    expect(state().document.model.processes[0]?.id).toBe('leaveProcess');
    expect(state().document.model.pools[0]?.processRef).toBe('leaveProcess');
  });

  it('rejects a process id that collides with any diagram id', () => {
    expect(() => state().execute(updateProcessPropertiesCommand({ id: 'review' }))).toThrowError(
      expect.objectContaining({ code: 'duplicate-element-id' }),
    );
    expect(state().undoStack).toHaveLength(0);
  });
});
