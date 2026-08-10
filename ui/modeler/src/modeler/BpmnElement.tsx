import type { PointerEvent as ReactPointerEvent } from 'react';

import type { FlowElementEnum, GraphicInfo, StartEvent } from '../generated/editor-protocol';

interface BpmnElementProps {
  element: Exclude<FlowElementEnum, { elementType: 'sequenceFlow' }>;
  bounds: GraphicInfo;
  labelBounds?: GraphicInfo;
  selected: boolean;
  dragOffset?: { x: number; y: number };
  onDragStart: (id: string, event: ReactPointerEvent<SVGGElement>) => void;
}

const taskTypes = new Set<FlowElementEnum['elementType']>([
  'task',
  'userTask',
  'serviceTask',
  'caseServiceTask',
  'sendTask',
  'scriptTask',
  'manualTask',
  'receiveTask',
  'businessRuleTask',
]);

const eventTypes = new Set<FlowElementEnum['elementType']>([
  'startEvent',
  'endEvent',
  'intermediateCatchEvent',
  'intermediateThrowEvent',
  'boundaryEvent',
]);

const gatewayTypes = new Set<FlowElementEnum['elementType']>([
  'exclusiveGateway',
  'parallelGateway',
  'inclusiveGateway',
  'eventBasedGateway',
  'complexGateway',
]);

const subprocessTypes = new Set<FlowElementEnum['elementType']>([
  'subProcess',
  'transaction',
  'eventSubProcess',
  'adhocSubProcess',
]);

export function BpmnElement({
  element,
  bounds,
  labelBounds,
  selected,
  dragOffset,
  onDragStart,
}: BpmnElementProps) {
  const id = element.id ?? element.elementType;
  const className = `diagram-element element-${element.elementType}${selected ? ' is-selected' : ''}`;
  const handlePointerDown = (event: ReactPointerEvent<SVGGElement>) => onDragStart(id, event);
  const transform = dragOffset ? `translate(${dragOffset.x} ${dragOffset.y})` : undefined;

  if (taskTypes.has(element.elementType)) {
    return (
      <g
        className={className}
        data-element-id={id}
        transform={transform}
        onPointerDown={handlePointerDown}
      >
        <rect
          className="element-surface task-surface"
          x={bounds.x}
          y={bounds.y}
          width={bounds.width}
          height={bounds.height}
          rx={10}
        />
        <TaskGlyph elementType={element.elementType} x={bounds.x + 17} y={bounds.y + 18} />
        <WrappedLabel
          text={element.name ?? humanize(element.elementType)}
          x={bounds.x + bounds.width / 2}
          y={bounds.y + bounds.height / 2}
          maxWidth={bounds.width - 34}
        />
      </g>
    );
  }

  if (eventTypes.has(element.elementType)) {
    const radius = Math.min(bounds.width, bounds.height) / 2;
    const centerX = bounds.x + bounds.width / 2;
    const centerY = bounds.y + bounds.height / 2;
    const definitionType = eventDefinitionType(element);
    return (
      <g
        className={className}
        data-element-id={id}
        transform={transform}
        onPointerDown={handlePointerDown}
      >
        <circle
          className="element-surface event-surface"
          cx={centerX}
          cy={centerY}
          r={Math.max(12, radius - 2)}
        />
        {element.elementType === 'endEvent' ? (
          <circle
            className="event-inner end-ring"
            cx={centerX}
            cy={centerY}
            r={Math.max(8, radius - 7)}
          />
        ) : null}
        {element.elementType.includes('intermediate') || element.elementType === 'boundaryEvent' ? (
          <circle className="event-inner" cx={centerX} cy={centerY} r={Math.max(8, radius - 6)} />
        ) : null}
        <EventGlyph
          type={definitionType}
          x={centerX}
          y={centerY}
          radius={radius}
          throwing={
            element.elementType === 'intermediateThrowEvent' || element.elementType === 'endEvent'
          }
        />
        <ExternalLabel
          text={element.name}
          bounds={labelBounds}
          fallback={{ x: centerX, y: bounds.y + bounds.height + 18 }}
        />
      </g>
    );
  }

  if (gatewayTypes.has(element.elementType)) {
    const centerX = bounds.x + bounds.width / 2;
    const centerY = bounds.y + bounds.height / 2;
    const half = Math.min(bounds.width, bounds.height) / 2 - 2;
    return (
      <g
        className={className}
        data-element-id={id}
        transform={transform}
        onPointerDown={handlePointerDown}
      >
        <path
          className="element-surface gateway-surface"
          d={`M ${centerX} ${centerY - half} L ${centerX + half} ${centerY} L ${centerX} ${
            centerY + half
          } L ${centerX - half} ${centerY} Z`}
        />
        <GatewayGlyph type={element.elementType} x={centerX} y={centerY} size={half * 0.5} />
        <ExternalLabel
          text={element.name}
          bounds={labelBounds}
          fallback={{ x: centerX, y: bounds.y + bounds.height + 18 }}
        />
      </g>
    );
  }

  if (subprocessTypes.has(element.elementType)) {
    return (
      <g
        className={className}
        data-element-id={id}
        transform={transform}
        onPointerDown={handlePointerDown}
      >
        <rect
          className="element-surface subprocess-surface"
          x={bounds.x}
          y={bounds.y}
          width={bounds.width}
          height={bounds.height}
          rx={8}
        />
        {element.elementType === 'transaction' ? (
          <rect
            className="subprocess-inner"
            x={bounds.x + 5}
            y={bounds.y + 5}
            width={bounds.width - 10}
            height={bounds.height - 10}
            rx={5}
          />
        ) : null}
        <text className="element-title subprocess-title" x={bounds.x + 16} y={bounds.y + 25}>
          {element.name ?? humanize(element.elementType)}
        </text>
        <rect
          className="collapsed-marker"
          x={bounds.x + bounds.width / 2 - 7}
          y={bounds.y + bounds.height - 19}
          width={14}
          height={14}
        />
        <path
          className="marker-stroke"
          d={`M ${bounds.x + bounds.width / 2 - 4} ${bounds.y + bounds.height - 12} h 8 M ${
            bounds.x + bounds.width / 2
          } ${bounds.y + bounds.height - 16} v 8`}
        />
      </g>
    );
  }

  if (element.elementType === 'callActivity') {
    return (
      <g
        className={className}
        data-element-id={id}
        transform={transform}
        onPointerDown={handlePointerDown}
      >
        <rect
          className="element-surface call-activity-surface"
          x={bounds.x}
          y={bounds.y}
          width={bounds.width}
          height={bounds.height}
          rx={9}
        />
        <WrappedLabel
          text={element.name ?? 'Call activity'}
          x={bounds.x + bounds.width / 2}
          y={bounds.y + bounds.height / 2}
          maxWidth={bounds.width - 28}
        />
      </g>
    );
  }

  return (
    <g
      className={className}
      data-element-id={id}
      transform={transform}
      onPointerDown={handlePointerDown}
    >
      <path
        className="element-surface data-object-surface"
        d={`M ${bounds.x} ${bounds.y} h ${bounds.width - 13} l 13 13 v ${bounds.height - 13} h -${
          bounds.width
        } Z`}
      />
      <path
        className="data-object-fold"
        d={`M ${bounds.x + bounds.width - 13} ${bounds.y} v 13 h 13`}
      />
      <ExternalLabel
        text={element.name}
        bounds={labelBounds}
        fallback={{ x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height + 18 }}
      />
    </g>
  );
}

function eventDefinitionType(element: BpmnElementProps['element']) {
  if (!eventTypes.has(element.elementType)) return undefined;
  const event = element as StartEvent;
  return event.eventDefinitions[0]?.eventDefinitionType;
}

function TaskGlyph({ elementType, x, y }: { elementType: string; x: number; y: number }) {
  if (elementType === 'userTask') {
    return (
      <g className="task-glyph">
        <circle cx={x} cy={y - 4} r={4} />
        <path d={`M ${x - 7} ${y + 7} q 1 -7 7 -7 q 6 0 7 7`} />
      </g>
    );
  }
  if (elementType === 'serviceTask' || elementType === 'caseServiceTask') {
    return (
      <g className="task-glyph gear-glyph">
        <circle cx={x} cy={y} r={7} />
        <circle cx={x} cy={y} r={2.4} />
      </g>
    );
  }
  if (elementType === 'sendTask' || elementType === 'receiveTask') {
    return (
      <path
        className="task-glyph envelope-glyph"
        d={`M ${x - 8} ${y - 6} h 16 v 12 h -16 Z m 0 0 l 8 7 l 8 -7`}
      />
    );
  }
  if (elementType === 'scriptTask') {
    return (
      <path
        className="task-glyph script-glyph"
        d={`M ${x - 7} ${y - 8} h 14 v 16 h -14 Z m 3 4 h 8 m -8 4 h 6`}
      />
    );
  }
  if (elementType === 'businessRuleTask') {
    return (
      <path
        className="task-glyph table-glyph"
        d={`M ${x - 8} ${y - 7} h 16 v 14 h -16 Z m 0 -1 v 5 h 16 m -10 -5 v 15`}
      />
    );
  }
  return (
    <rect className="task-glyph generic-glyph" x={x - 6} y={y - 6} width={12} height={12} rx={2} />
  );
}

function EventGlyph({
  type,
  x,
  y,
  radius,
  throwing,
}: {
  type?: string;
  x: number;
  y: number;
  radius: number;
  throwing: boolean;
}) {
  const className = `event-glyph${throwing ? ' is-throwing' : ''}`;
  if (type === 'timerEventDefinition') {
    return (
      <g className="event-glyph timer-glyph">
        <circle cx={x} cy={y} r={Math.max(6, radius - 9)} />
        <path
          d={`M ${x} ${y - radius + 11} v ${radius - 11} l ${Math.max(3, radius / 3)} ${Math.max(2, radius / 5)}`}
        />
      </g>
    );
  }
  if (type === 'messageEventDefinition') {
    return (
      <path
        className={className}
        d={`M ${x - radius / 2} ${y - radius / 3} h ${radius} v ${radius / 1.5} h -${radius} Z m 0 0 l ${radius / 2} ${radius / 2.8} l ${radius / 2} -${radius / 2.8}`}
      />
    );
  }
  if (type === 'signalEventDefinition') {
    return (
      <path
        className={className}
        d={`M ${x} ${y - radius / 2} l ${radius / 2} ${radius} h -${radius} Z`}
      />
    );
  }
  if (type === 'errorEventDefinition') {
    return (
      <path
        className={className}
        d={`M ${x + radius / 4} ${y - radius / 2} l -${radius / 2} ${radius / 2.5} l ${radius / 4} ${radius / 8} l -${radius / 4} ${radius / 2} l ${radius / 2} -${radius / 2.5} l -${radius / 4} -${radius / 8} Z`}
      />
    );
  }
  if (type === 'terminateEventDefinition') {
    return (
      <circle className="event-glyph terminate-glyph" cx={x} cy={y} r={Math.max(5, radius - 9)} />
    );
  }
  if (type === 'cancelEventDefinition') {
    return (
      <path
        className={className}
        d={`M ${x - radius / 2.5} ${y - radius / 2.5} l ${radius / 1.25} ${radius / 1.25} M ${x + radius / 2.5} ${y - radius / 2.5} l -${radius / 1.25} ${radius / 1.25}`}
      />
    );
  }
  if (type === 'compensateEventDefinition') {
    return (
      <path
        className={className}
        d={`M ${x} ${y - radius / 2.2} l -${radius / 2.4} ${radius / 2.2} l ${radius / 2.4} ${radius / 2.2} Z M ${x + radius / 2.2} ${y - radius / 2.2} l -${radius / 2.4} ${radius / 2.2} l ${radius / 2.4} ${radius / 2.2} Z`}
      />
    );
  }
  if (type === 'conditionalEventDefinition') {
    return (
      <path
        className={className}
        d={`M ${x - radius / 2.4} ${y - radius / 2} h ${radius / 1.2} v ${radius} h -${radius / 1.2} Z m ${radius / 6} -${radius / 1.35} h ${radius / 1.9} m -${radius / 1.9} ${radius / 4} h ${radius / 1.9} m -${radius / 1.9} ${radius / 4} h ${radius / 1.9}`}
      />
    );
  }
  if (type === 'linkEventDefinition') {
    return (
      <path
        className={className}
        d={`M ${x - radius / 2} ${y - radius / 5} h ${radius / 1.4} v -${radius / 3} l ${radius / 2.2} ${radius / 2} l -${radius / 2.2} ${radius / 2} v -${radius / 3} h -${radius / 1.4} Z`}
      />
    );
  }
  if (type === 'escalationEventDefinition') {
    return (
      <path
        className={className}
        d={`M ${x} ${y - radius / 2} l ${radius / 2.2} ${radius} l -${radius / 2.2} -${radius / 3} l -${radius / 2.2} ${radius / 3} Z`}
      />
    );
  }
  if (type === 'variableListenerEventDefinition') {
    return (
      <path
        className={className}
        d={`M ${x - radius / 2} ${y + radius / 3} v -${radius / 2} m ${radius / 3} ${radius / 2} v -${radius} m ${radius / 3} ${radius} v -${radius / 1.4} m ${radius / 3} ${radius / 1.4} v -${radius / 2.8}`}
      />
    );
  }
  return null;
}

function GatewayGlyph({ type, x, y, size }: { type: string; x: number; y: number; size: number }) {
  if (type === 'parallelGateway') {
    return (
      <path
        className="gateway-glyph"
        d={`M ${x - size} ${y} h ${size * 2} M ${x} ${y - size} v ${size * 2}`}
      />
    );
  }
  if (type === 'inclusiveGateway') {
    return <circle className="gateway-glyph" cx={x} cy={y} r={size} />;
  }
  if (type === 'eventBasedGateway') {
    return (
      <path
        className="gateway-glyph"
        d={`M ${x} ${y - size} l ${size * 0.95} ${size * 0.7} l -${size * 0.36} ${size * 1.12} h -${size * 1.18} l -${size * 0.36} -${size * 1.12} Z`}
      />
    );
  }
  if (type === 'complexGateway') {
    return (
      <path
        className="gateway-glyph"
        d={`M ${x - size} ${y} h ${size * 2} M ${x} ${y - size} v ${size * 2} M ${x - size * 0.72} ${y - size * 0.72} l ${size * 1.44} ${size * 1.44} M ${x + size * 0.72} ${y - size * 0.72} l -${size * 1.44} ${size * 1.44}`}
      />
    );
  }
  return (
    <path
      className="gateway-glyph"
      d={`M ${x - size} ${y - size} l ${size * 2} ${size * 2} M ${x + size} ${y - size} l -${size * 2} ${size * 2}`}
    />
  );
}

function WrappedLabel({
  text,
  x,
  y,
  maxWidth,
}: {
  text: string;
  x: number;
  y: number;
  maxWidth: number;
}) {
  const words = text.split(/\s+/);
  const midpoint = Math.ceil(words.length / 2);
  const lines =
    words.length > 2
      ? [words.slice(0, midpoint).join(' '), words.slice(midpoint).join(' ')]
      : [text];
  return (
    <text
      className="element-title"
      x={x}
      y={y - (lines.length - 1) * 8}
      textAnchor="middle"
      style={{ maxWidth }}
    >
      {lines.map((line, index) => (
        <tspan key={line} x={x} dy={index === 0 ? 0 : 17}>
          {line}
        </tspan>
      ))}
    </text>
  );
}

function ExternalLabel({
  text,
  bounds,
  fallback,
}: {
  text?: string | null;
  bounds?: GraphicInfo;
  fallback: { x: number; y: number };
}) {
  if (!text) return null;
  return (
    <text
      className="external-label"
      x={bounds ? bounds.x + bounds.width / 2 : fallback.x}
      y={bounds ? bounds.y + Math.min(bounds.height, 16) : fallback.y}
      textAnchor="middle"
    >
      {text}
    </text>
  );
}

function humanize(value: string) {
  return value.replace(/([a-z])([A-Z])/g, '$1 $2').replace(/^./, (letter) => letter.toUpperCase());
}
