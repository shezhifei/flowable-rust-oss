import { produce } from 'immer';
import { describe, expect, it } from 'vitest';

import { createPaletteElement } from './elementFactory';
import { resolveBoundaryAttachment, resolveDropOwner, type BoundaryElementSize } from './ownership';
import { sampleDocument } from './sampleDocument';

describe('canonical ownership resolution', () => {
  it('chooses the deepest subprocess and the containing lane at a drop point', () => {
    const document = documentWithNestedSubprocesses();
    let resolution: ReturnType<typeof resolveDropOwner> = null;

    produce(document, (draft) => {
      resolution = resolveDropOwner(draft, { x: 390, y: 190 });
    });

    expect(resolution).toMatchObject({
      laneId: 'managerLane',
      ownerId: 'inner-subprocess',
      processId: 'leaveProcess',
    });
  });

  it('resolves a second pool to its referenced process and lane', () => {
    const document = documentWithSecondProcess();
    let resolution: ReturnType<typeof resolveDropOwner> = null;

    produce(document, (draft) => {
      resolution = resolveDropOwner(draft, { x: 1510, y: 170 });
    });

    expect(resolution).toMatchObject({
      laneId: 'second-lane',
      ownerId: 'second-process',
      processId: 'second-process',
    });
  });

  it('snaps a timer boundary to the nearest legal activity border', () => {
    const document = structuredClone(sampleDocument);
    let attachment: ReturnType<typeof resolveBoundaryAttachment> = null;
    const size: BoundaryElementSize = { width: 34, height: 34 };

    produce(document, (draft) => {
      attachment = resolveBoundaryAttachment(draft, 'leaveProcess', { x: 464, y: 185 }, size);
    });

    expect(attachment).toMatchObject({
      hostId: 'review',
      laneId: 'managerLane',
      ownerId: 'leaveProcess',
      processId: 'leaveProcess',
      bounds: { x: 443, y: 168, width: 34, height: 34 },
    });
  });
});

function documentWithNestedSubprocesses() {
  const document = structuredClone(sampleDocument);
  const process = required(document.model.processes[0]);
  const outer = createPaletteElement('subprocess', 'outer-subprocess');
  const inner = createPaletteElement('subprocess', 'inner-subprocess');
  if (outer.elementType !== 'subProcess' || inner.elementType !== 'subProcess') {
    throw new Error('expected subprocess fixtures');
  }
  outer.flowElements = [inner];
  outer.flowElementMap = { 'inner-subprocess': inner };
  process.flowElements?.push(outer);
  document.model.locationMap['outer-subprocess'] = bounds(250, 100, 420, 250);
  document.model.locationMap['inner-subprocess'] = bounds(320, 130, 240, 150);
  return document;
}

function documentWithSecondProcess() {
  const document = structuredClone(sampleDocument);
  const first = required(document.model.processes[0]);
  const second = structuredClone(first);
  second.id = 'second-process';
  second.name = 'Second process';
  second.flowElements = [];
  second.flowElementMap = {};
  second.dataObjects = [];
  second.artifacts = [];
  second.artifactMap = {};
  second.lanes = [
    {
      id: 'second-lane',
      name: 'Second lane',
      flowReferences: [],
      attributes: {},
      extensionElements: {},
      xmlRowNumber: 0,
      xmlColumnNumber: 0,
    },
  ];
  document.model.processes.push(second);
  const pool = structuredClone(required(document.model.pools[0]));
  pool.id = 'second-pool';
  pool.name = 'Second pool';
  pool.processRef = 'second-process';
  document.model.pools.push(pool);
  document.model.locationMap['second-pool'] = bounds(1400, 72, 600, 300);
  document.model.locationMap['second-lane'] = bounds(1440, 72, 560, 300);
  return document;
}

function bounds(x: number, y: number, width: number, height: number) {
  return {
    x,
    y,
    width,
    height,
    rotation: 0,
    expanded: true,
    xmlRowNumber: 0,
    xmlColumnNumber: 0,
  };
}

function required<T>(value: T | undefined): T {
  if (value === undefined) throw new Error('expected fixture value to exist');
  return value;
}
