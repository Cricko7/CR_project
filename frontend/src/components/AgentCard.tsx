import { Agent } from '../types/api';
import { Zap } from 'lucide-react';
import { useCyberLife } from '../hooks/useCyberLife';

interface Props {
  agent: Agent;
  onInspect: () => void;
}

export const AgentCard = ({ agent, onInspect }: Props) => {
  const { triggerTick } = useCyberLife();

  return (
    <div className="group bg-white/10 backdrop-blur-xl rounded-3xl p-6 border border-white/20 
                    hover:border-white/40 hover:scale-105 transition-all hover:shadow-2xl cursor-pointer"
         onClick={onInspect}>
      <div className="flex items-center justify-between mb-4">
        <div className="flex items-center space-x-4">
          <div className="w-16 h-16 bg-gradient-to-br from-purple-400 to-pink-500 rounded-2xl flex items-center justify-center shadow-lg">
            <span className="font-bold text-xl text-white">{agent.name.slice(0, 2)}</span>
          </div>
          <div>
            <h3 className="font-bold text-xl bg-gradient-to-r from-white to-gray-200 bg-clip-text text-transparent">
              {agent.name}
            </h3>
            <p className="text-sm text-gray-400">AI Agent</p>
          </div>
        </div>
        <button onClick={(e) => { e.stopPropagation(); triggerTick(agent.id); }} 
                className="p-2 hover:bg-white/20 rounded-xl opacity-0 group-hover:opacity-100 transition-all">
          <Zap className="w-5 h-5 text-yellow-400" />
        </button>
      </div>
      <div className="text-sm text-gray-300">ID: {agent.id.slice(0, 8)}...</div>
    </div>
  );
};
