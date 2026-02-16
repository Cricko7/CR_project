import { useEffect, useMemo, useState } from 'react';
import { AgentCard } from './AgentCard';
import { AgentInspector } from './AgentInspector';
import { Graph } from './Graph';
import { useCyberLife } from '../hooks/useCyberLife';
import {
  AnimatedBackground,
  BentoGrid,
  BentoGridItem,
  GlassCard,
  ModernInput,
  SkeletonCard
} from './base';

export const Dashboard = () => {
  const { agents, events, graph, loadAgents, loadGraph } = useCyberLife();
  const [selectedAgent, setSelectedAgent] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    void Promise.all([loadAgents(), loadGraph()]).finally(() => setLoading(false));
  }, [loadAgents, loadGraph]);

  const filteredAgents = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return agents;
    return agents.filter((agent) => agent.name.toLowerCase().includes(normalized));
  }, [agents, query]);

  if (loading) {
    return (
      <div className="relative min-h-screen overflow-hidden p-4 sm:p-8">
        <AnimatedBackground />
        <div className="relative mx-auto max-w-7xl space-y-8">
          <div className="space-y-2 text-center sm:text-left">
            <h1 className="text-4xl font-extrabold tracking-tight sm:text-6xl">
              CyberLife Simulator
            </h1>
            <p className="text-sm text-slate-200/80 sm:text-base">Loading visualization base...</p>
          </div>
          <BentoGrid>
            <BentoGridItem span={2}>
              <SkeletonCard className="min-h-[420px]" lines={5} />
            </BentoGridItem>
            <BentoGridItem className="space-y-6">
              <SkeletonCard className="min-h-[200px]" lines={4} showAvatar={false} />
              <SkeletonCard className="min-h-[200px]" lines={4} showAvatar={false} />
            </BentoGridItem>
          </BentoGrid>
        </div>
      </div>
    );
  }

  return (
    <div className="relative min-h-screen overflow-hidden p-4 sm:p-8">
      <AnimatedBackground />
      {selectedAgent ? (
        <AgentInspector agentId={selectedAgent} onClose={() => setSelectedAgent(null)} />
      ) : null}

      <div className="relative mx-auto max-w-7xl space-y-8">
        <header className="space-y-4 text-center sm:text-left">
          <h1 className="bg-gradient-to-r from-white via-cyan-100 to-indigo-200 bg-clip-text text-4xl font-black text-transparent sm:text-6xl">
            CyberLife Simulator
          </h1>
          <p className="text-sm text-slate-200/80 sm:text-base">Hackathon visual foundation</p>
          <ModernInput
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            label="Search agents"
            className="mx-auto max-w-md sm:mx-0"
          />
        </header>

        <BentoGrid className="items-start">
          <BentoGridItem span={2}>
            <GlassCard className="p-6 sm:p-8">
              <div className="mb-6 flex items-center justify-between gap-4">
                <h2 className="text-2xl font-bold sm:text-3xl">Agents</h2>
                <div className="rounded-full border border-white/10 bg-white/5 px-3 py-1 text-sm text-slate-200">
                  {filteredAgents.length} visible
                </div>
              </div>
              <div className="grid grid-cols-1 gap-5 md:grid-cols-2 xl:grid-cols-3">
                {filteredAgents.map((agent) => (
                  <AgentCard
                    key={agent.id}
                    agent={agent}
                    onInspect={() => setSelectedAgent(agent.id)}
                  />
                ))}
              </div>
              {filteredAgents.length === 0 ? (
                <div className="rounded-2xl border border-white/10 bg-white/5 p-6 text-center text-slate-300">
                  No agents match the current filter.
                </div>
              ) : null}
            </GlassCard>
          </BentoGridItem>

          <BentoGridItem className="space-y-6">
            <GlassCard className="max-h-[28rem] overflow-y-auto p-6">
              <h2 className="mb-5 text-2xl font-bold">Live Events</h2>
              <div className="space-y-3">
                {events.slice(0, 15).map((event) => (
                  <div
                    key={event.id}
                    className="rounded-2xl border border-white/10 bg-white/5 p-4 transition-colors hover:bg-white/10"
                  >
                    <div className="font-semibold text-cyan-200">{event.event_type}</div>
                    <p className="text-sm text-slate-100/90">{event.description}</p>
                    <p className="mt-1 text-xs text-slate-300/80">
                      {new Date(event.occurred_at).toLocaleTimeString()}
                    </p>
                  </div>
                ))}
              </div>
            </GlassCard>

            <GlassCard className="p-6">
              <h2 className="mb-4 text-2xl font-bold">Relationship Graph</h2>
              <div className="h-96">
                <Graph graph={graph} />
              </div>
            </GlassCard>
          </BentoGridItem>
        </BentoGrid>
      </div>
    </div>
  );
};
