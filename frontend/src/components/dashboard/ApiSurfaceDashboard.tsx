import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useAuth } from '../../auth/AuthProvider';
import { cn } from '../../lib/cn';
import { checkBackendHealth, isBackendUnavailableError, requestJson, toWebSocketUrl } from '../../lib/backend';
import { AnimatedBackground, GlassCard, SkeletonCard } from '../base';
import { Badge, Button, Card, CardContent, Input, Label, Separator, Slider } from '../ui';
import { API_ENDPOINTS, CATEGORY_LABELS, RELATIONSHIP_GRAPH_3D_EDGES, RELATIONSHIP_GRAPH_3D_NODES } from './mockApiCatalog';
import { RelationshipGraph3D } from './RelationshipGraph3D';
import { CategoryOperationsModal } from './CategoryOperationsModal';
import type { EndpointCategory, Graph3DEdge, Graph3DNode } from './types';

type CategoryFilter = 'all' | EndpointCategory;

interface ActivityItem {
  id: string;
  text: string;
  at: string;
}

interface RelationshipGraphNodeDto {
  agent_id: string;
  name: string;
  avatar_url?: string | null;
}

interface RelationshipGraphEdgeDto {
  id: number;
  agent_a: string;
  agent_b: string;
  affinity_score: number;
}

interface RelationshipGraphDto {
  nodes: RelationshipGraphNodeDto[];
  edges: RelationshipGraphEdgeDto[];
}

interface TimeScaleDto {
  time_scale: number;
}

interface WsEventItemDto {
  id: number;
  event_type: string;
  agent_id?: string | null;
  description?: string;
  payload?: string;
  occurred_at?: string;
}

interface LifeFeedItem {
  id: number;
  agentId: string | null;
  eventType: string;
  description: string;
  occurredAt: string;
  moodLabel: string | null;
}

interface InspectorDto {
  agent: {
    id: string;
    name: string;
    avatar_url?: string | null;
    personality_json?: Record<string, unknown>;
  };
  state?: {
    mood_label: string;
    valence: number;
    arousal: number;
  } | null;
  recent_events: Array<{
    id: number;
    event_type: string;
    payload?: string;
  }>;
  recent_memories: Array<{
    memory_id: number;
    content: string;
    importance: number;
    created_at: string;
  }>;
}

type WsEventsMessage =
  | { type: 'snapshot'; items: WsEventItemDto[] }
  | { type: 'event_appended'; item: WsEventItemDto }
  | { type: 'tick_skipped'; agent_id: string; reason: string }
  | { type: 'error'; message: string };

type WsRelationshipsMessage =
  | { type: 'snapshot'; graph: RelationshipGraphDto }
  | { type: 'edge_updated'; edge: RelationshipGraphEdgeDto }
  | { type: 'error'; message: string };

const ACTIVITY_STORAGE_KEY = 'cyberlife.dashboard.activity.timeline';

const categoryEntries = Object.entries(CATEGORY_LABELS) as Array<[EndpointCategory, string]>;

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

const hashString = (value: string) =>
  Array.from(value).reduce((acc, char) => ((acc * 33) ^ char.charCodeAt(0)) >>> 0, 5381);

const positionNode = (id: string, label: string, index: number, total: number): Graph3DNode => {
  const hash = hashString(id);
  const angle = ((index + 1) / Math.max(total, 1)) * Math.PI * 2 + (hash % 360) * (Math.PI / 180) * 0.2;
  const radius = 85 + (hash % 55);

  return {
    id,
    label,
    x: Math.round(Math.cos(angle) * radius),
    y: Math.round(Math.sin(angle) * 78),
    z: Math.round(((hash >> 3) % 180) - 90)
  };
};

const mapGraphNodes = (nodes: RelationshipGraphNodeDto[]) =>
  [...nodes]
    .sort((a, b) => a.agent_id.localeCompare(b.agent_id))
    .map((node, index, arr) => positionNode(node.agent_id, node.name, index, arr.length));

const mapGraphEdges = (edges: RelationshipGraphEdgeDto[]): Graph3DEdge[] =>
  edges.map((edge) => ({
    id: `edge-${edge.id}`,
    source: edge.agent_a,
    target: edge.agent_b,
    affinity: clamp(edge.affinity_score, -1, 1)
  }));

const ensureGraphNode = (nodes: Graph3DNode[], nodeId: string) => {
  if (nodes.some((node) => node.id === nodeId)) return nodes;
  const label = nodeId.length >= 6 ? nodeId.slice(0, 6) : nodeId;
  return [...nodes, positionNode(nodeId, label, nodes.length, nodes.length + 1)];
};

const upsertEdge = (edges: Graph3DEdge[], incoming: Graph3DEdge) => {
  const idx = edges.findIndex((edge) => edge.id === incoming.id);
  if (idx === -1) return [incoming, ...edges];
  return edges.map((edge, index) => (index === idx ? incoming : edge));
};

const parseWsPayload = (raw: string) => {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  if (!(trimmed.startsWith('{') || trimmed.startsWith('['))) return null;
  try {
    return JSON.parse(trimmed) as unknown;
  } catch {
    return null;
  }
};

const pickTimeScale = (payload: unknown) => {
  if (!payload || typeof payload !== 'object') return null;
  const record = payload as Record<string, unknown>;
  const direct = record.time_scale ?? record.timeScale;
  if (typeof direct === 'number' && Number.isFinite(direct)) return direct;
  if (typeof direct === 'string') {
    const parsed = Number(direct);
    if (Number.isFinite(parsed)) return parsed;
  }
  return null;
};

const FEED_LIMIT = 80;

const parseEventPayload = (payload?: string) => {
  if (!payload || typeof payload !== 'string') return null;
  const trimmed = payload.trim();
  if (!trimmed.startsWith('{')) return null;
  try {
    return JSON.parse(trimmed) as Record<string, unknown>;
  } catch {
    return null;
  }
};

const extractMoodLabel = (payload?: string) => {
  const parsed = parseEventPayload(payload);
  if (!parsed) return null;
  const directMood = parsed.mood_label;
  if (typeof directMood === 'string' && directMood.trim().length > 0) return directMood;
  const emotion = parsed.emotion;
  if (!emotion || typeof emotion !== 'object') return null;
  const next = (emotion as Record<string, unknown>).next;
  if (!next || typeof next !== 'object') return null;
  const nextMood = (next as Record<string, unknown>).mood_label;
  return typeof nextMood === 'string' && nextMood.trim().length > 0 ? nextMood : null;
};

const normalizeFeedItem = (item: WsEventItemDto): LifeFeedItem => ({
  id: item.id,
  agentId: item.agent_id ?? null,
  eventType: item.event_type,
  description: item.description?.trim() || item.event_type,
  occurredAt: item.occurred_at ?? new Date().toISOString(),
  moodLabel: extractMoodLabel(item.payload)
});

const initials = (name: string) =>
  name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase() ?? '')
    .join('');

const moodMeta = (moodLabel: string | null) => {
  switch ((moodLabel ?? 'neutral').toLowerCase()) {
    case 'excited':
      return { icon: '^', className: 'bg-amber-500/20 text-amber-300 border-amber-400/50' };
    case 'content':
    case 'calm':
      return { icon: 'o', className: 'bg-emerald-500/20 text-emerald-300 border-emerald-400/50' };
    case 'angry':
      return { icon: '!', className: 'bg-rose-500/20 text-rose-300 border-rose-400/50' };
    case 'sad':
      return { icon: 'v', className: 'bg-blue-500/20 text-blue-300 border-blue-400/50' };
    case 'anxious':
      return { icon: '*', className: 'bg-orange-500/20 text-orange-300 border-orange-400/50' };
    case 'tired':
      return { icon: '-', className: 'bg-slate-500/20 text-slate-300 border-slate-400/50' };
    default:
      return { icon: 'o', className: 'bg-cyan-500/20 text-cyan-300 border-cyan-400/50' };
  }
};

const parseStoredActivity = (): ActivityItem[] => {
  try {
    const raw = window.localStorage.getItem(ACTIVITY_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((item): item is ActivityItem => {
      if (!item || typeof item !== 'object') return false;
      const record = item as Record<string, unknown>;
      return typeof record.id === 'string' && typeof record.text === 'string' && typeof record.at === 'string';
    });
  } catch {
    return [];
  }
};

const readDecisionPlan = (events: InspectorDto['recent_events']) => {
  for (const event of events) {
    if (event.event_type !== 'agent.tick.executed') continue;
    const payload = parseEventPayload(event.payload);
    const pipeline = payload?.decision_pipeline;
    if (!pipeline || typeof pipeline !== 'object') continue;
    const record = pipeline as Record<string, unknown>;
    return {
      reflection: typeof record.reflection === 'string' ? record.reflection : '',
      goal: typeof record.goal === 'string' ? record.goal : '',
      actionPlan: typeof record.action_plan === 'string' ? record.action_plan : '',
      execution: typeof record.execution === 'string' ? record.execution : ''
    };
  }
  return {
    reflection: '',
    goal: '',
    actionPlan: '',
    execution: ''
  };
};

const SectionHint = ({ text }: { text: string }) => (
  <span
    className="inline-flex h-4 w-4 items-center justify-center rounded-full border border-cyan-300/55 text-[10px] font-bold text-cyan-200"
    title={text}
    aria-label={text}
  >
    ?
  </span>
);

export const ApiSurfaceDashboard = () => {
  const { session, logout, refreshNow } = useAuth();
  const searchRef = useRef<HTMLInputElement | null>(null);
  const backendStateRef = useRef<boolean | null>(null);
  const timeScaleSyncTimerRef = useRef<number | null>(null);
  const activityIdRef = useRef(1);

  const [search, setSearch] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<CategoryFilter>('all');
  const [activeCategoryModal, setActiveCategoryModal] = useState<EndpointCategory | null>(null);
  const [graphExpanded, setGraphExpanded] = useState(false);
  const [activityExpanded, setActivityExpanded] = useState(false);
  const [spotlightGraph, setSpotlightGraph] = useState(false);
  const [timeScale, setTimeScale] = useState(1.5);
  const [timeLabel, setTimeLabel] = useState(new Date().toLocaleTimeString());
  const [refreshingSession, setRefreshingSession] = useState(false);
  const [backendOnline, setBackendOnline] = useState<boolean | null>(null);
  const [backendGraphLoaded, setBackendGraphLoaded] = useState(false);
  const [graphNodesLive, setGraphNodesLive] = useState<Graph3DNode[]>([]);
  const [graphEdgesLive, setGraphEdgesLive] = useState<Graph3DEdge[]>([]);
  const [agentProfiles, setAgentProfiles] = useState<
    Record<string, { name: string; avatarUrl: string | null }>
  >({});
  const [lifeFeed, setLifeFeed] = useState<LifeFeedItem[]>([]);
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [inspectorLoading, setInspectorLoading] = useState(false);
  const [inspectorData, setInspectorData] = useState<InspectorDto | null>(null);
  const [inspectorError, setInspectorError] = useState<string | null>(null);
  const [eventTargetAgentId, setEventTargetAgentId] = useState<string>('');
  const [eventDescription, setEventDescription] = useState('');
  const [messageSenderId, setMessageSenderId] = useState('');
  const [messageReceiverId, setMessageReceiverId] = useState('');
  const [messageContent, setMessageContent] = useState('');
  const [controlBusy, setControlBusy] = useState<{ event: boolean; message: boolean }>({
    event: false,
    message: false
  });
  const [activity, setActivity] = useState<ActivityItem[]>(() => {
    const restored = parseStoredActivity();
    if (restored.length > 0) return restored;
    return [{ id: 'boot-0', text: 'Control deck is online', at: new Date().toLocaleTimeString() }];
  });
  const [streamStates, setStreamStates] = useState({
    events: true,
    relationships: true
  });

  const accessToken = session?.tokens.accessToken;

  const pushActivity = useCallback((text: string) => {
    const nextId = `${Date.now()}-${activityIdRef.current}`;
    activityIdRef.current += 1;
    setActivity((prev) => [{ id: nextId, text, at: new Date().toLocaleTimeString() }, ...prev]);
  }, []);

  const mergeAgentProfiles = useCallback((nodes: RelationshipGraphNodeDto[]) => {
    setAgentProfiles((prev) => {
      const next = { ...prev };
      for (const node of nodes) {
        next[node.agent_id] = {
          name: node.name,
          avatarUrl: node.avatar_url ?? null
        };
      }
      return next;
    });
  }, []);

  useEffect(() => {
    try {
      window.localStorage.setItem(ACTIVITY_STORAGE_KEY, JSON.stringify(activity));
    } catch {
      // Ignore storage quota/private mode failures.
    }
  }, [activity]);

  useEffect(() => {
    if (Object.keys(agentProfiles).length > 0) return;
    const bootstrap = RELATIONSHIP_GRAPH_3D_NODES.reduce<
      Record<string, { name: string; avatarUrl: string | null }>
    >((acc, node) => {
      acc[node.id] = { name: node.label, avatarUrl: null };
      return acc;
    }, {});
    setAgentProfiles(bootstrap);
  }, [agentProfiles]);

  useEffect(() => {
    const timer = window.setInterval(() => setTimeLabel(new Date().toLocaleTimeString()), 1000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === '/' && document.activeElement !== searchRef.current) {
        event.preventDefault();
        searchRef.current?.focus();
      }
      if (event.key.toLowerCase() === 'g') setGraphExpanded(true);
      if (event.key.toLowerCase() === 'a') setActivityExpanded(true);
      if (event.key === 'Escape') {
        setGraphExpanded(false);
        setActivityExpanded(false);
        setActiveCategoryModal(null);
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);

  useEffect(() => {
    let disposed = false;

    const probe = async () => {
      const healthy = await checkBackendHealth();
      if (!disposed) setBackendOnline(healthy);
    };

    void probe();
    const interval = window.setInterval(() => void probe(), 15000);

    return () => {
      disposed = true;
      window.clearInterval(interval);
    };
  }, []);

  useEffect(() => {
    if (backendOnline === null) return;
    if (backendStateRef.current === backendOnline) return;
    backendStateRef.current = backendOnline;
    if (backendOnline) {
      pushActivity('Backend online: using live API and WebSocket streams');
      return;
    }
    setBackendGraphLoaded(false);
    pushActivity('Backend unavailable: fallback mock mode enabled');
  }, [backendOnline, pushActivity]);

  const loadTimeScale = useCallback(async () => {
    if (!backendOnline) return;
    try {
      const payload = await requestJson<TimeScaleDto>({
        path: '/simulation/time-scale',
        accessToken
      });
      if (typeof payload.time_scale === 'number') {
        setTimeScale(Number(clamp(payload.time_scale, 0.1, 10).toFixed(2)));
      }
    } catch (error) {
      if (isBackendUnavailableError(error)) {
        setBackendOnline(false);
        return;
      }
      pushActivity(`Time scale sync failed: ${error instanceof Error ? error.message : 'unknown error'}`);
    }
  }, [accessToken, backendOnline, pushActivity]);

  const applyTimeScale = useCallback(
    (nextValue: number) => {
      const next = Number(clamp(nextValue, 0.1, 10).toFixed(2));
      setTimeScale(next);

      if (!backendOnline) return;

      if (timeScaleSyncTimerRef.current !== null) {
        window.clearTimeout(timeScaleSyncTimerRef.current);
      }

      timeScaleSyncTimerRef.current = window.setTimeout(() => {
        void requestJson<TimeScaleDto>({
          path: '/simulation/time-scale',
          method: 'POST',
          body: { time_scale: next },
          accessToken
        })
          .then((payload) => {
            const synced = pickTimeScale(payload);
            if (synced !== null) {
              setTimeScale(Number(clamp(synced, 0.1, 10).toFixed(2)));
            }
          })
          .catch((error) => {
            if (isBackendUnavailableError(error)) {
              setBackendOnline(false);
              return;
            }
            pushActivity(`Time scale update failed: ${error instanceof Error ? error.message : 'unknown error'}`);
          });
      }, 300);
    },
    [accessToken, backendOnline, pushActivity]
  );

  useEffect(
    () => () => {
      if (timeScaleSyncTimerRef.current !== null) {
        window.clearTimeout(timeScaleSyncTimerRef.current);
      }
    },
    []
  );

  const loadRelationshipGraph = useCallback(async () => {
    if (!backendOnline) return;
    try {
      const payload = await requestJson<RelationshipGraphDto>({
        path: '/relationships/graph',
        query: { limit_edges: 300 },
        accessToken
      });
      mergeAgentProfiles(payload.nodes);
      setGraphNodesLive(mapGraphNodes(payload.nodes));
      setGraphEdgesLive(mapGraphEdges(payload.edges));
      setBackendGraphLoaded(true);
    } catch (error) {
      if (isBackendUnavailableError(error)) {
        setBackendOnline(false);
        return;
      }
      pushActivity(`Graph snapshot failed: ${error instanceof Error ? error.message : 'unknown error'}`);
    }
  }, [accessToken, backendOnline, mergeAgentProfiles, pushActivity]);

  useEffect(() => {
    if (!backendOnline) return;
    void loadTimeScale();
    void loadRelationshipGraph();
  }, [backendOnline, loadRelationshipGraph, loadTimeScale]);

  useEffect(() => {
    if (!backendOnline || !streamStates.events) return;

    let socket: WebSocket | null = null;
    let reconnectTimer: number | null = null;
    let reconnectAttempt = 0;
    let cancelled = false;

    const connect = () => {
      if (cancelled) return;

      const socketQuery: Record<string, string | number> = { snapshot_limit: 50 };
      if (accessToken) socketQuery.access_token = accessToken;

      socket = new WebSocket(toWebSocketUrl('/ws/events', socketQuery));

      socket.onopen = () => {
        reconnectAttempt = 0;
        pushActivity('Events stream connected');
      };

      socket.onmessage = (event) => {
        const raw = typeof event.data === 'string' ? event.data : String(event.data);
        const parsed = parseWsPayload(raw);
        if (!parsed || typeof parsed !== 'object') return;
        const message = parsed as WsEventsMessage;
        if (message.type === 'snapshot') {
          const nextFeed = message.items
            .map(normalizeFeedItem)
            .sort((left, right) => right.id - left.id)
            .slice(0, FEED_LIMIT);
          setLifeFeed(nextFeed);
          pushActivity(`Events snapshot loaded: ${message.items.length}`);
          return;
        }
        if (message.type === 'event_appended') {
          const nextItem = normalizeFeedItem(message.item);
          setLifeFeed((prev) => [nextItem, ...prev.filter((item) => item.id !== nextItem.id)].slice(0, FEED_LIMIT));
          const agentShort = nextItem.agentId ? nextItem.agentId.slice(0, 8) : 'global';
          pushActivity(`Event: ${nextItem.eventType} (${agentShort})`);
          return;
        }
        if (message.type === 'tick_skipped') {
          pushActivity(`Tick skipped (${message.agent_id.slice(0, 8)}): ${message.reason}`);
          return;
        }
        if (message.type === 'error') {
          pushActivity(`Events stream error: ${message.message}`);
        }
      };

      socket.onerror = () => {
        socket?.close();
      };

      socket.onclose = () => {
        if (cancelled) return;
        const delay = Math.min(10000, 500 * 2 ** reconnectAttempt);
        reconnectAttempt += 1;
        reconnectTimer = window.setTimeout(connect, delay);
      };
    };

    connect();

    return () => {
      cancelled = true;
      if (reconnectTimer !== null) window.clearTimeout(reconnectTimer);
      socket?.close();
    };
  }, [accessToken, backendOnline, pushActivity, streamStates.events]);

  useEffect(() => {
    if (!backendOnline || !streamStates.relationships) return;

    let socket: WebSocket | null = null;
    let reconnectTimer: number | null = null;
    let reconnectAttempt = 0;
    let cancelled = false;

    const connect = () => {
      if (cancelled) return;

      const socketQuery: Record<string, string | number> = { snapshot_limit: 300 };
      if (accessToken) socketQuery.access_token = accessToken;

      socket = new WebSocket(toWebSocketUrl('/ws/relationships', socketQuery));

      socket.onopen = () => {
        reconnectAttempt = 0;
        pushActivity('Relationship stream connected');
      };

      socket.onmessage = (event) => {
        const raw = typeof event.data === 'string' ? event.data : String(event.data);
        const parsed = parseWsPayload(raw);
        if (!parsed || typeof parsed !== 'object') return;
        const message = parsed as WsRelationshipsMessage;
        if (message.type === 'snapshot') {
          mergeAgentProfiles(message.graph.nodes);
          setGraphNodesLive(mapGraphNodes(message.graph.nodes));
          setGraphEdgesLive(mapGraphEdges(message.graph.edges));
          setBackendGraphLoaded(true);
          pushActivity(`Relationship snapshot loaded: ${message.graph.edges.length} edges`);
          return;
        }
        if (message.type === 'edge_updated') {
          const nextEdge: Graph3DEdge = {
            id: `edge-${message.edge.id}`,
            source: message.edge.agent_a,
            target: message.edge.agent_b,
            affinity: clamp(message.edge.affinity_score, -1, 1)
          };
          setGraphEdgesLive((prev) => upsertEdge(prev, nextEdge));
          setGraphNodesLive((prev) => {
            const withA = ensureGraphNode(prev, message.edge.agent_a);
            return ensureGraphNode(withA, message.edge.agent_b);
          });
          setBackendGraphLoaded(true);
          pushActivity(
            `Edge updated: ${message.edge.agent_a.slice(0, 6)} -> ${message.edge.agent_b.slice(0, 6)} (${message.edge.affinity_score.toFixed(2)})`
          );
          return;
        }
        if (message.type === 'error') {
          pushActivity(`Relationship stream error: ${message.message}`);
        }
      };

      socket.onerror = () => {
        socket?.close();
      };

      socket.onclose = () => {
        if (cancelled) return;
        const delay = Math.min(10000, 500 * 2 ** reconnectAttempt);
        reconnectAttempt += 1;
        reconnectTimer = window.setTimeout(connect, delay);
      };
    };

    connect();

    return () => {
      cancelled = true;
      if (reconnectTimer !== null) window.clearTimeout(reconnectTimer);
      socket?.close();
    };
  }, [accessToken, backendOnline, mergeAgentProfiles, pushActivity, streamStates.relationships]);

  const filteredEndpoints = useMemo(() => {
    const normalized = search.trim().toLowerCase();
    return API_ENDPOINTS.filter((endpoint) => {
      const categoryOk = selectedCategory === 'all' || endpoint.category === selectedCategory;
      const searchOk =
        normalized.length === 0 ||
        endpoint.title.toLowerCase().includes(normalized) ||
        endpoint.summary.toLowerCase().includes(normalized);
      return categoryOk && searchOk;
    });
  }, [search, selectedCategory]);

  const groupedByCategory = useMemo(() => {
    return filteredEndpoints.reduce<Record<EndpointCategory, typeof filteredEndpoints>>(
      (acc, endpoint) => {
        acc[endpoint.category].push(endpoint);
        return acc;
      },
      {
        system: [],
        simulation: [],
        events: [],
        agents: [],
        relationships: [],
        memory: [],
        realtime: []
      }
    );
  }, [filteredEndpoints]);

  const activeStreamsCount = Number(streamStates.events) + Number(streamStates.relationships);
  const accessTtlSeconds = session
    ? Math.max(0, Math.floor((new Date(session.tokens.accessExpiresAt).getTime() - Date.now()) / 1000))
    : 0;

  const graphNodes = backendOnline && backendGraphLoaded ? graphNodesLive : RELATIONSHIP_GRAPH_3D_NODES;
  const graphEdges = backendOnline && backendGraphLoaded ? graphEdgesLive : RELATIONSHIP_GRAPH_3D_EDGES;
  const showDashboardSkeleton = backendOnline !== true && !backendGraphLoaded;

  useEffect(() => {
    if (showDashboardSkeleton) setActiveCategoryModal(null);
  }, [showDashboardSkeleton]);

  const agentDirectory = useMemo(() => {
    const merged = new Map<string, string>();
    for (const [id, profile] of Object.entries(agentProfiles)) {
      merged.set(id, profile.name);
    }
    for (const node of graphNodes) {
      if (!merged.has(node.id)) merged.set(node.id, node.label);
    }
    return Array.from(merged.entries())
      .map(([id, name]) => ({ id, name }))
      .sort((left, right) => left.name.localeCompare(right.name));
  }, [agentProfiles, graphNodes]);

  const resolveAgentDisplay = useCallback(
    (agentId: string | null) => {
      if (!agentId) {
        return { id: null, name: 'System', avatarUrl: null };
      }
      const profile = agentProfiles[agentId];
      return {
        id: agentId,
        name: profile?.name ?? `agent-${agentId.slice(0, 8)}`,
        avatarUrl: profile?.avatarUrl ?? null
      };
    },
    [agentProfiles]
  );

  useEffect(() => {
    if (agentDirectory.length === 0) return;
    if (!messageSenderId) {
      setMessageSenderId(agentDirectory[0].id);
    }
    if (!messageReceiverId) {
      const fallbackReceiver =
        agentDirectory.find((agent) => agent.id !== agentDirectory[0].id)?.id ?? agentDirectory[0].id;
      setMessageReceiverId(fallbackReceiver);
    }
  }, [agentDirectory, messageReceiverId, messageSenderId]);

  const feedPreview = useMemo(() => lifeFeed.slice(0, 14), [lifeFeed]);
  const decisionPlan = useMemo(
    () => (inspectorData ? readDecisionPlan(inspectorData.recent_events) : null),
    [inspectorData]
  );

  const activeCategoryLabel = activeCategoryModal ? CATEGORY_LABELS[activeCategoryModal] : undefined;
  const activeCategoryEndpoints = activeCategoryModal ? groupedByCategory[activeCategoryModal] : [];

  const openInspector = useCallback(
    async (agentId: string) => {
      setSelectedAgentId(agentId);
      setInspectorLoading(true);
      setInspectorError(null);
      try {
        const payload = await requestJson<InspectorDto>({
          path: `/agents/${agentId}/inspector`,
          query: {
            events_limit: 20,
            messages_limit: 10,
            relationships_limit: 10,
            timeline_limit: 20,
            memories_limit: 20,
            recall_top_k: 8
          },
          accessToken
        });
        setInspectorData(payload);
      } catch (error) {
        if (isBackendUnavailableError(error)) {
          setBackendOnline(false);
        }
        setInspectorData(null);
        setInspectorError(error instanceof Error ? error.message : 'Failed to load agent inspector');
      } finally {
        setInspectorLoading(false);
      }
    },
    [accessToken]
  );

  const submitQuickEvent = useCallback(async () => {
    const description = eventDescription.trim();
    if (!description.length) {
      pushActivity('Quick event rejected: description is empty');
      return;
    }

    setControlBusy((prev) => ({ ...prev, event: true }));
    try {
      await requestJson({
        path: '/interventions',
        method: 'POST',
        body: {
          admin_user_id: session?.user.id ?? 'dashboard-admin',
          action: {
            type: 'append_event',
            agent_id: eventTargetAgentId || null,
            event_type: 'manual_event',
            description
          }
        },
        accessToken
      });
      setEventDescription('');
      pushActivity('Manual event added');
    } catch (error) {
      if (isBackendUnavailableError(error)) {
        setBackendOnline(false);
      }
      pushActivity(`Manual event failed: ${error instanceof Error ? error.message : 'unknown error'}`);
    } finally {
      setControlBusy((prev) => ({ ...prev, event: false }));
    }
  }, [accessToken, eventDescription, eventTargetAgentId, pushActivity, session?.user.id]);

  const submitQuickMessage = useCallback(async () => {
    const content = messageContent.trim();
    if (!messageSenderId || !messageReceiverId || !content.length) {
      pushActivity('Quick message rejected: fill sender, receiver and text');
      return;
    }
    if (messageSenderId === messageReceiverId) {
      pushActivity('Quick message rejected: sender and receiver must be different');
      return;
    }

    setControlBusy((prev) => ({ ...prev, message: true }));
    try {
      await requestJson({
        path: '/interventions',
        method: 'POST',
        body: {
          admin_user_id: session?.user.id ?? 'dashboard-admin',
          action: {
            type: 'send_message',
            sender_agent_id: messageSenderId,
            receiver_agent_id: messageReceiverId,
            content
          }
        },
        accessToken
      });
      setMessageContent('');
      pushActivity('Message queued');
    } catch (error) {
      if (isBackendUnavailableError(error)) {
        setBackendOnline(false);
      }
      pushActivity(`Message send failed: ${error instanceof Error ? error.message : 'unknown error'}`);
    } finally {
      setControlBusy((prev) => ({ ...prev, message: false }));
    }
  }, [
    accessToken,
    messageContent,
    messageReceiverId,
    messageSenderId,
    pushActivity,
    session?.user.id
  ]);

  return (
    <div className="relative h-screen overflow-hidden p-3 sm:p-4">
      <AnimatedBackground />
      <div className="dashboard-vignette pointer-events-none absolute inset-0" />

      <div className="relative mx-auto flex h-full max-w-[1750px] flex-col gap-3">
        <GlassCard className="panel-sheen shrink-0 p-3 sm:p-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <h1 className="text-xl font-black text-white sm:text-2xl">CyberLife Control Deck</h1>
              <p className="text-xs text-slate-300/80">Press `/` search, `G` expand graph, `A` expand feed</p>
            </div>
            <div className="flex items-center gap-2">
              <Badge variant="outline">{filteredEndpoints.length} operations</Badge>
              <Badge variant="outline">{activeStreamsCount} streams active</Badge>
              <Badge variant="outline">Access {accessTtlSeconds}s</Badge>
              <Badge variant={backendOnline ? 'secondary' : 'outline'}>
                {backendOnline === null ? 'Backend ...' : backendOnline ? 'Backend Online' : 'Backend Reconnecting'}
              </Badge>
              <Badge variant="secondary">{timeLabel}</Badge>
              {session ? <Badge variant="outline">{session.user.name}</Badge> : null}
              <Button
                size="sm"
                variant="ghost"
                onClick={() => {
                  setRefreshingSession(true);
                  void refreshNow().finally(() => setRefreshingSession(false));
                }}
              >
                {refreshingSession ? 'Refreshing...' : 'Refresh Session'}
              </Button>
              <Button size="sm" variant="ghost" onClick={logout}>
                Logout
              </Button>
            </div>
          </div>

          <Separator className="my-3" />

          <div className="grid grid-cols-1 gap-2 md:grid-cols-[1.2fr_260px_auto]">
            <div className="space-y-1">
              <div className="flex items-center gap-1">
                <Label>Search capabilities</Label>
                <SectionHint text="Фильтр операций по названию и описанию. Быстрый доступ: клавиша /." />
              </div>
              <Input
                ref={searchRef}
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                title="Введите часть названия операции или описание."
              />
            </div>
            <div className="space-y-1">
              <div className="flex items-center gap-1">
                <Label>Scope</Label>
                <SectionHint text="Ограничивает список операций выбранным доменом." />
              </div>
              <select
                value={selectedCategory}
                onChange={(event) => setSelectedCategory(event.target.value as CategoryFilter)}
                className="h-10 w-full rounded-md border border-white/15 bg-slate-900/70 px-3 text-sm text-slate-100"
                title="Выберите домен API: system/simulation/events/agents и т.д."
              >
                <option value="all">All domains</option>
                {categoryEntries.map(([key, label]) => (
                  <option key={key} value={key} className="bg-slate-950">
                    {label}
                  </option>
                ))}
              </select>
            </div>
            <div className="flex items-end gap-2">
              <Button variant={spotlightGraph ? 'secondary' : 'outline'} onClick={() => setSpotlightGraph((prev) => !prev)}>
                {spotlightGraph ? 'Exit Spotlight' : 'Spotlight'}
              </Button>
              <Button variant="outline" onClick={() => setGraphExpanded(true)}>
                Expand Graph
              </Button>
              <Button variant="outline" onClick={() => setActivityExpanded(true)}>
                Expand Feed
              </Button>
            </div>
          </div>
        </GlassCard>

        <div className="relative min-h-0 flex-1">
          <div
            className={cn(
              'grid min-h-0 h-full gap-3',
              spotlightGraph ? 'xl:grid-cols-[minmax(0,1fr)]' : 'xl:grid-cols-[300px_minmax(0,1fr)_360px]'
            )}
          >
          {!spotlightGraph ? (
            <GlassCard className="panel-sheen min-h-0 p-3">
              <div className="flex h-full flex-col gap-2">
                <div>
                  <div className="flex items-center gap-1">
                    <h2 className="text-xs font-semibold uppercase tracking-wide text-slate-200">Domains</h2>
                    <SectionHint text="Список секций API. Нажмите секцию, чтобы открыть доступные операции." />
                  </div>
                  <p className="text-[11px] text-slate-300/70">Open domain popup to run operations.</p>
                </div>

                <div className="dashboard-scroll grid min-h-0 flex-1 gap-2 overflow-auto pr-1">
                  {categoryEntries.map(([category, label]) => {
                    const items = groupedByCategory[category];
                    return (
                      <button
                        key={category}
                        type="button"
                        onClick={() => {
                          setActiveCategoryModal(category);
                          pushActivity(`${label} opened`);
                        }}
                        className="rounded-lg border border-white/10 bg-slate-900/60 p-3 text-left transition-colors hover:bg-slate-800/70"
                      >
                        <div className="flex items-center justify-between gap-2">
                          <div className="text-sm font-semibold text-white">{label}</div>
                          <Badge variant="outline">{items.length}</Badge>
                        </div>
                        <p className="mt-1 text-[11px] text-slate-300/75">Tap to open actions</p>
                      </button>
                    );
                  })}
                </div>
              </div>
            </GlassCard>
          ) : null}

          <div className="grid min-h-0 gap-3 grid-rows-[minmax(0,1fr)_150px]">
            <GlassCard className="panel-sheen min-h-0 p-3">
              <div className="mb-2 flex items-center justify-between">
                <div>
                  <div className="flex items-center gap-1">
                    <h2 className="text-base font-semibold text-white">Relationship Graph</h2>
                    <SectionHint text="Граф связей между агентами. Ребра обновляются в реальном времени." />
                  </div>
                  <p className="text-xs text-slate-300/80">Click a node to open agent inspector. Full view supports rotate.</p>
                </div>
                <Button size="sm" variant="outline" onClick={() => setGraphExpanded(true)}>
                  Full View
                </Button>
              </div>
              <RelationshipGraph3D
                nodes={graphNodes}
                edges={graphEdges}
                interactive={false}
                onNodeSelect={(agentId) => {
                  void openInspector(agentId);
                }}
                className="dashboard-grid-bg h-[calc(100%-42px)]"
              />
            </GlassCard>

            <GlassCard className="panel-sheen p-3">
              <div className="grid h-full grid-cols-1 gap-2 md:grid-cols-2">
                <Card className="border-white/10 bg-slate-900/60">
                  <CardContent className="flex h-full items-center justify-between p-3">
                    <div>
                      <div className="flex items-center gap-1 text-xs text-slate-200">
                        <span>Events Stream</span>
                        <SectionHint text="Поток доменных событий /ws/events. Live/Pause управляет подпиской." />
                      </div>
                      <div className="text-[11px] text-slate-400">instant updates</div>
                    </div>
                    <Button
                      size="sm"
                      variant={streamStates.events ? 'secondary' : 'outline'}
                      onClick={() => {
                        setStreamStates((prev) => {
                          const nextValue = !prev.events;
                          pushActivity(`Events stream ${nextValue ? 'resumed' : 'paused'}`);
                          return { ...prev, events: nextValue };
                        });
                      }}
                    >
                      {streamStates.events ? 'Live' : 'Paused'}
                    </Button>
                  </CardContent>
                </Card>
                <Card className="border-white/10 bg-slate-900/60">
                  <CardContent className="flex h-full items-center justify-between p-3">
                    <div>
                      <div className="flex items-center gap-1 text-xs text-slate-200">
                        <span>Relationship Stream</span>
                        <SectionHint text="Поток обновлений связей /ws/relationships. Обновляет ребра графа." />
                      </div>
                      <div className="text-[11px] text-slate-400">edge updates</div>
                    </div>
                    <Button
                      size="sm"
                      variant={streamStates.relationships ? 'secondary' : 'outline'}
                      onClick={() => {
                        setStreamStates((prev) => {
                          const nextValue = !prev.relationships;
                          pushActivity(`Relationship stream ${nextValue ? 'resumed' : 'paused'}`);
                          return { ...prev, relationships: nextValue };
                        });
                      }}
                    >
                      {streamStates.relationships ? 'Live' : 'Paused'}
                    </Button>
                  </CardContent>
                </Card>
              </div>
            </GlassCard>
          </div>

          {!spotlightGraph ? (
            <GlassCard className="panel-sheen min-h-0 p-3">
              <div className="flex h-full flex-col gap-3">
                <div>
                  <div className="flex items-center gap-1">
                    <h2 className="text-xs font-semibold uppercase tracking-wide text-slate-200">Simulation Tempo</h2>
                    <SectionHint text="Регулирует скорость фоновой симуляции (тики, доставка сообщений, mood decay)." />
                  </div>
                  <p className="text-[11px] text-slate-300/75">Affects runtime only.</p>
                </div>

                <Card className="border-white/10 bg-slate-900/60">
                  <CardContent className="space-y-2 p-3">
                    <div className="flex items-center justify-between text-sm text-slate-200">
                      <span>Time Scale</span>
                      <span className="font-semibold text-cyan-200">{timeScale.toFixed(2)}x</span>
                    </div>
                    <Slider
                      min={0.1}
                      max={10}
                      step={0.1}
                      value={[timeScale]}
                      onValueChange={(values) => applyTimeScale(values[0] ?? timeScale)}
                    />
                    <div className="grid grid-cols-6 gap-1">
                      {[0.1, 0.5, 1, 2, 5, 10].map((preset) => (
                        <Button key={preset} size="sm" variant="outline" className="text-[11px]" onClick={() => applyTimeScale(preset)}>
                          {preset}x
                        </Button>
                      ))}
                    </div>
                  </CardContent>
                </Card>

                <Card className="border-white/10 bg-slate-900/60">
                  <CardContent className="space-y-3 p-3">
                    <div className="flex items-center gap-1">
                      <h3 className="text-sm font-semibold text-white">Control Panel</h3>
                      <SectionHint text="Быстрые действия: добавить событие и отправить сообщение конкретному агенту." />
                    </div>

                    <div className="space-y-2">
                      <Label className="text-xs">Add Event</Label>
                      <select
                        value={eventTargetAgentId}
                        onChange={(event) => setEventTargetAgentId(event.target.value)}
                        className="h-8 w-full rounded-md border border-white/15 bg-slate-900/70 px-2 text-xs text-slate-100"
                      >
                        <option value="">Global event</option>
                        {agentDirectory.map((agent) => (
                          <option key={`event-agent-${agent.id}`} value={agent.id} className="bg-slate-950">
                            {agent.name}
                          </option>
                        ))}
                      </select>
                      <Input
                        value={eventDescription}
                        onChange={(event) => setEventDescription(event.target.value)}
                        placeholder="Example: Found treasure!"
                        className="h-8 text-xs"
                      />
                      <Button size="sm" onClick={() => void submitQuickEvent()} disabled={controlBusy.event}>
                        {controlBusy.event ? 'Adding...' : 'Add Event'}
                      </Button>
                    </div>

                    <Separator />

                    <div className="space-y-2">
                      <Label className="text-xs">Send Message</Label>
                      <div className="grid grid-cols-2 gap-2">
                        <select
                          value={messageSenderId}
                          onChange={(event) => setMessageSenderId(event.target.value)}
                          className="h-8 rounded-md border border-white/15 bg-slate-900/70 px-2 text-xs text-slate-100"
                        >
                          {agentDirectory.map((agent) => (
                            <option key={`sender-${agent.id}`} value={agent.id} className="bg-slate-950">
                              {agent.name}
                            </option>
                          ))}
                        </select>
                        <select
                          value={messageReceiverId}
                          onChange={(event) => setMessageReceiverId(event.target.value)}
                          className="h-8 rounded-md border border-white/15 bg-slate-900/70 px-2 text-xs text-slate-100"
                        >
                          {agentDirectory.map((agent) => (
                            <option key={`receiver-${agent.id}`} value={agent.id} className="bg-slate-950">
                              {agent.name}
                            </option>
                          ))}
                        </select>
                      </div>
                      <Input
                        value={messageContent}
                        onChange={(event) => setMessageContent(event.target.value)}
                        placeholder="Type message to selected agent"
                        className="h-8 text-xs"
                      />
                      <Button size="sm" onClick={() => void submitQuickMessage()} disabled={controlBusy.message}>
                        {controlBusy.message ? 'Sending...' : 'Send Message'}
                      </Button>
                    </div>
                  </CardContent>
                </Card>

                <Separator />

                <div className="min-h-0 flex flex-1 flex-col">
                  <div className="mb-2 flex items-center justify-between">
                    <div className="flex items-center gap-1">
                      <h3 className="text-sm font-semibold text-white">Life Feed</h3>
                      <SectionHint text="Лента событий агентов в реальном времени: аватар, имя, тип события, текущее настроение." />
                    </div>
                    <div className="flex gap-1">
                      <Button size="sm" variant="ghost" onClick={() => setActivityExpanded(true)}>
                        Open
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => setLifeFeed([])}>
                        Clear
                      </Button>
                    </div>
                  </div>
                  <div className="dashboard-scroll min-h-0 flex-1 space-y-2 overflow-y-auto pr-1">
                    {feedPreview.length === 0 ? (
                      <Card className="border-dashed border-white/20 bg-slate-900/50">
                        <CardContent className="p-3 text-xs text-slate-300/80">No live events yet.</CardContent>
                      </Card>
                    ) : (
                      feedPreview.map((item) => {
                        const agent = resolveAgentDisplay(item.agentId);
                        const mood = moodMeta(item.moodLabel);
                        return (
                          <Card key={`feed-${item.id}`} className="border-white/10 bg-slate-900/60">
                            <CardContent className="space-y-2 p-3">
                              <div className="flex items-start justify-between gap-2">
                                <div className="flex items-center gap-2 min-w-0">
                                  <button
                                    type="button"
                                    className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-white/15 bg-slate-800 text-xs font-bold text-slate-100"
                                    onClick={() => {
                                      if (agent.id) void openInspector(agent.id);
                                    }}
                                  >
                                    {initials(agent.name || 'SY')}
                                  </button>
                                  <div className="min-w-0">
                                    <div className="truncate text-xs font-semibold text-slate-100">{agent.name}</div>
                                    <div className="truncate text-[11px] text-slate-400">{item.eventType}</div>
                                  </div>
                                </div>
                                <span className={cn('rounded border px-2 py-0.5 text-[10px] font-semibold uppercase', mood.className)}>
                                  {mood.icon} {item.moodLabel ?? 'neutral'}
                                </span>
                              </div>
                              <p className="text-xs text-slate-200">{item.description}</p>
                              <p className="text-[11px] text-slate-400">{new Date(item.occurredAt).toLocaleTimeString()}</p>
                            </CardContent>
                          </Card>
                        );
                      })
                    )}
                  </div>
                </div>
              </div>
            </GlassCard>
          ) : null}
          </div>

          {showDashboardSkeleton ? (
            <div className="absolute inset-0 z-20 rounded-2xl bg-slate-950/65 p-3 backdrop-blur-sm">
              <div className={cn('grid h-full gap-3', spotlightGraph ? 'grid-cols-1' : 'xl:grid-cols-[300px_minmax(0,1fr)_360px]')}>
                {!spotlightGraph ? (
                  <div className="space-y-3">
                    <SkeletonCard lines={5} className="h-40" />
                    <SkeletonCard lines={4} className="h-40" />
                    <SkeletonCard lines={4} className="h-40" />
                  </div>
                ) : null}

                <div className="grid min-h-0 gap-3 grid-rows-[minmax(0,1fr)_150px]">
                  <SkeletonCard lines={7} showAvatar={false} className="h-full" />
                  <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
                    <SkeletonCard lines={2} showAvatar={false} className="h-full" />
                    <SkeletonCard lines={2} showAvatar={false} className="h-full" />
                  </div>
                </div>

                {!spotlightGraph ? (
                  <div className="space-y-3">
                    <SkeletonCard lines={4} showAvatar={false} className="h-44" />
                    <SkeletonCard lines={6} showAvatar={false} className="h-[calc(100%-188px)]" />
                  </div>
                ) : null}
              </div>
            </div>
          ) : null}
        </div>
      </div>

      <CategoryOperationsModal
        category={activeCategoryModal}
        categoryLabel={activeCategoryLabel}
        endpoints={activeCategoryEndpoints}
        open={activeCategoryModal !== null && !showDashboardSkeleton}
        onClose={() => setActiveCategoryModal(null)}
        timeScale={timeScale}
        accessToken={accessToken}
        operatorUserId={session?.user.id}
        agentDirectory={agentDirectory}
        onTimeScaleChange={setTimeScale}
        onRun={pushActivity}
      />

      {graphExpanded ? (
        <div
          className="fixed inset-0 z-[95] flex items-center justify-center overflow-y-auto bg-black/75 p-4 backdrop-blur-sm"
          onClick={(event) => {
            if (event.target === event.currentTarget) setGraphExpanded(false);
          }}
        >
          <GlassCard className="h-[92vh] w-[95vw] max-w-[1500px] p-4" onClick={(event) => event.stopPropagation()}>
            <div className="mb-3 flex items-center justify-between">
              <div>
                <div className="flex items-center gap-1">
                  <h3 className="text-lg font-semibold text-white">Relationship Graph</h3>
                  <SectionHint text="Расширенный режим визуализации связей и affinity между агентами." />
                </div>
                <p className="text-xs text-slate-300/80">Detailed scene mode. Double-click graph to reset camera.</p>
              </div>
              <Button variant="ghost" onClick={() => setGraphExpanded(false)}>
                Close
              </Button>
            </div>
            <RelationshipGraph3D
              nodes={graphNodes}
              edges={graphEdges}
              interactive
              onNodeSelect={(agentId) => {
                void openInspector(agentId);
              }}
              className="dashboard-grid-bg h-[calc(92vh-82px)]"
            />
          </GlassCard>
        </div>
      ) : null}

      {activityExpanded ? (
        <div
          className="fixed inset-0 z-[94] flex items-center justify-center overflow-y-auto bg-black/70 p-4 backdrop-blur-sm"
          onClick={(event) => {
            if (event.target === event.currentTarget) setActivityExpanded(false);
          }}
        >
          <GlassCard
            className="my-auto flex h-[82vh] max-h-[88vh] w-[95vw] max-w-[900px] min-h-0 flex-col overflow-hidden p-4"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="mb-3 flex items-center justify-between">
              <div className="flex items-center gap-1">
                <h3 className="text-lg font-semibold text-white">Full Life Feed</h3>
                <SectionHint text="Полная лента событий агентов с текущими mood-маркерами." />
              </div>
              <div className="flex gap-2">
                <Button
                  variant="ghost"
                  onClick={(event) => {
                    event.stopPropagation();
                    setLifeFeed([]);
                  }}
                >
                  Clear
                </Button>
                <Button variant="ghost" onClick={() => setActivityExpanded(false)}>
                  Close
                </Button>
              </div>
            </div>
            <div className="dashboard-scroll min-h-0 flex-1 space-y-2 overflow-y-auto overscroll-contain pr-1">
              {lifeFeed.length === 0 ? (
                <Card className="border-dashed border-white/20 bg-slate-900/50">
                  <CardContent className="p-3 text-sm text-slate-300/80">No live events yet.</CardContent>
                </Card>
              ) : (
                lifeFeed.map((item) => {
                  const agent = resolveAgentDisplay(item.agentId);
                  const mood = moodMeta(item.moodLabel);
                  return (
                    <Card key={`modal-feed-${item.id}`} className="border-white/10 bg-slate-900/60">
                      <CardContent className="space-y-2 p-3">
                        <div className="flex items-start justify-between gap-2">
                          <div className="flex items-center gap-2 min-w-0">
                            <button
                              type="button"
                              className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-white/15 bg-slate-800 text-xs font-bold text-slate-100"
                              onClick={() => {
                                if (agent.id) void openInspector(agent.id);
                              }}
                            >
                              {initials(agent.name || 'SY')}
                            </button>
                            <div className="min-w-0">
                              <div className="truncate text-sm font-semibold text-slate-100">{agent.name}</div>
                              <div className="truncate text-xs text-slate-400">{item.eventType}</div>
                            </div>
                          </div>
                          <span className={cn('rounded border px-2 py-0.5 text-[10px] font-semibold uppercase', mood.className)}>
                            {mood.icon} {item.moodLabel ?? 'neutral'}
                          </span>
                        </div>
                        <p className="text-sm text-slate-200">{item.description}</p>
                        <p className="text-xs text-slate-400">{new Date(item.occurredAt).toLocaleString()}</p>
                      </CardContent>
                    </Card>
                  );
                })
              )}
            </div>
          </GlassCard>
        </div>
      ) : null}

      {selectedAgentId ? (
        <div
          className="fixed inset-0 z-[99] flex items-center justify-center overflow-y-auto bg-black/75 p-4 backdrop-blur-sm"
          onClick={(event) => {
            if (event.target === event.currentTarget) {
              setSelectedAgentId(null);
              setInspectorData(null);
              setInspectorError(null);
            }
          }}
        >
          <GlassCard
            className="my-auto flex w-[95vw] max-w-[980px] min-h-0 flex-col overflow-hidden p-4"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="mb-3 flex items-center justify-between">
              <div>
                <h3 className="text-lg font-semibold text-white">Agent Inspector</h3>
                <p className="text-xs text-slate-300/80">Profile, memories and current plan</p>
              </div>
              <Button
                variant="ghost"
                onClick={() => {
                  setSelectedAgentId(null);
                  setInspectorData(null);
                  setInspectorError(null);
                }}
              >
                Close
              </Button>
            </div>

            {inspectorLoading ? (
              <SkeletonCard lines={8} showAvatar className="h-64" />
            ) : inspectorError ? (
              <Card className="border-rose-500/40 bg-rose-950/40">
                <CardContent className="p-4 text-sm text-rose-200">{inspectorError}</CardContent>
              </Card>
            ) : inspectorData ? (
              <div className="dashboard-scroll min-h-0 flex-1 space-y-3 overflow-y-auto pr-1">
                <Card className="border-white/10 bg-slate-900/60">
                  <CardContent className="space-y-2 p-4">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-3">
                        <div className="inline-flex h-10 w-10 items-center justify-center rounded-xl border border-white/15 bg-slate-800 text-sm font-bold text-slate-100">
                          {initials(inspectorData.agent.name)}
                        </div>
                        <div>
                          <div className="text-sm font-semibold text-white">{inspectorData.agent.name}</div>
                          <div className="text-xs text-slate-400">{inspectorData.agent.id}</div>
                        </div>
                      </div>
                      <span className={cn('rounded border px-2 py-0.5 text-[10px] font-semibold uppercase', moodMeta(inspectorData.state?.mood_label ?? 'neutral').className)}>
                        {moodMeta(inspectorData.state?.mood_label ?? 'neutral').icon} {inspectorData.state?.mood_label ?? 'neutral'}
                      </span>
                    </div>
                    <div className="text-xs text-slate-300">
                      Valence: {inspectorData.state?.valence?.toFixed(2) ?? '0.00'} | Arousal:{' '}
                      {inspectorData.state?.arousal?.toFixed(2) ?? '0.00'}
                    </div>
                  </CardContent>
                </Card>

                <Card className="border-white/10 bg-slate-900/60">
                  <CardContent className="space-y-2 p-4">
                    <div className="text-sm font-semibold text-white">Character</div>
                    <pre className="overflow-x-auto rounded-md border border-white/10 bg-slate-950/60 p-3 text-xs text-slate-200">
                      {JSON.stringify(inspectorData.agent.personality_json ?? {}, null, 2)}
                    </pre>
                  </CardContent>
                </Card>

                <Card className="border-white/10 bg-slate-900/60">
                  <CardContent className="space-y-2 p-4">
                    <div className="text-sm font-semibold text-white">Current Plan</div>
                    <div className="space-y-1 text-xs text-slate-200">
                      <p><span className="text-slate-400">Reflection:</span> {decisionPlan?.reflection || 'n/a'}</p>
                      <p><span className="text-slate-400">Goal:</span> {decisionPlan?.goal || 'n/a'}</p>
                      <p><span className="text-slate-400">Action:</span> {decisionPlan?.actionPlan || 'n/a'}</p>
                      <p><span className="text-slate-400">Execution:</span> {decisionPlan?.execution || 'n/a'}</p>
                    </div>
                  </CardContent>
                </Card>

                <Card className="border-white/10 bg-slate-900/60">
                  <CardContent className="space-y-2 p-4">
                    <div className="text-sm font-semibold text-white">Key Memories</div>
                    <div className="space-y-2">
                      {inspectorData.recent_memories.slice(0, 8).map((memory) => (
                        <div key={memory.memory_id} className="rounded-md border border-white/10 bg-slate-950/50 p-3">
                          <p className="text-xs text-slate-100">{memory.content}</p>
                          <p className="mt-1 text-[11px] text-slate-400">
                            importance {memory.importance.toFixed(2)} | {new Date(memory.created_at).toLocaleString()}
                          </p>
                        </div>
                      ))}
                      {inspectorData.recent_memories.length === 0 ? (
                        <p className="text-xs text-slate-400">No memories yet.</p>
                      ) : null}
                    </div>
                  </CardContent>
                </Card>
              </div>
            ) : (
              <Card className="border-dashed border-white/20 bg-slate-900/50">
                <CardContent className="p-4 text-sm text-slate-300/80">No inspector data.</CardContent>
              </Card>
            )}
          </GlassCard>
        </div>
      ) : null}
    </div>
  );
};
