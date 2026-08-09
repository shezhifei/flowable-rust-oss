import { useRef, useState, type PointerEvent as ReactPointerEvent, type WheelEvent } from 'react';

import type {
  ArtifactEnum,
  FlowElementEnum,
  GraphicInfo,
  MessageFlow,
} from '../generated/editor-protocol';
import { BpmnElement } from './BpmnElement';
import { moveElementCommand } from './commands';
import { documentElements } from './diagramModel';
import { useModelerStore } from './modelerStore';

const CANVAS_WIDTH = 1400;
const CANVAS_HEIGHT = 620;

export function BpmnCanvas() {
  const document = useModelerStore((state) => state.document);
  const viewport = useModelerStore((state) => state.viewport);
  const selectedElementId = useModelerStore((state) => state.selectedElementId);
  const selectElement = useModelerStore((state) => state.selectElement);
  const panBy = useModelerStore((state) => state.panBy);
  const zoomBy = useModelerStore((state) => state.zoomBy);
  const execute = useModelerStore((state) => state.execute);
  const dragOrigin = useRef<{ x: number; y: number } | null>(null);
  const elementDrag = useRef<{ elementId: string; x: number; y: number } | null>(null);
  const [dragOffset, setDragOffset] = useState<{ x: number; y: number } | null>(null);

  const elements = documentElements(document);
  const nodes = elements.filter(isNode);
  const flows = elements.filter(isSequenceFlow);

  const handlePointerDown = (event: ReactPointerEvent<SVGSVGElement>) => {
    if (event.button !== 0) return;
    if (event.target instanceof Element && event.target.closest('.diagram-element')) return;
    dragOrigin.current = { x: event.clientX, y: event.clientY };
    event.currentTarget.setPointerCapture(event.pointerId);
    if (event.target === event.currentTarget) selectElement(null);
  };

  const handlePointerMove = (event: ReactPointerEvent<SVGSVGElement>) => {
    if (elementDrag.current) {
      setDragOffset({
        x: (event.clientX - elementDrag.current.x) / viewport.zoom,
        y: (event.clientY - elementDrag.current.y) / viewport.zoom,
      });
      return;
    }
    if (!dragOrigin.current) return;
    const deltaX = event.clientX - dragOrigin.current.x;
    const deltaY = event.clientY - dragOrigin.current.y;
    dragOrigin.current = { x: event.clientX, y: event.clientY };
    panBy(deltaX, deltaY);
  };

  const handlePointerUp = (event: ReactPointerEvent<SVGSVGElement>) => {
    if (elementDrag.current) {
      const finalOffset = {
        x: (event.clientX - elementDrag.current.x) / viewport.zoom,
        y: (event.clientY - elementDrag.current.y) / viewport.zoom,
      };
      if (finalOffset.x !== 0 || finalOffset.y !== 0) {
        execute(moveElementCommand(elementDrag.current.elementId, finalOffset.x, finalOffset.y));
      }
      elementDrag.current = null;
      setDragOffset(null);
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
      return;
    }
    dragOrigin.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const handleElementDragStart = (elementId: string, event: ReactPointerEvent<SVGGElement>) => {
    if (event.button !== 0) return;
    event.stopPropagation();
    selectElement(elementId);
    elementDrag.current = { elementId, x: event.clientX, y: event.clientY };
    event.currentTarget.ownerSVGElement?.setPointerCapture(event.pointerId);
  };

  const handleWheel = (event: WheelEvent<SVGSVGElement>) => {
    event.preventDefault();
    zoomBy(event.deltaY < 0 ? 1.1 : 0.9);
  };

  return (
    <div className="canvas-viewport" data-testid="canvas-viewport">
      <svg
        className="bpmn-canvas"
        viewBox={`0 0 ${CANVAS_WIDTH} ${CANVAS_HEIGHT}`}
        role="application"
        aria-label="BPMN process canvas"
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerCancel={handlePointerUp}
        onWheel={handleWheel}
      >
        <defs>
          <marker
            id="sequence-arrow"
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="7"
            markerHeight="7"
            orient="auto"
          >
            <path d="M 0 0 L 10 5 L 0 10 z" />
          </marker>
          <marker
            id="message-arrow"
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="7"
            markerHeight="7"
            orient="auto"
          >
            <path className="message-marker" d="M 0 0 L 10 5 L 0 10 Z" />
          </marker>
          <filter id="selection-glow" x="-40%" y="-40%" width="180%" height="180%">
            <feDropShadow dx="0" dy="0" stdDeviation="4" floodColor="#ff5c35" floodOpacity="0.34" />
          </filter>
        </defs>
        <g transform={`translate(${viewport.x} ${viewport.y}) scale(${viewport.zoom})`}>
          <PoolAndLanes />
          <g className="flow-layer">
            {flows.map((flow) => (
              <FlowPath key={flow.id ?? `${flow.sourceRef}-${flow.targetRef}`} flow={flow} />
            ))}
            {Object.values(document.model.messageFlows).map((flow) => (
              <MessageFlowPath key={flow.id ?? `${flow.sourceRef}-${flow.targetRef}`} flow={flow} />
            ))}
            {[
              ...document.model.globalArtifacts,
              ...document.model.processes.flatMap((process) => process.artifacts ?? []),
            ].map((association) => (
              <AssociationPath
                key={association.id ?? `${association.sourceRef}-${association.targetRef}`}
                association={association}
              />
            ))}
          </g>
          <DataStores />
          <g className="node-layer">
            {nodes.map((element) => {
              const id = element.id;
              const bounds = id ? document.model.locationMap[id] : undefined;
              if (!id || !bounds) return null;
              return (
                <BpmnElement
                  key={id}
                  element={element}
                  bounds={bounds}
                  labelBounds={document.model.labelLocationMap[id]}
                  selected={selectedElementId === id}
                  dragOffset={selectedElementId === id ? (dragOffset ?? undefined) : undefined}
                  onSelect={selectElement}
                  onDragStart={handleElementDragStart}
                />
              );
            })}
          </g>
        </g>
      </svg>
      <div className="canvas-coordinate" aria-hidden="true">
        {Math.round(viewport.zoom * 100)}% · x {Math.round(viewport.x)} · y {Math.round(viewport.y)}
      </div>
    </div>
  );
}

function PoolAndLanes() {
  const model = useModelerStore((state) => state.document.model);
  return (
    <g className="pool-layer">
      {model.pools.map((pool) => {
        if (!pool.id) return null;
        const bounds = model.locationMap[pool.id];
        if (!bounds) return null;
        return (
          <g key={pool.id} className="pool-shape" data-element-id={pool.id}>
            <rect x={bounds.x} y={bounds.y} width={bounds.width} height={bounds.height} />
            <line
              x1={bounds.x + 40}
              y1={bounds.y}
              x2={bounds.x + 40}
              y2={bounds.y + bounds.height}
            />
            <text
              transform={`translate(${bounds.x + 24} ${bounds.y + bounds.height / 2}) rotate(-90)`}
              textAnchor="middle"
            >
              {pool.name ?? 'Pool'}
            </text>
          </g>
        );
      })}
      {model.processes
        .flatMap((process) => process.lanes ?? [])
        .map((lane) => {
          if (!lane.id) return null;
          const bounds = model.locationMap[lane.id];
          if (!bounds) return null;
          return (
            <g key={lane.id} className="lane-shape" data-element-id={lane.id}>
              <rect x={bounds.x} y={bounds.y} width={bounds.width} height={bounds.height} />
              <text
                transform={`translate(${bounds.x + 22} ${bounds.y + bounds.height / 2}) rotate(-90)`}
                textAnchor="middle"
              >
                {lane.name ?? 'Lane'}
              </text>
            </g>
          );
        })}
    </g>
  );
}

function DataStores() {
  const model = useModelerStore((state) => state.document.model);
  return (
    <g className="data-store-layer">
      {Object.values(model.dataStores).map((store) => {
        if (!store.id) return null;
        const bounds = model.locationMap[store.id];
        if (!bounds) return null;
        const centerX = bounds.x + bounds.width / 2;
        return (
          <g key={store.id} className="data-store-shape" data-element-id={store.id}>
            <path
              d={`M ${bounds.x} ${bounds.y + 7} C ${bounds.x} ${bounds.y - 2}, ${bounds.x + bounds.width} ${bounds.y - 2}, ${bounds.x + bounds.width} ${bounds.y + 7} v ${bounds.height - 14} C ${bounds.x + bounds.width} ${bounds.y + bounds.height + 2}, ${bounds.x} ${bounds.y + bounds.height + 2}, ${bounds.x} ${bounds.y + bounds.height - 7} Z`}
            />
            <ellipse cx={centerX} cy={bounds.y + 7} rx={bounds.width / 2} ry={7} />
            <text x={centerX} y={bounds.y + bounds.height + 18} textAnchor="middle">
              {store.name ?? 'Data store'}
            </text>
          </g>
        );
      })}
    </g>
  );
}

function FlowPath({ flow }: { flow: Extract<FlowElementEnum, { elementType: 'sequenceFlow' }> }) {
  const model = useModelerStore((state) => state.document.model);
  if (!flow.id) return null;
  const points = resolveWaypoints(flow, model.flowLocationMap[flow.id], model.locationMap);
  if (points.length < 2) return null;
  const label = model.labelLocationMap[flow.id];
  return (
    <g className="sequence-flow" data-element-id={flow.id}>
      <path d={polylinePath(points)} markerEnd="url(#sequence-arrow)" />
      {flow.conditionExpression ? (
        <path className="condition-marker" d={conditionMarker(points[0])} />
      ) : null}
      {flow.name ? (
        <text
          x={label ? label.x + label.width / 2 : midpoint(points).x}
          y={label ? label.y + 14 : midpoint(points).y - 9}
          textAnchor="middle"
        >
          {flow.name}
        </text>
      ) : null}
    </g>
  );
}

function MessageFlowPath({ flow }: { flow: MessageFlow }) {
  const model = useModelerStore((state) => state.document.model);
  if (!flow.id) return null;
  const points = resolveWaypoints(flow, model.flowLocationMap[flow.id], model.locationMap);
  if (points.length < 2) return null;
  return <path className="message-flow" d={polylinePath(points)} markerEnd="url(#message-arrow)" />;
}

function AssociationPath({ association }: { association: ArtifactEnum }) {
  const model = useModelerStore((state) => state.document.model);
  if (!association.id) return null;
  const points = resolveWaypoints(
    association,
    model.flowLocationMap[association.id],
    model.locationMap,
  );
  if (points.length < 2) return null;
  return <path className="association-flow" d={polylinePath(points)} />;
}

function resolveWaypoints(
  flow: { sourceRef?: string | null; targetRef?: string | null; waypoints?: GraphicInfo[] },
  diWaypoints: GraphicInfo[] | undefined,
  locations: Record<string, GraphicInfo>,
) {
  if (diWaypoints && diWaypoints.length >= 2) return diWaypoints;
  if (flow.waypoints && flow.waypoints.length >= 2) return flow.waypoints;
  const source = flow.sourceRef ? locations[flow.sourceRef] : undefined;
  const target = flow.targetRef ? locations[flow.targetRef] : undefined;
  if (!source || !target) return [];
  return [center(source), center(target)];
}

function center(bounds: GraphicInfo): GraphicInfo {
  return { ...bounds, x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2 };
}

function polylinePath(points: GraphicInfo[]) {
  return points.map((point, index) => `${index === 0 ? 'M' : 'L'} ${point.x} ${point.y}`).join(' ');
}

function midpoint(points: GraphicInfo[]) {
  const point = points[Math.floor(points.length / 2)] ?? points[0];
  return point ?? { x: 0, y: 0 };
}

function conditionMarker(point: GraphicInfo | undefined) {
  if (!point) return '';
  return `M ${point.x + 6} ${point.y} l 6 -6 l 6 6 l -6 6 Z`;
}

function isSequenceFlow(
  element: FlowElementEnum,
): element is Extract<FlowElementEnum, { elementType: 'sequenceFlow' }> {
  return element.elementType === 'sequenceFlow';
}

function isNode(
  element: FlowElementEnum,
): element is Exclude<FlowElementEnum, { elementType: 'sequenceFlow' }> {
  return element.elementType !== 'sequenceFlow';
}
