import { memo, useMemo, useRef, useState, type PointerEventHandler } from 'react';
import { cn } from '../../lib/cn';
import type { Graph3DEdge, Graph3DNode } from './types';

interface ProjectedNode extends Graph3DNode {
  sx: number;
  sy: number;
  scale: number;
  depth: number;
}

export interface RelationshipGraph3DProps {
  nodes: Graph3DNode[];
  edges: Graph3DEdge[];
  interactive?: boolean;
  className?: string;
}

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

export const RelationshipGraph3D = memo(({
  nodes,
  edges,
  interactive = true,
  className
}: RelationshipGraph3DProps) => {
  const [yaw, setYaw] = useState(0.9);
  const [pitch, setPitch] = useState(0.42);
  const [isDragging, setIsDragging] = useState(false);
  const [hoveredNodeId, setHoveredNodeId] = useState<string | null>(null);
  const [hoveredEdgeId, setHoveredEdgeId] = useState<string | null>(null);
  const dragRef = useRef<{ x: number; y: number; yaw: number; pitch: number } | null>(null);

  const { projectedNodes, projectedEdges } = useMemo(() => {
    const width = 640;
    const height = 360;
    const centerX = width / 2;
    const centerY = height / 2;
    const perspective = 420;
    const depthOffset = 300;
    const sinY = Math.sin(yaw);
    const cosY = Math.cos(yaw);
    const sinX = Math.sin(pitch);
    const cosX = Math.cos(pitch);

    const projected = nodes.map<ProjectedNode>((node) => {
      const rotatedX = node.x * cosY - node.z * sinY;
      const rotatedZ = node.x * sinY + node.z * cosY;
      const tiltedY = node.y * cosX - rotatedZ * sinX;
      const tiltedZ = node.y * sinX + rotatedZ * cosX;
      const depth = tiltedZ + depthOffset;
      const scale = perspective / depth;

      return {
        ...node,
        sx: centerX + rotatedX * scale,
        sy: centerY + tiltedY * scale,
        scale,
        depth
      };
    });

    const byId = new Map(projected.map((node) => [node.id, node]));
    const normalizedEdges = edges
      .map((edge) => {
        const source = byId.get(edge.source);
        const target = byId.get(edge.target);
        if (!source || !target) return null;
        return { edge, source, target };
      })
      .filter((item): item is { edge: Graph3DEdge; source: ProjectedNode; target: ProjectedNode } => item !== null);

    return { projectedNodes: projected, projectedEdges: normalizedEdges };
  }, [edges, nodes, pitch, yaw]);

  const hoveredNode = hoveredNodeId ? projectedNodes.find((node) => node.id === hoveredNodeId) ?? null : null;
  const hoveredEdge = hoveredEdgeId ? edges.find((edge) => edge.id === hoveredEdgeId) ?? null : null;

  const onPointerDown: PointerEventHandler<HTMLDivElement> = (event) => {
    if (!interactive) return;
    dragRef.current = { x: event.clientX, y: event.clientY, yaw, pitch };
    setIsDragging(true);
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const onPointerMove: PointerEventHandler<HTMLDivElement> = (event) => {
    if (!interactive) return;
    if (!dragRef.current) return;
    const dx = event.clientX - dragRef.current.x;
    const dy = event.clientY - dragRef.current.y;
    setYaw(dragRef.current.yaw + dx * 0.008);
    setPitch(clamp(dragRef.current.pitch + dy * 0.006, -1.1, 1.1));
  };

  const onPointerUp: PointerEventHandler<HTMLDivElement> = (event) => {
    if (!interactive) return;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    dragRef.current = null;
    setIsDragging(false);
  };

  return (
    <div
      className={cn(
        'relative h-[22rem] w-full overflow-hidden rounded-xl border border-white/10',
        interactive ? (isDragging ? 'cursor-grabbing' : 'cursor-grab') : 'cursor-default',
        className
      )}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onDoubleClick={() => {
        if (!interactive) return;
        setYaw(0.9);
        setPitch(0.42);
      }}
    >
      <div className="absolute inset-0 bg-[radial-gradient(circle_at_20%_20%,rgba(14,165,233,0.18),transparent_35%),radial-gradient(circle_at_80%_15%,rgba(129,140,248,0.16),transparent_30%),linear-gradient(to_bottom,#020617,#01030b)]" />
      <div className="pointer-events-none absolute left-3 top-3 z-10 max-w-[75%] rounded-md border border-white/15 bg-slate-950/70 px-3 py-2 text-xs text-slate-200">
        {hoveredNode
          ? `Node: ${hoveredNode.label} (${hoveredNode.id})`
          : hoveredEdge
            ? `Edge: ${hoveredEdge.source} -> ${hoveredEdge.target} | affinity ${hoveredEdge.affinity.toFixed(2)}`
            : interactive
              ? 'Drag to rotate. Hover nodes/edges for details. Double-click to reset view.'
              : 'Preview mode. Open full view to rotate.'}
      </div>
      <svg className="relative h-full w-full" viewBox="0 0 640 360" role="img" aria-label="3D relationship graph mock">
        <g>
          {projectedEdges.map(({ edge, source, target }) => {
            const affinity = clamp((edge.affinity + 1) / 2, 0, 1);
            const hue = 8 + affinity * 130;
            const alpha = clamp((source.scale + target.scale) * 0.4, 0.2, 0.95);
            const highlighted = hoveredEdgeId === edge.id;
            return (
              <line
                key={edge.id}
                x1={source.sx}
                y1={source.sy}
                x2={target.sx}
                y2={target.sy}
                stroke={`hsla(${hue}, 90%, ${highlighted ? 72 : 62}%, ${highlighted ? 1 : alpha})`}
                strokeWidth={(highlighted ? 3 : 1.2) + affinity * 2.4}
                onMouseEnter={() => {
                  if (!interactive) return;
                  if (!isDragging) setHoveredEdgeId(edge.id);
                }}
                onMouseLeave={() => setHoveredEdgeId(null)}
              />
            );
          })}
        </g>
        <g>
          {[...projectedNodes]
            .sort((a, b) => b.depth - a.depth)
            .map((node) => (
              <g key={node.id} transform={`translate(${node.sx}, ${node.sy})`}>
                <circle
                  r={clamp(node.scale * 26, 5, 17)}
                  fill="rgba(15, 23, 42, 0.85)"
                  stroke={hoveredNodeId === node.id ? 'rgba(186,230,253,1)' : 'rgba(103, 232, 249, 0.9)'}
                  strokeWidth={hoveredNodeId === node.id ? 2.2 : 1.3}
                  onMouseEnter={() => {
                    if (!interactive) return;
                    if (!isDragging) setHoveredNodeId(node.id);
                  }}
                  onMouseLeave={() => setHoveredNodeId(null)}
                />
                <text
                  dy="0.34em"
                  textAnchor="middle"
                  fill="rgba(226, 232, 240, 0.95)"
                  style={{ fontSize: `${clamp(node.scale * 14, 8, 12)}px`, fontWeight: 700 }}
                >
                  {node.label.slice(0, 2).toUpperCase()}
                </text>
              </g>
            ))}
        </g>
      </svg>
    </div>
  );
});

RelationshipGraph3D.displayName = 'RelationshipGraph3D';
