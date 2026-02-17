import { useState, useEffect, useCallback } from 'react';
import { checkBackendHealth, requestJson, toWebSocketUrl } from '../lib/backend';
import { authService } from '../auth/authService';
import type { Agent, Event, GraphSnapshot, InspectorResponse } from '../types/api';

export const useCyberLife = () => {
  const [agents, setAgents] = useState<Agent[]>([]);
  const [events, setEvents] = useState<Event[]>([]);
  const [graph, setGraph] = useState<GraphSnapshot>({ nodes: [], edges: [] });
  const getAccessToken = () => authService.getSession()?.tokens.accessToken;

  useEffect(() => {
    let socket: WebSocket | null = null;
    let cancelled = false;

    const connect = async () => {
      const online = await checkBackendHealth();
      if (!online || cancelled) return;

      const accessToken = getAccessToken();
      const wsQuery: Record<string, string | number> = { snapshot_limit: 50 };
      if (accessToken) wsQuery.access_token = accessToken;

      socket = new WebSocket(toWebSocketUrl('/ws/events', wsQuery));
      socket.onmessage = (message) => {
        const raw = typeof message.data === 'string' ? message.data : String(message.data);
        try {
          const payload = JSON.parse(raw) as { type: string; item?: Event; items?: Event[] };
          if (payload.type === 'event_appended' && payload.item) {
            setEvents((prev) => [payload.item!, ...prev.slice(0, 50)]);
            return;
          }
          if (payload.type === 'snapshot' && Array.isArray(payload.items)) {
            setEvents(payload.items);
          }
        } catch {
          // Ignore malformed WS payloads here to keep hook resilient.
        }
      };
    };

    void connect();

    return () => {
      cancelled = true;
      socket?.close();
    };
  }, []);

  const loadAgents = useCallback(async () => {
    const graphPayload = await requestJson<{
      nodes: Array<{ agent_id: string; name: string; created_at?: string }>;
      edges: GraphSnapshot['edges'];
    }>({
      path: '/relationships/graph',
      query: { limit_edges: 300 },
      accessToken: getAccessToken()
    });

    const normalizedAgents: Agent[] = graphPayload.nodes.map((node) => ({
      id: node.agent_id,
      name: node.name,
      created_at: node.created_at ?? new Date().toISOString()
    }));
    setAgents(normalizedAgents);
    setGraph({
      nodes: normalizedAgents,
      edges: graphPayload.edges
    });
  }, []);

  const loadGraph = useCallback(async () => {
    const payload = await requestJson<{
      nodes: Array<{ agent_id: string; name: string; created_at?: string }>;
      edges: GraphSnapshot['edges'];
    }>({
      path: '/relationships/graph',
      query: { limit_edges: 300 },
      accessToken: getAccessToken()
    });

    setGraph({
      nodes: payload.nodes.map((node) => ({
        id: node.agent_id,
        name: node.name,
        created_at: node.created_at ?? new Date().toISOString()
      })),
      edges: payload.edges
    });
  }, []);

  const loadInspector = useCallback(async (agentId: string) => {
    const params = new URLSearchParams({
      events_limit: '20', messages_limit: '10', relationships_limit: '5',
      timeline_limit: '20', memories_limit: '20', recall_top_k: '8'
    });
    return requestJson<InspectorResponse>({
      path: `/agents/${agentId}/inspector`,
      query: Object.fromEntries(params.entries()),
      accessToken: getAccessToken()
    });
  }, []);

  const triggerTick = useCallback(async (agentId: string) => {
    await requestJson({
      path: `/agents/${agentId}/ticks`,
      method: 'POST',
      accessToken: getAccessToken()
    });
  }, []);

  return { agents, events, graph, loadAgents, loadGraph, loadInspector, triggerTick };
};
