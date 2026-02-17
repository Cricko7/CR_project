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

type WsEventsMessage =
  | { type: 'snapshot'; items: Array<{ id: number; event_type: string; agent_id: string }> }
  | { type: 'event_appended'; item: { id: number; event_type: string; agent_id: string } }
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

  const clearActivity = useCallback(() => {
    setActivity(() => []);
  }, []);

  useEffect(() => {
    try {
      window.localStorage.setItem(ACTIVITY_STORAGE_KEY, JSON.stringify(activity));
    } catch {
      // Ignore storage quota/private mode failures.
    }
  }, [activity]);

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
  }, [accessToken, backendOnline, pushActivity]);

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
          pushActivity(`Events snapshot loaded: ${message.items.length}`);
          return;
        }
        if (message.type === 'event_appended') {
          pushActivity(`Event: ${message.item.event_type} (${message.item.agent_id.slice(0, 8)})`);
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
  }, [accessToken, backendOnline, pushActivity, streamStates.relationships]);

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

  const agentDirectory = useMemo(
    () => graphNodes.map((node) => ({ id: node.id, name: node.label })),
    [graphNodes]
  );

  const activeCategoryLabel = activeCategoryModal ? CATEGORY_LABELS[activeCategoryModal] : undefined;
  const activeCategoryEndpoints = activeCategoryModal ? groupedByCategory[activeCategoryModal] : [];

  return (
    <div className="relative h-screen overflow-hidden p-3 sm:p-4">
      <AnimatedBackground />
      <div className="dashboard-vignette pointer-events-none absolute inset-0" />

      <div className="relative mx-auto flex h-full max-w-[1750px] flex-col gap-3">
        <GlassCard className="panel-sheen shrink-0 p-3 sm:p-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <h1 className="text-xl font-black text-white sm:text-2xl">CyberLife Control Deck</h1>
              <p className="text-xs text-slate-300/80">Press `/` search, `G` expand graph, `A` expand activity</p>
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
              <Label>Search capabilities</Label>
              <Input
                ref={searchRef}
                value={search}
                onChange={(event) => setSearch(event.target.value)}
              />
            </div>
            <div className="space-y-1">
              <Label>Scope</Label>
              <select
                value={selectedCategory}
                onChange={(event) => setSelectedCategory(event.target.value as CategoryFilter)}
                className="h-10 w-full rounded-md border border-white/15 bg-slate-900/70 px-3 text-sm text-slate-100"
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
                Expand Activity
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
                  <h2 className="text-xs font-semibold uppercase tracking-wide text-slate-200">Domains</h2>
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
                  <h2 className="text-base font-semibold text-white">Relationship Graph</h2>
                  <p className="text-xs text-slate-300/80">Drag to rotate. Hover node or edge for details.</p>
                </div>
                <Button size="sm" variant="outline" onClick={() => setGraphExpanded(true)}>
                  Full View
                </Button>
              </div>
              <RelationshipGraph3D
                nodes={graphNodes}
                edges={graphEdges}
                interactive={false}
                className="dashboard-grid-bg h-[calc(100%-42px)]"
              />
            </GlassCard>

            <GlassCard className="panel-sheen p-3">
              <div className="grid h-full grid-cols-1 gap-2 md:grid-cols-2">
                <Card className="border-white/10 bg-slate-900/60">
                  <CardContent className="flex h-full items-center justify-between p-3">
                    <div>
                      <div className="text-xs text-slate-200">Events Stream</div>
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
                      <div className="text-xs text-slate-200">Relationship Stream</div>
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
                  <h2 className="text-xs font-semibold uppercase tracking-wide text-slate-200">Simulation Tempo</h2>
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

                <Separator />

                <div className="min-h-0 flex flex-1 flex-col">
                  <div className="mb-2 flex items-center justify-between">
                    <h3 className="text-sm font-semibold text-white">Activity</h3>
                    <div className="flex gap-1">
                      <Button size="sm" variant="ghost" onClick={() => setActivityExpanded(true)}>
                        Open
                      </Button>
                      <Button size="sm" variant="ghost" onClick={clearActivity}>
                        Clear
                      </Button>
                    </div>
                  </div>
                  <div className="dashboard-scroll min-h-0 flex-1 space-y-2 overflow-y-auto pr-1">
                    {activity.length === 0 ? (
                      <Card className="border-dashed border-white/20 bg-slate-900/50">
                        <CardContent className="p-3 text-xs text-slate-300/80">No activity yet.</CardContent>
                      </Card>
                    ) : (
                      activity.slice(0, 12).map((item) => (
                        <Card key={item.id} className="border-white/10 bg-slate-900/60">
                          <CardContent className="space-y-1 p-3">
                            <p className="text-xs text-slate-100">{item.text}</p>
                            <p className="text-[11px] text-slate-400">{item.at}</p>
                          </CardContent>
                        </Card>
                      ))
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
                <h3 className="text-lg font-semibold text-white">Relationship Graph</h3>
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
              <h3 className="text-lg font-semibold text-white">Full Activity Timeline</h3>
              <div className="flex gap-2">
                <Button
                  variant="ghost"
                  onClick={(event) => {
                    event.stopPropagation();
                    clearActivity();
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
              {activity.length === 0 ? (
                <Card className="border-dashed border-white/20 bg-slate-900/50">
                  <CardContent className="p-3 text-sm text-slate-300/80">No activity yet.</CardContent>
                </Card>
              ) : (
                activity.map((item) => (
                  <Card key={item.id} className="border-white/10 bg-slate-900/60">
                    <CardContent className="space-y-1 p-3">
                      <p className="text-sm text-slate-100">{item.text}</p>
                      <p className="text-xs text-slate-400">{item.at}</p>
                    </CardContent>
                  </Card>
                ))
              )}
            </div>
          </GlassCard>
        </div>
      ) : null}
    </div>
  );
};
