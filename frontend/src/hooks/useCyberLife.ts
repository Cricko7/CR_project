import { useState, useEffect, useCallback } from 'react';
import { io } from 'socket.io-client';
import type { Agent, Event, GraphSnapshot, InspectorResponse } from '../types/api';

const API_BASE = 'http://localhost:8080';

export const useCyberLife = () => {
  const [agents, setAgents] = useState<Agent[]>([]);
  const [events, setEvents] = useState<Event[]>([]);
  const [graph, setGraph] = useState<GraphSnapshot>({ nodes: [], edges: [] });

  useEffect(() => {
    const socket = io(`${API_BASE}/ws/events`);
    socket.on('event_appended', (event: Event) => {
      setEvents(prev => [event, ...prev.slice(0, 50)]);
    });
    return () => { socket.close(); };
  }, []);

  const loadAgents = useCallback(async () => {
    const res = await fetch(`${API_BASE}/agents`);
    const data = await res.json();
    setAgents(Array.isArray(data.items) ? data.items : data);
  }, []);

  const loadGraph = useCallback(async () => {
    const res = await fetch(`${API_BASE}/relationships/graph`);
    setGraph(await res.json());
  }, []);

  const loadInspector = useCallback(async (agentId: string) => {
    const params = new URLSearchParams({
      events_limit: '20', messages_limit: '10', relationships_limit: '5',
      timeline_limit: '20', memories_limit: '20', recall_top_k: '8'
    });
    const res = await fetch(`${API_BASE}/agents/${agentId}/inspector?${params}`);
    return res.json() as Promise<InspectorResponse>;
  }, []);

  const triggerTick = useCallback(async (agentId: string) => {
    await fetch(`${API_BASE}/agents/${agentId}/ticks`, { method: 'POST' });
  }, []);

  return { agents, events, graph, loadAgents, loadGraph, loadInspector, triggerTick };
};
