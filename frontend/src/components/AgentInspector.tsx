import { useState, useEffect } from 'react';
import { useCyberLife } from '../hooks/useCyberLife';
import { InspectorResponse } from '../types/api';
import { X, MessageSquare, Brain } from 'lucide-react';

interface Props {
  agentId: string;
  onClose: () => void;
}

export const AgentInspector = ({ agentId, onClose }: Props) => {
  const { loadInspector } = useCyberLife();
  const [data, setData] = useState<InspectorResponse | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    loadInspector(agentId).then(setData).finally(() => setLoading(false));
  }, [agentId, loadInspector]);

  if (loading) return (
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm z-50 flex items-center justify-center p-4">
      <div className="bg-white/90 backdrop-blur-xl rounded-3xl p-12 w-full max-w-4xl">
        <div className="animate-pulse space-y-4">
          <div className="h-8 bg-gray-300 rounded w-48"></div>
          <div className="h-4 bg-gray-300 rounded w-96"></div>
        </div>
      </div>
    </div>
  );

  if (!data) return null;

  return (
    <div className="fixed inset-0 bg-black/70 backdrop-blur-sm z-50 flex items-center justify-center p-4">
      <div className="bg-white/95 backdrop-blur-3xl rounded-3xl max-w-6xl max-h-[90vh] overflow-y-auto w-full shadow-2xl border border-white/50">
        <div className="sticky top-0 bg-gradient-to-r from-purple-600 to-indigo-600 text-white p-8 rounded-t-3xl">
          <div className="flex items-center justify-between">
            <div className="flex items-center space-x-4">
              <div className="w-20 h-20 bg-gradient-to-br from-emerald-400 to-teal-500 rounded-3xl flex items-center justify-center shadow-2xl">
                <span className="font-black text-2xl">{data.agent.name.slice(0, 2)}</span>
              </div>
              <div>
                <h1 className="text-4xl font-black">{data.agent.name}</h1>
                <div className="flex items-center space-x-4 text-xl mt-2 opacity-90">
                  <span className="px-4 py-2 bg-white/20 rounded-2xl font-bold">{data.state.mood_label}</span>
                  <span>Valence: {data.state.valence.toFixed(2)}</span>
                  <span>Arousal: {data.state.arousal.toFixed(2)}</span>
                </div>
              </div>
            </div>
            <button onClick={onClose} className="p-2 hover:bg-white/30 rounded-2xl transition-all">
              <X className="w-6 h-6" />
            </button>
          </div>
        </div>

        <div className="p-8 space-y-8">
          <div className="grid grid-cols-2 md:grid-cols-4 gap-6">
            <div className="text-center p-6 bg-gradient-to-br from-blue-50 to-indigo-50 rounded-2xl">
              <div className="text-3xl font-black text-blue-600">{data.summary.events_count}</div>
              <div className="text-sm text-gray-600 mt-1">Событий</div>
            </div>
            <div className="text-center p-6 bg-gradient-to-br from-emerald-50 to-teal-50 rounded-2xl">
              <div className="text-3xl font-black text-emerald-600">{data.summary.messages_count}</div>
              <div className="text-sm text-gray-600 mt-1">Сообщений</div>
            </div>
            <div className="text-center p-6 bg-gradient-to-br from-purple-50 to-violet-50 rounded-2xl">
              <div className="text-3xl font-black text-purple-600">{data.summary.relationships_count}</div>
              <div className="text-sm text-gray-600 mt-1">Отношений</div>
            </div>
            <div className="text-center p-6 bg-gradient-to-br from-amber-50 to-orange-50 rounded-2xl">
              <div className="text-3xl font-black text-amber-600">{data.summary.memories_count}</div>
              <div className="text-sm text-gray-600 mt-1">Памяти</div>
            </div>
          </div>

          <div className="grid grid-cols-1 lg:grid-cols-2 gap-8">
            <div>
              <h3 className="text-2xl font-bold flex items-center space-x-3 mb-6">
                <Brain className="w-8 h-8 text-purple-600" />
                <span>Последние воспоминания</span>
              </h3>
              <div className="space-y-4 max-h-80 overflow-y-auto">
                {data.recent_memories.slice(0, 6).map((mem, i) => (
                  <div key={i} className="p-6 bg-gradient-to-r from-indigo-50 to-purple-50 rounded-2xl border-l-4 border-indigo-400">
                    <p className="font-semibold text-gray-900">{mem.content || '—'}</p>
                    <p className="text-sm text-gray-500 mt-2">{new Date().toLocaleString()}</p>
                  </div>
                ))}
              </div>
            </div>
            <div>
              <h3 className="text-2xl font-bold mb-6">🤝 Отношения</h3>
              <div className="space-y-4">
                {data.recent_relationships.slice(0, 5).map((rel, i) => (
                  <div key={i} className="flex items-center justify-between p-6 rounded-2xl border hover:shadow-md transition-all bg-gradient-to-r from-gray-50 to-gray-100">
                    <div>
                      <div className="font-bold">{rel.agent_b || 'Unknown'}</div>
                      <div className="text-sm text-gray-600">{rel.history_summary || '—'}</div>
                    </div>
                    <div className="text-2xl font-black px-4 py-2 rounded-xl bg-emerald-100 text-emerald-800">
                      {rel.affinity_score?.toFixed(2) || '0.00'}
                    </div>
                  </div>
                ))}
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};
