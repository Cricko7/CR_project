export interface Agent {
  id: string;
  name: string;
  personality_json?: any;
  created_at: string;
}

export interface AgentState {
  agent_id: string;
  mood_label: string;
  valence: number;
  arousal: number;
  updated_at: string;
}

export interface Event {
  id: number;
  agent_id: string;
  event_type: string;
  description: string;
  occurred_at: string;
}

export interface InspectorResponse {
  agent: Agent;
  state: AgentState;
  summary: { events_count: number; messages_count: number; relationships_count: number; memories_count: number };
  recent_events: Event[];
  recent_messages: any[];
  recent_relationships: any[];
  relationship_timeline: any[];
  recent_memories: any[];
}

export interface GraphSnapshot {
  nodes: Agent[];
  edges: any[];
}
