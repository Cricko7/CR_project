import { useEffect, useRef } from 'react';
import * as d3 from 'd3';
import { GraphSnapshot } from '../types/api';

interface Props {
  graph: GraphSnapshot;
}

export const Graph = ({ graph }: Props) => {
  const svgRef = useRef<SVGSVGElement>(null);

  useEffect(() => {
    if (!svgRef.current || graph.nodes.length === 0) return;

    const svg = d3.select(svgRef.current);
    const width = 400, height = 384;
    svg.selectAll("*").remove();

    const nodes = graph.nodes.map(d => ({ ...d, x: width / 2, y: height / 2 }));
    const links = graph.edges;

    const simulation = d3.forceSimulation(nodes)
      .force('link', d3.forceLink(links).id(d => (d as any).id).distance(80))
      .force('charge', d3.forceManyBody().strength(-200))
      .force('center', d3.forceCenter(width / 2, height / 2));

    const link = svg.append('g')
      .selectAll('line')
      .data(links)
      .enter().append('line')
      .attr('stroke-width', 3)
      .attr('stroke', d => d3.interpolateRdYlGn((d.affinity_score + 1) / 2 || 0.5));

    const node = svg.append('g')
      .selectAll('g')
      .data(nodes)
      .enter().append('g');

    node.append('circle')
      .attr('r', 18)
      .attr('fill', '#10B981')
      .attr('stroke', '#fff')
      .attr('stroke-width', 2);

    node.append('text')
      .attr('dy', 5)
      .attr('text-anchor', 'middle')
      .style('font-weight', 'bold')
      .style('font-size', '12px')
      .text(d => (d as any).name.slice(0, 2));

    simulation.on('tick', () => {
      link
        .attr('x1', d => (d.source as any).x)
        .attr('y1', d => (d.source as any).y)
        .attr('x2', d => (d.target as any).x)
        .attr('y2', d => (d.target as any).y);
      node.attr('transform', d => `translate(${(d as any).x}, ${(d as any).y})`);
    });
  }, [graph]);

  return <svg ref={svgRef} viewBox="0 0 400 384" className="w-full h-full" />;
};

export default Graph;
