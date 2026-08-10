import {
  useRef,
  useState,
  type DragEvent as ReactDragEvent,
  type PointerEvent as ReactPointerEvent,
  type WheelEvent,
} from 'react';

import type {
  ArtifactEnum,
  BpmnEditorDocument,
  BpmnModel,
  FlowElementEnum,
  GraphicInfo,
  MessageFlow,
} from '../generated/editor-protocol';
import { BpmnElement } from './BpmnElement';
import {
  BPMN_PALETTE_MIME,
  createAtPointCommand,
  moveAndReparentElementsCommand,
  nextPaletteElementId,
} from './creationCommands';
import { documentArtifacts, documentElements } from './diagramModel';
import type { CanonicalPaletteElementKind } from './elementFactory';
import {
  alignmentGuideCandidates,
  marqueeElementIds,
  normalizeRect,
  snapPointToGrid,
  type AlignmentGuideCandidate,
  type Point,
  type Rect,
} from './geometry';
import { useModelerStore, type EditorTool } from './modelerStore';
import { resizeElementCommand } from './transformCommands';

const CANVAS_WIDTH = 1400;
const CANVAS_HEIGHT = 620;
const MIN_MARQUEE_DRAG = 3;

interface CanvasClientBounds {
  height: number;
  left: number;
  top: number;
  width: number;
}

interface ViewportTransform {
  x: number;
  y: number;
  zoom: number;
}

interface MarqueeDrag {
  clientStart: Point;
  modelStart: Point;
}

interface ElementDrag {
  clientStart: Point;
  elementIds: string[];
  primaryId: string;
}

interface ResizeDrag {
  clientStart: Point;
  elementId: string;
  height: number;
  width: number;
}

interface CanvasRenderState {
  document: BpmnEditorDocument;
  selectedElementIds: string[];
  tool: EditorTool;
  viewport: ViewportTransform;
}

interface BpmnCanvasProps {
  /** Optional controlled rendering state for read-only previews and deterministic rendering tests. */
  renderState?: CanvasRenderState;
}

export function BpmnCanvas({ renderState }: BpmnCanvasProps = {}) {
  const storeDocument = useModelerStore((state) => state.document);
  const storeViewport = useModelerStore((state) => state.viewport);
  const storeTool = useModelerStore((state) => state.tool);
  const storeSelectedElementIds = useModelerStore((state) => state.selectedElementIds);
  const document = renderState?.document ?? storeDocument;
  const viewport = renderState?.viewport ?? storeViewport;
  const tool = renderState?.tool ?? storeTool;
  const selectedElementIds = renderState?.selectedElementIds ?? storeSelectedElementIds;
  const selectElement = useModelerStore((state) => state.selectElement);
  const selectElements = useModelerStore((state) => state.selectElements);
  const panBy = useModelerStore((state) => state.panBy);
  const zoomBy = useModelerStore((state) => state.zoomBy);
  const execute = useModelerStore((state) => state.execute);
  const panOrigin = useRef<Point | null>(null);
  const marqueeDrag = useRef<MarqueeDrag | null>(null);
  const elementDrag = useRef<ElementDrag | null>(null);
  const resizeDrag = useRef<ResizeDrag | null>(null);
  const [dragPreview, setDragPreview] = useState<{
    elementIds: string[];
    offset: Point;
  } | null>(null);
  const [alignmentGuides, setAlignmentGuides] = useState<AlignmentGuideCandidate[]>([]);
  const [resizePreview, setResizePreview] = useState<{
    elementId: string;
    height: number;
    width: number;
  } | null>(null);
  const [marquee, setMarquee] = useState<Rect | null>(null);

  const elements = documentElements(document);
  const artifacts = documentArtifacts(document);
  const nodes = elements.filter(isNode);
  const flows = elements.filter(isSequenceFlow);
  const associations = artifacts.filter(isAssociation);
  const annotations = artifacts.filter(isTextAnnotation);
  const groups = artifacts.filter(isGroup);
  const selectedIds = new Set(selectedElementIds);
  const resizeTargetId = [...selectedElementIds]
    .reverse()
    .find((elementId) => isResizableElement(document, elementId));

  const handlePointerDown = (event: ReactPointerEvent<SVGSVGElement>) => {
    if (isPanGesture(tool, event.button)) {
      event.preventDefault();
      panOrigin.current = clientPoint(event);
      event.currentTarget.setPointerCapture(event.pointerId);
      return;
    }
    if (tool !== 'pointer' || event.button !== 0) return;

    const elementId = elementIdFromTarget(event.target);
    if (elementId) {
      const additive = isAdditiveSelection(event);
      if (additive) {
        selectElement(elementId, true);
        return;
      }
      const dragIds = selectedIds.has(elementId) ? selectedElementIds : [elementId];
      if (!selectedIds.has(elementId)) selectElement(elementId);
      elementDrag.current = {
        clientStart: clientPoint(event),
        elementIds: dragIds,
        primaryId: elementId,
      };
      event.currentTarget.setPointerCapture(event.pointerId);
      return;
    }

    const clientStart = clientPoint(event);
    const modelStart = clientPointToModel(
      clientStart,
      event.currentTarget.getBoundingClientRect(),
      viewport,
    );
    marqueeDrag.current = { clientStart, modelStart };
    setMarquee({ ...modelStart, width: 0, height: 0 });
    selectElements([]);
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const handlePointerMove = (event: ReactPointerEvent<SVGSVGElement>) => {
    if (resizeDrag.current) {
      const delta = clientDeltaToModel(
        {
          x: event.clientX - resizeDrag.current.clientStart.x,
          y: event.clientY - resizeDrag.current.clientStart.y,
        },
        event.currentTarget.getBoundingClientRect(),
        viewport.zoom,
      );
      setResizePreview({
        elementId: resizeDrag.current.elementId,
        width: Math.max(10, snapPointToGrid({ x: resizeDrag.current.width + delta.x, y: 0 }).x),
        height: Math.max(
          10,
          snapPointToGrid({ x: 0, y: resizeDrag.current.height + delta.y }).y,
        ),
      });
      return;
    }
    if (elementDrag.current) {
      const delta = snapPointToGrid(
        clientDeltaToModel(
          {
            x: event.clientX - elementDrag.current.clientStart.x,
            y: event.clientY - elementDrag.current.clientStart.y,
          },
          event.currentTarget.getBoundingClientRect(),
          viewport.zoom,
        ),
      );
      setDragPreview({ elementIds: elementDrag.current.elementIds, offset: delta });
      setAlignmentGuides(
        dragAlignmentGuides(
          document,
          elementDrag.current.primaryId,
          elementDrag.current.elementIds,
          delta,
        ),
      );
      return;
    }
    if (panOrigin.current) {
      const next = clientPoint(event);
      const delta = clientDeltaToCanvas(
        { x: next.x - panOrigin.current.x, y: next.y - panOrigin.current.y },
        event.currentTarget.getBoundingClientRect(),
      );
      panOrigin.current = next;
      panBy(delta.x, delta.y);
      return;
    }
    if (marqueeDrag.current) {
      const point = clientPointToModel(
        clientPoint(event),
        event.currentTarget.getBoundingClientRect(),
        viewport,
      );
      setMarquee({
        x: marqueeDrag.current.modelStart.x,
        y: marqueeDrag.current.modelStart.y,
        width: point.x - marqueeDrag.current.modelStart.x,
        height: point.y - marqueeDrag.current.modelStart.y,
      });
    }
  };

  const handlePointerUp = (event: ReactPointerEvent<SVGSVGElement>) => {
    if (resizeDrag.current) {
      const preview = resizePreview;
      if (preview) {
        execute(resizeElementCommand(preview.elementId, preview.width, preview.height));
      }
      resizeDrag.current = null;
      setResizePreview(null);
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
      return;
    }
    if (elementDrag.current) {
      const finalOffset = snapPointToGrid(
        clientDeltaToModel(
          {
            x: event.clientX - elementDrag.current.clientStart.x,
            y: event.clientY - elementDrag.current.clientStart.y,
          },
          event.currentTarget.getBoundingClientRect(),
          viewport.zoom,
        ),
      );
      if (finalOffset.x !== 0 || finalOffset.y !== 0) {
        execute(
          moveAndReparentElementsCommand(
            elementDrag.current.elementIds,
            finalOffset.x,
            finalOffset.y,
          ),
        );
      }
      elementDrag.current = null;
      setDragPreview(null);
      setAlignmentGuides([]);
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
      return;
    }
    if (marqueeDrag.current) {
      const clientEnd = clientPoint(event);
      const distance = Math.hypot(
        clientEnd.x - marqueeDrag.current.clientStart.x,
        clientEnd.y - marqueeDrag.current.clientStart.y,
      );
      if (distance >= MIN_MARQUEE_DRAG) {
        const modelEnd = clientPointToModel(
          clientEnd,
          event.currentTarget.getBoundingClientRect(),
          viewport,
        );
        selectElements(
          marqueeElementIds(
            {
              x: marqueeDrag.current.modelStart.x,
              y: marqueeDrag.current.modelStart.y,
              width: modelEnd.x - marqueeDrag.current.modelStart.x,
              height: modelEnd.y - marqueeDrag.current.modelStart.y,
            },
            selectableElementBounds(document),
            'contains',
          ),
        );
      }
    }
    panOrigin.current = null;
    marqueeDrag.current = null;
    setMarquee(null);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const handlePointerCancel = (event: ReactPointerEvent<SVGSVGElement>) => {
    elementDrag.current = null;
    resizeDrag.current = null;
    panOrigin.current = null;
    marqueeDrag.current = null;
    setDragPreview(null);
    setAlignmentGuides([]);
    setResizePreview(null);
    setMarquee(null);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const handleElementDragStart = (elementId: string, event: ReactPointerEvent<SVGGElement>) => {
    if (tool !== 'pointer' || event.button !== 0) return;
    event.stopPropagation();
    const additive = isAdditiveSelection(event);
    if (additive) {
      selectElement(elementId, true);
      return;
    }
    const dragIds = selectedIds.has(elementId) ? selectedElementIds : [elementId];
    if (!selectedIds.has(elementId)) selectElement(elementId);
    elementDrag.current = {
      clientStart: clientPoint(event),
      elementIds: dragIds,
      primaryId: elementId,
    };
    event.currentTarget.ownerSVGElement?.setPointerCapture(event.pointerId);
  };

  const handlePaletteDrop = (event: ReactDragEvent<SVGSVGElement>) => {
    const kind = paletteKind(event.dataTransfer.getData(BPMN_PALETTE_MIME));
    if (!kind) return;
    event.preventDefault();
    const point = clientPointToModel(
      clientPoint(event),
      event.currentTarget.getBoundingClientRect(),
      viewport,
    );
    const elementId = nextPaletteElementId(document, kind);
    execute(createAtPointCommand(kind, elementId, point));
    if (useModelerStore.getState().document.model.locationMap[elementId]) {
      selectElement(elementId);
      useModelerStore.getState().setTool('pointer');
    }
  };

  const handleWheel = (event: WheelEvent<SVGSVGElement>) => {
    event.preventDefault();
    zoomBy(event.deltaY < 0 ? 1.1 : 0.9);
  };

  const handleResizeStart = (
    elementId: string,
    bounds: GraphicInfo,
    event: ReactPointerEvent<SVGRectElement>,
  ) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    resizeDrag.current = {
      clientStart: clientPoint(event),
      elementId,
      height: bounds.height,
      width: bounds.width,
    };
    setResizePreview({ elementId, height: bounds.height, width: bounds.width });
    event.currentTarget.ownerSVGElement?.setPointerCapture(event.pointerId);
  };

  return (
    <div className="canvas-viewport" data-testid="canvas-viewport">
      <svg
        className={`bpmn-canvas tool-${tool}`}
        viewBox={`0 0 ${CANVAS_WIDTH} ${CANVAS_HEIGHT}`}
        role="application"
        aria-label="BPMN process canvas"
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerCancel={handlePointerCancel}
        onDragOver={(event) => {
          if (event.dataTransfer.types.includes(BPMN_PALETTE_MIME)) event.preventDefault();
        }}
        onDrop={handlePaletteDrop}
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
          <marker
            id="association-arrow"
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="7"
            markerHeight="7"
            orient="auto-start-reverse"
          >
            <path className="association-marker" d="M 1 1 L 9 5 L 1 9" />
          </marker>
          <filter id="selection-glow" x="-40%" y="-40%" width="180%" height="180%">
            <feDropShadow dx="0" dy="0" stdDeviation="4" floodColor="#ff5c35" floodOpacity="0.34" />
          </filter>
        </defs>
        <g transform={`translate(${viewport.x} ${viewport.y}) scale(${viewport.zoom})`}>
          <PoolAndLanes model={document.model} selectedIds={selectedIds} />
          <g className="group-layer">
            {groups.map((group) => (
              <GroupShape
                key={group.id ?? group.categoryValueRef ?? 'group'}
                group={group}
                model={document.model}
                selected={Boolean(group.id && selectedIds.has(group.id))}
              />
            ))}
          </g>
          <g className="flow-layer">
            {flows.map((flow) => (
              <FlowPath
                key={flow.id ?? `${flow.sourceRef}-${flow.targetRef}`}
                flow={flow}
                model={document.model}
                selected={Boolean(flow.id && selectedIds.has(flow.id))}
              />
            ))}
            {Object.values(document.model.messageFlows).map((flow) => (
              <MessageFlowPath
                key={flow.id ?? `${flow.sourceRef}-${flow.targetRef}`}
                flow={flow}
                model={document.model}
              />
            ))}
            {associations.map((association) => (
              <AssociationPath
                key={association.id ?? `${association.sourceRef}-${association.targetRef}`}
                association={association}
                model={document.model}
                selected={Boolean(association.id && selectedIds.has(association.id))}
              />
            ))}
          </g>
          <g className="annotation-layer">
            {annotations.map((annotation) => (
              <TextAnnotationShape
                key={annotation.id ?? annotation.text ?? 'annotation'}
                annotation={annotation}
                model={document.model}
                selected={Boolean(annotation.id && selectedIds.has(annotation.id))}
              />
            ))}
          </g>
          <DataStores model={document.model} selectedIds={selectedIds} />
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
                  selected={selectedIds.has(id)}
                  dragOffset={dragPreview?.elementIds.includes(id) ? dragPreview.offset : undefined}
                  onDragStart={handleElementDragStart}
                />
              );
            })}
          </g>
          <AlignmentGuides guides={alignmentGuides} />
          {resizeTargetId && document.model.locationMap[resizeTargetId] ? (
            <ResizeOverlay
              bounds={document.model.locationMap[resizeTargetId]}
              elementId={resizeTargetId}
              preview={resizePreview?.elementId === resizeTargetId ? resizePreview : undefined}
              onResizeStart={handleResizeStart}
            />
          ) : null}
          {marquee ? <MarqueeRect rect={marquee} /> : null}
        </g>
      </svg>
      <div className="canvas-coordinate" aria-hidden="true">
        {Math.round(viewport.zoom * 100)}% · x {Math.round(viewport.x)} · y {Math.round(viewport.y)}
      </div>
    </div>
  );
}

function PoolAndLanes({
  model,
  selectedIds,
}: {
  model: BpmnModel;
  selectedIds: ReadonlySet<string>;
}) {
  return (
    <g className="pool-layer">
      {model.pools.map((pool) => {
        if (!pool.id) return null;
        const bounds = model.locationMap[pool.id];
        if (!bounds) return null;
        return (
          <g
            key={pool.id}
            className={`pool-shape${selectedIds.has(pool.id) ? ' is-selected' : ''}`}
            data-element-id={pool.id}
          >
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
            <g
              key={lane.id}
              className={`lane-shape${selectedIds.has(lane.id) ? ' is-selected' : ''}`}
              data-element-id={lane.id}
            >
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

function DataStores({
  model,
  selectedIds,
}: {
  model: BpmnModel;
  selectedIds: ReadonlySet<string>;
}) {
  return (
    <g className="data-store-layer">
      {Object.values(model.dataStores).map((store) => {
        if (!store.id) return null;
        const bounds = model.locationMap[store.id];
        if (!bounds) return null;
        const centerX = bounds.x + bounds.width / 2;
        return (
          <g
            key={store.id}
            className={`data-store-shape${selectedIds.has(store.id) ? ' is-selected' : ''}`}
            data-element-id={store.id}
          >
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

function GroupShape({
  group,
  model,
  selected,
}: {
  group: Extract<ArtifactEnum, { artifactType: 'group' }>;
  model: BpmnModel;
  selected: boolean;
}) {
  if (!group.id) return null;
  const bounds = model.locationMap[group.id];
  if (!bounds) return null;
  return (
    <g className={`group-shape${selected ? ' is-selected' : ''}`} data-element-id={group.id}>
      <rect x={bounds.x} y={bounds.y} width={bounds.width} height={bounds.height} rx={12} />
      {group.categoryValueRef ? (
        <text x={bounds.x + 12} y={bounds.y + 18}>
          {group.categoryValueRef}
        </text>
      ) : null}
    </g>
  );
}

function TextAnnotationShape({
  annotation,
  model,
  selected,
}: {
  annotation: Extract<ArtifactEnum, { artifactType: 'textAnnotation' }>;
  model: BpmnModel;
  selected: boolean;
}) {
  if (!annotation.id) return null;
  const bounds = model.locationMap[annotation.id];
  if (!bounds) return null;
  const lines = wrapAnnotation(annotation.text ?? '', Math.max(8, Math.floor(bounds.width / 7)));
  return (
    <g
      className={`text-annotation${selected ? ' is-selected' : ''}`}
      data-element-id={annotation.id}
    >
      <path
        d={`M ${bounds.x + 12} ${bounds.y} H ${bounds.x} V ${bounds.y + bounds.height} H ${bounds.x + 12}`}
      />
      <text x={bounds.x + 18} y={bounds.y + 17}>
        {lines.map((line, index) => (
          <tspan key={`${line}-${index}`} x={bounds.x + 18} dy={index === 0 ? 0 : 15}>
            {line}
          </tspan>
        ))}
      </text>
    </g>
  );
}

function wrapAnnotation(text: string, maxCharacters: number) {
  const words = text.trim().split(/\s+/).filter(Boolean);
  const lines: string[] = [];
  for (const word of words) {
    const current = lines.at(-1);
    if (!current || current.length + word.length + 1 > maxCharacters) lines.push(word);
    else lines[lines.length - 1] = `${current} ${word}`;
  }
  return lines.length ? lines : [''];
}

function FlowPath({
  flow,
  model,
  selected,
}: {
  flow: Extract<FlowElementEnum, { elementType: 'sequenceFlow' }>;
  model: BpmnModel;
  selected: boolean;
}) {
  if (!flow.id) return null;
  const points = resolveWaypoints(flow, model.flowLocationMap[flow.id], model.locationMap);
  if (points.length < 2) return null;
  const label = model.labelLocationMap[flow.id];
  return (
    <g className={`sequence-flow${selected ? ' is-selected' : ''}`} data-element-id={flow.id}>
      <path className="flow-hit-target" d={polylinePath(points)} />
      <path className="flow-visual" d={polylinePath(points)} markerEnd="url(#sequence-arrow)" />
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

function MessageFlowPath({ flow, model }: { flow: MessageFlow; model: BpmnModel }) {
  if (!flow.id) return null;
  const points = resolveWaypoints(flow, model.flowLocationMap[flow.id], model.locationMap);
  if (points.length < 2) return null;
  return <path className="message-flow" d={polylinePath(points)} markerEnd="url(#message-arrow)" />;
}

function AssociationPath({
  association,
  model,
  selected,
}: {
  association: Extract<ArtifactEnum, { artifactType: 'association' }>;
  model: BpmnModel;
  selected: boolean;
}) {
  if (!association.id) return null;
  const points = resolveWaypoints(
    association,
    model.flowLocationMap[association.id],
    model.locationMap,
  );
  if (points.length < 2) return null;
  const direction = association.associationDirection?.toLowerCase();
  return (
    <g
      className={`association-flow${selected ? ' is-selected' : ''}`}
      data-element-id={association.id}
    >
      <path className="flow-hit-target" d={polylinePath(points)} />
      <path
        className="flow-visual"
        d={polylinePath(points)}
        markerStart={direction === 'both' ? 'url(#association-arrow)' : undefined}
        markerEnd={
          direction === 'one' || direction === 'both' ? 'url(#association-arrow)' : undefined
        }
      />
    </g>
  );
}

function MarqueeRect({ rect }: { rect: Rect }) {
  const normalized = normalizeRect(rect);
  return (
    <rect
      className="selection-marquee"
      x={normalized.x}
      y={normalized.y}
      width={normalized.width}
      height={normalized.height}
    />
  );
}

function AlignmentGuides({ guides }: { guides: AlignmentGuideCandidate[] }) {
  const xGuide = guides.find((guide) => guide.axis === 'x');
  const yGuide = guides.find((guide) => guide.axis === 'y');
  if (!xGuide && !yGuide) return null;
  return (
    <g className="alignment-guides" aria-hidden="true">
      {xGuide ? <line x1={xGuide.value} y1={-10000} x2={xGuide.value} y2={10000} /> : null}
      {yGuide ? <line x1={-10000} y1={yGuide.value} x2={10000} y2={yGuide.value} /> : null}
    </g>
  );
}

function ResizeOverlay({
  bounds,
  elementId,
  onResizeStart,
  preview,
}: {
  bounds: GraphicInfo;
  elementId: string;
  onResizeStart: (
    elementId: string,
    bounds: GraphicInfo,
    event: ReactPointerEvent<SVGRectElement>,
  ) => void;
  preview?: { height: number; width: number };
}) {
  const width = preview?.width ?? bounds.width;
  const height = preview?.height ?? bounds.height;
  return (
    <g className="resize-overlay" data-resize-element-id={elementId}>
      <rect className="resize-outline" x={bounds.x} y={bounds.y} width={width} height={height} />
      <rect
        className="resize-handle"
        x={bounds.x + width - 6}
        y={bounds.y + height - 6}
        width={12}
        height={12}
        rx={2}
        role="button"
        aria-label={`Resize ${elementId}`}
        onPointerDown={(event) => onResizeStart(elementId, bounds, event)}
      />
    </g>
  );
}

function dragAlignmentGuides(
  document: BpmnEditorDocument,
  primaryId: string,
  movingIds: readonly string[],
  offset: Point,
) {
  const primary = document.model.locationMap[primaryId];
  if (!primary) return [];
  const excluded = new Set(movingIds);
  const candidates = Object.fromEntries(
    Object.entries(document.model.locationMap)
      .filter(([id]) => !excluded.has(id))
      .map(([id, bounds]) => [id, rectOf(bounds)]),
  );
  return alignmentGuideCandidates(
    {
      x: primary.x + offset.x,
      y: primary.y + offset.y,
      width: primary.width,
      height: primary.height,
    },
    candidates,
    5,
  );
}

function clientPointToModel(
  client: Point,
  canvas: CanvasClientBounds,
  viewport: ViewportTransform,
): Point {
  const canvasPoint = clientPointToCanvas(client, canvas);
  return {
    x: (canvasPoint.x - viewport.x) / viewport.zoom,
    y: (canvasPoint.y - viewport.y) / viewport.zoom,
  };
}

function clientDeltaToCanvas(
  delta: Point,
  canvas: Pick<CanvasClientBounds, 'height' | 'width'>,
): Point {
  return {
    x: delta.x * (CANVAS_WIDTH / nonZeroDimension(canvas.width)),
    y: delta.y * (CANVAS_HEIGHT / nonZeroDimension(canvas.height)),
  };
}

function clientDeltaToModel(
  delta: Point,
  canvas: Pick<CanvasClientBounds, 'height' | 'width'>,
  zoom: number,
): Point {
  const canvasDelta = clientDeltaToCanvas(delta, canvas);
  return { x: canvasDelta.x / zoom, y: canvasDelta.y / zoom };
}

function isAdditiveSelection(modifiers: { ctrlKey: boolean; metaKey: boolean }): boolean {
  return modifiers.ctrlKey || modifiers.metaKey;
}

function isPanGesture(tool: EditorTool, button: number): boolean {
  return tool === 'hand' ? button === 0 || button === 1 : button === 1;
}

function selectableElementBounds(document: BpmnEditorDocument): Record<string, Rect> {
  const bounds: Record<string, Rect> = {};
  for (const [id, location] of Object.entries(document.model.locationMap)) {
    bounds[id] = rectOf(location);
  }
  for (const [id, waypoints] of Object.entries(document.model.flowLocationMap)) {
    if (waypoints.length === 0) continue;
    const xs = waypoints.map((point) => point.x);
    const ys = waypoints.map((point) => point.y);
    const minX = Math.min(...xs);
    const minY = Math.min(...ys);
    bounds[id] = {
      x: minX,
      y: minY,
      width: Math.max(...xs) - minX,
      height: Math.max(...ys) - minY,
    };
  }
  return bounds;
}

function clientPointToCanvas(client: Point, canvas: CanvasClientBounds): Point {
  return {
    x: (client.x - canvas.left) * (CANVAS_WIDTH / nonZeroDimension(canvas.width)),
    y: (client.y - canvas.top) * (CANVAS_HEIGHT / nonZeroDimension(canvas.height)),
  };
}

function clientPoint(event: { clientX: number; clientY: number }): Point {
  return { x: event.clientX, y: event.clientY };
}

function elementIdFromTarget(target: EventTarget): string | undefined {
  return target instanceof Element
    ? (target.closest<SVGElement>('[data-element-id]')?.dataset.elementId ?? undefined)
    : undefined;
}

function rectOf(bounds: GraphicInfo): Rect {
  return { x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height };
}

function nonZeroDimension(value: number): number {
  return value > 0 ? value : 1;
}

const paletteKinds = new Set<CanonicalPaletteElementKind>([
  'start',
  'end',
  'userTask',
  'exclusiveGateway',
  'subprocess',
  'boundaryTimer',
  'data',
]);

function paletteKind(value: string): CanonicalPaletteElementKind | null {
  return paletteKinds.has(value as CanonicalPaletteElementKind)
    ? (value as CanonicalPaletteElementKind)
    : null;
}

const resizableFlowTypes = new Set<FlowElementEnum['elementType']>([
  'subProcess',
  'transaction',
  'eventSubProcess',
  'adhocSubProcess',
]);

function isResizableElement(document: BpmnEditorDocument, elementId: string): boolean {
  const flowElement = documentElements(document).find((element) => element.id === elementId);
  if (flowElement && resizableFlowTypes.has(flowElement.elementType)) return true;
  if (document.model.pools.some((pool) => pool.id === elementId)) return true;
  if (
    document.model.processes.some((process) =>
      process.lanes?.some((lane) => lane.id === elementId),
    )
  ) {
    return true;
  }
  return documentArtifacts(document).some(
    (artifact) => artifact.id === elementId && artifact.artifactType === 'group',
  );
}

// These pure functions remain colocated with their sole event consumer because the C2
// ownership boundary permits no additional production module in this change.
// eslint-disable-next-line react-refresh/only-export-components
export const canvasSelectionGeometry = {
  clientDeltaToCanvas,
  clientDeltaToModel,
  clientPointToModel,
  isAdditiveSelection,
  isPanGesture,
  selectableElementBounds,
};

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

function isAssociation(
  artifact: ArtifactEnum,
): artifact is Extract<ArtifactEnum, { artifactType: 'association' }> {
  return artifact.artifactType === 'association';
}

function isTextAnnotation(
  artifact: ArtifactEnum,
): artifact is Extract<ArtifactEnum, { artifactType: 'textAnnotation' }> {
  return artifact.artifactType === 'textAnnotation';
}

function isGroup(
  artifact: ArtifactEnum,
): artifact is Extract<ArtifactEnum, { artifactType: 'group' }> {
  return artifact.artifactType === 'group';
}
