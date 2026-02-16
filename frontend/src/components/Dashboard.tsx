import { useEffect, useState } from 'react';
import { AgentCard } from './AgentCard';
import { AgentInspector } from './AgentInspector';
import { Graph } from './Graph';
import { useCyberLife } from '../hooks/useCyberLife';

export const Dashboard = () => {
  const { agents, events, graph, loadAgents, loadGraph } = useCyberLife();
  const [selectedAgent, setSelectedAgent] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    Promise.all([loadAgents(), loadGraph()]).then(() => setLoading(false));
  }, [loadAgents, loadGraph]);

  if (loading) return <div className="flex items-center justify-center min-h-screen">
    <div className="text-center">
      <div className="w-16 h-16 border-4 border-purple-500 border-t-transparent rounded-full animate-spin mx-auto mb-4" />
      <h1 className="text-3xl font-bold bg-gradient-to-r from-purple-400 to-pink-400 bg-clip-text text-transparent">
        CyberLife Simulator
      </h1>
    </div>
  </div>;

  return (
    <div className="min-h-screen p-8">
      {selectedAgent && <AgentInspector agentId={selectedAgent} onClose={() => setSelectedAgent(null)} />}
      
      <div className="max-w-7xl mx-auto">
        <div className="text-center mb-16">
          <h1 className="text-6xl font-black bg-gradient-to-r from-white via-purple-200 to-pink-200 bg-clip-text text-transparent mb-4">
            CyberLife Simulator
          </h1>
          <p className="text-xl text-purple-200">Хакатон «КИБЕР РЫВОК» 2026</p>
        </div>

        <div className="grid grid-cols-1 lg:grid-cols-2 gap-12 mb-16">
          <div className="bg-white/5 backdrop-blur-xl rounded-3xl p-8 border border-white/20">
            <h2 className="text-3xl font-bold mb-8">🧠 Агенты</h2>
            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
              {agents.map(agent => (
                <AgentCard key={agent.id} agent={agent} onInspect={() => setSelectedAgent(agent.id)} />
              ))}
            </div>
          </div>

          <div className="space-y-8">
            <div className="bg-white/5 backdrop-blur-xl rounded-3xl p-8 border border-white/20 max-h-96 overflow-y-auto">
              <h2 className="text-3xl font-bold mb-6">📡 Live Events</h2>
              <div className="space-y-4">
                {events.slice(0, 15).map(event => (
                  <div key={event.id} className="p-4 bg-white/5 rounded-2xl hover:bg-white/10 transition-all">
                    <div className="font-bold text-purple-300">{event.event_type}</div>
                    <p className="text-sm">{event.description}</p>
                    <p className="text-xs text-gray-400">{new Date(event.occurred_at).toLocaleTimeString()}</p>
                  </div>
                ))}
              </div>
            </div>

            <div className="bg-white/5 backdrop-blur-xl rounded-3xl p-8 border border-white/20">
              <h2 className="text-3xl font-bold mb-6 text-center">🕸️ Граф отношений</h2>
              <div className="h-96"><Graph graph={graph} /></div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};
