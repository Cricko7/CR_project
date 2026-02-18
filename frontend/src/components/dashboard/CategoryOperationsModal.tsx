import { useEffect, useMemo, useState } from 'react';
import { Badge, Button, Card, CardContent, CardHeader, CardTitle, Input, Label, Separator, Slider } from '../ui';
import { cn } from '../../lib/cn';
import {
  isBackendUnavailableError,
  readSingleWebSocketEvent,
  requestJson,
  resolvePathTemplate,
  type QueryParams
} from '../../lib/backend';
import { AGENT_DIRECTORY } from './mockApiCatalog';
import type { EndpointCategory, EndpointDefinition, EndpointParam } from './types';

interface OperationCardProps {
  endpoint: EndpointDefinition;
  timeScale: number;
  onTimeScaleChange: (value: number) => void;
  accessToken?: string;
  operatorUserId?: string;
  agentDirectory: AgentDirectoryEntry[];
  onRun: (message: string) => void;
}

interface CategoryOperationsModalProps {
  category: EndpointCategory | null;
  categoryLabel?: string;
  endpoints: EndpointDefinition[];
  open: boolean;
  timeScale: number;
  onClose: () => void;
  accessToken?: string;
  operatorUserId?: string;
  agentDirectory?: AgentDirectoryEntry[];
  onTimeScaleChange: (value: number) => void;
  onRun: (message: string) => void;
}

interface AgentInputProps {
  value: string;
  onChange: (next: string) => void;
  className?: string;
  directory: AgentDirectoryEntry[];
  hint?: string;
}

interface AgentDirectoryEntry {
  id: string;
  name: string;
}

interface ParsedBodyDefaults {
  values: Record<string, string>;
  hiddenValues: Record<string, string>;
  template: Record<string, unknown>;
}

const SYSTEM_MANAGED_BODY_KEYS = new Set(['admin_user_id', 'user_id']);
const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const AGENT_ID_KEYS = new Set(['agent_id', 'sender_agent_id', 'receiver_agent_id']);

const isUuid = (value: string) => UUID_PATTERN.test(value.trim());

const findInvalidAgentIdPath = (value: unknown, path = ''): string | null => {
  if (Array.isArray(value)) {
    for (let index = 0; index < value.length; index += 1) {
      const nextPath = `${path}[${index}]`;
      const found = findInvalidAgentIdPath(value[index], nextPath);
      if (found) return found;
    }
    return null;
  }

  if (!value || typeof value !== 'object') return null;
  const record = value as Record<string, unknown>;

  for (const [key, nested] of Object.entries(record)) {
    const nextPath = path ? `${path}.${key}` : key;
    if (AGENT_ID_KEYS.has(key)) {
      if (typeof nested !== 'string' || !isUuid(nested)) return nextPath;
      continue;
    }

    const found = findInvalidAgentIdPath(nested, nextPath);
    if (found) return found;
  }

  return null;
};

const resolveParamHint = (endpoint: EndpointDefinition, param: EndpointParam) => {
  if (param.key === 'id' && endpoint.path.includes('/agents/{id}')) return 'UUID агента, для которого выполняется операция.';
  if (param.key === 'agent_id') return 'Опциональный фильтр по UUID агента.';
  if (param.key === 'limit') return 'Максимальное число элементов в ответе.';
  if (param.key === 'snapshot_limit') return 'Сколько последних записей отдать в начальном snapshot.';
  if (param.key === 'after_id') return 'Курсор для пагинации событий: вернуть записи с id > after_id.';
  if (param.key === 'recall_query') return 'Текст запроса для семантического поиска по памяти агента.';
  if (param.key === 'top_k' || param.key === 'recall_top_k') return 'Сколько самых релевантных результатов вернуть.';
  if (param.key === 'time_scale') return 'Скорость симуляции: 1.0 - нормальная, >1 быстрее, <1 медленнее.';
  return `Параметр запроса: ${param.label}.`;
};

const resolveBodyFieldHint = (key: string) => {
  if (key === 'admin_user_id' || key === 'user_id') return 'Заполняется автоматически из вашей текущей сессии.';
  if (key === 'sender_agent_id') return 'UUID агента-отправителя.';
  if (key === 'receiver_agent_id') return 'UUID агента-получателя.';
  if (key === 'content') return 'Текст сообщения/памяти/события.';
  if (key === 'tick_id') return 'Идемпотентный идентификатор тика. Можно оставить пустым.';
  if (key === 'time_scale') return 'Скорость симуляции в диапазоне [0.1..10.0].';
  return `Поле тела запроса: ${niceKey(key)}.`;
};

const parseDefaultFields = (endpoint: EndpointDefinition): ParsedBodyDefaults => {
  if (!endpoint.defaultBody) return { values: {}, hiddenValues: {}, template: {} };
  try {
    const parsed = JSON.parse(endpoint.defaultBody) as Record<string, unknown>;
    const values: Record<string, string> = {};
    const hiddenValues: Record<string, string> = {};
    Object.keys(parsed).forEach((key) => {
      if (SYSTEM_MANAGED_BODY_KEYS.has(key)) {
        hiddenValues[key] = '';
      } else {
        values[key] = '';
      }
    });

    return { values, hiddenValues, template: parsed };
  } catch {
    return { values: {}, hiddenValues: {}, template: {} };
  }
};

const niceKey = (key: string) =>
  key
    .replace(/_/g, ' ')
    .replace(/\b\w/g, (char) => char.toUpperCase());

const findAgentById = (directory: AgentDirectoryEntry[], id: string) =>
  directory.find((agent) => agent.id.toLowerCase() === id.toLowerCase()) ?? null;

const findAgentByName = (directory: AgentDirectoryEntry[], name: string) =>
  directory.find((agent) => agent.name.toLowerCase() === name.toLowerCase()) ?? null;

const isAgentParam = (endpoint: EndpointDefinition, param: EndpointParam) => {
  if (param.key.includes('agent')) return true;
  if (param.label.toLowerCase().includes('agent')) return true;
  if (param.key === 'id' && endpoint.path.includes('/agents/{id}')) return true;
  return false;
};

const isAgentBodyKey = (key: string) => key.includes('agent') || key.endsWith('_id_agent');

const AgentInput = ({ value, onChange, className, directory, hint }: AgentInputProps) => {
  const [displayValue, setDisplayValue] = useState(() => findAgentById(directory, value)?.name ?? value);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    setDisplayValue(findAgentById(directory, value)?.name ?? value);
  }, [directory, value]);

  const filtered = useMemo(() => {
    const normalized = displayValue.trim().toLowerCase();
    if (!normalized) return directory;
    return directory.filter((agent) => agent.name.toLowerCase().includes(normalized));
  }, [directory, displayValue]);

  return (
    <div className="relative">
      <Input
        value={displayValue}
        className={className}
        title={hint}
        aria-label={hint}
        onFocus={() => setOpen(true)}
        onBlur={() => {
          window.setTimeout(() => setOpen(false), 120);
        }}
        onChange={(event) => {
          const next = event.target.value;
          setDisplayValue(next);
          setOpen(true);

          const byName = findAgentByName(directory, next.trim());
          if (byName) {
            onChange(byName.id);
            return;
          }

          onChange(next);
        }}
      />
      {open ? (
        <div className="dashboard-scroll absolute z-20 mt-1 max-h-40 w-full overflow-auto rounded-md border border-white/15 bg-slate-950/95 p-1 shadow-2xl">
          {filtered.length === 0 ? (
            <div className="px-2 py-2 text-xs text-slate-400">No matching agents</div>
          ) : (
            filtered.map((agent) => (
              <button
                key={agent.id}
                type="button"
                className="flex w-full items-center justify-between rounded px-2 py-1.5 text-left text-xs text-slate-100 hover:bg-white/10"
                onMouseDown={(event) => {
                  event.preventDefault();
                  setDisplayValue(agent.name);
                  onChange(agent.id);
                  setOpen(false);
                }}
              >
                <span>{agent.name}</span>
              </button>
            ))
          )}
        </div>
      ) : null}
    </div>
  );
};

const parseMaybeNumber = (value: unknown) => {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string') {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return null;
};

const coerceBodyValue = (raw: string, template: unknown): unknown => {
  if (!raw.trim()) return template;
  if (typeof template === 'number') {
    const parsed = Number(raw);
    return Number.isFinite(parsed) ? parsed : template;
  }
  if (typeof template === 'boolean') {
    const normalized = raw.trim().toLowerCase();
    return normalized === 'true' || normalized === '1' || normalized === 'yes';
  }
  if (Array.isArray(template) || (template !== null && typeof template === 'object')) {
    try {
      return JSON.parse(raw);
    } catch {
      return template;
    }
  }
  return raw;
};

const buildRequestBody = (
  values: Record<string, string>,
  hiddenValues: Record<string, string>,
  template: Record<string, unknown>
) => {
  const mergedValues = { ...values, ...hiddenValues };
  return Object.keys(template).reduce<Record<string, unknown>>((acc, key) => {
    acc[key] = coerceBodyValue(mergedValues[key] ?? '', template[key]);
    return acc;
  }, {});
};

const pickTimeScale = (payload: unknown) => {
  if (!payload || typeof payload !== 'object') return null;
  const record = payload as Record<string, unknown>;
  return parseMaybeNumber(record.time_scale ?? record.timeScale);
};

const formatErrorMessage = (error: unknown) => (error instanceof Error ? error.message : 'Unexpected error');

const asRecord = (value: unknown): Record<string, unknown> | null => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
};

const asRecordArray = (value: unknown): Record<string, unknown>[] =>
  Array.isArray(value) ? value.map(asRecord).filter((item): item is Record<string, unknown> => item !== null) : [];

const readString = (record: Record<string, unknown>, key: string) =>
  typeof record[key] === 'string' ? String(record[key]).trim() : '';

const readNumber = (record: Record<string, unknown>, key: string) => parseMaybeNumber(record[key]);

const truncate = (text: string, max = 180) => (text.length > max ? `${text.slice(0, max - 1)}...` : text);

const bulletLines = (lines: string[]) => lines.map((line) => `• ${line}`).join('\n');

const summarizeImportantResponse = (endpointId: string, payload: unknown): string | null => {
  const root = asRecord(payload);

  if (!root) {
    if (typeof payload === 'string' && payload.trim().length > 0) return truncate(payload.trim(), 200);
    return null;
  }

  if (endpointId === 'time-scale-set' || endpointId === 'time-scale-get') return null;

  if (endpointId === 'list-messages' || endpointId === 'relationship-history') {
    const items = asRecordArray(root.items);
    const messages = items
      .map((item) => readString(item, 'content'))
      .filter((text) => text.length > 0)
      .slice(0, 4)
      .map((text) => truncate(text, 220));
    if (messages.length > 0) return bulletLines(messages);
    return items.length > 0 ? `Messages: ${items.length}` : null;
  }

  if (endpointId === 'memory-recall') {
    const items = asRecordArray(root.items);
    const memories = items
      .map((item) => readString(item, 'content'))
      .filter((text) => text.length > 0)
      .slice(0, 4)
      .map((text) => truncate(text, 220));
    if (memories.length > 0) return bulletLines(memories);
    return items.length > 0 ? `Matches: ${items.length}` : null;
  }

  if (endpointId === 'events') {
    const items = asRecordArray(root.items);
    const events = items
      .map((item) => {
        const description = readString(item, 'description');
        if (description.length > 0) return description;
        const eventType = readString(item, 'event_type');
        return eventType.length > 0 ? eventType : '';
      })
      .filter((text) => text.length > 0)
      .slice(0, 4)
      .map((text) => truncate(text, 200));
    if (events.length > 0) return bulletLines(events);
    return items.length > 0 ? `Events: ${items.length}` : null;
  }

  if (endpointId === 'agent-state') {
    const mood = readString(root, 'mood_label');
    const valence = readNumber(root, 'valence');
    const arousal = readNumber(root, 'arousal');
    const lines = [
      mood ? `Mood: ${mood}` : '',
      valence !== null ? `Valence: ${valence.toFixed(2)}` : '',
      arousal !== null ? `Arousal: ${arousal.toFixed(2)}` : ''
    ].filter((line) => line.length > 0);
    return lines.length > 0 ? lines.join('\n') : null;
  }

  if (endpointId === 'agent-create') {
    const id = readString(root, 'id');
    const name = readString(root, 'name');
    const lines = [name ? `Created: ${name}` : '', id ? `ID: ${id}` : ''].filter((line) => line.length > 0);
    return lines.length > 0 ? lines.join('\n') : 'Agent created';
  }

  if (endpointId === 'agent-inspector') {
    const summary = asRecord(root.summary);
    if (!summary) return null;
    const eventsCount = readNumber(summary, 'events_count');
    const messagesCount = readNumber(summary, 'messages_count');
    const relationshipsCount = readNumber(summary, 'relationships_count');
    const timelineCount = readNumber(summary, 'timeline_count');
    const memoriesCount = readNumber(summary, 'memories_count');
    const lines = [
      eventsCount !== null ? `Events: ${eventsCount}` : '',
      messagesCount !== null ? `Messages: ${messagesCount}` : '',
      relationshipsCount !== null ? `Relationships: ${relationshipsCount}` : '',
      timelineCount !== null ? `Timeline: ${timelineCount}` : '',
      memoriesCount !== null ? `Memories: ${memoriesCount}` : ''
    ].filter((line) => line.length > 0);
    return lines.length > 0 ? lines.join('   |   ') : null;
  }

  if (endpointId === 'send-message') {
    const id = readNumber(root, 'message_id');
    const status = readString(root, 'status');
    if (id !== null && status.length > 0) return `Message #${id} (${status})`;
    if (id !== null) return `Message #${id}`;
    return status.length > 0 ? status : null;
  }

  if (endpointId === 'relationships-graph') {
    const nodes = asRecordArray(root.nodes);
    const edges = asRecordArray(root.edges);
    return `Graph nodes: ${nodes.length}\nGraph edges: ${edges.length}`;
  }

  if (endpointId === 'memory-append') {
    const memoryId = readNumber(root, 'memory_id');
    const status = readString(root, 'embedding_status');
    if (memoryId !== null && status.length > 0) return `Memory #${memoryId}\nStatus: ${status}`;
    if (memoryId !== null) return `Memory #${memoryId}`;
    return status.length > 0 ? status : null;
  }

  if (endpointId === 'memory-summarize') {
    const created = typeof root.created_summary === 'boolean' ? root.created_summary : null;
    const count = readNumber(root, 'source_count');
    const lines = [
      created !== null ? `Created summary: ${created ? 'yes' : 'no'}` : '',
      count !== null ? `Source entries: ${count}` : ''
    ].filter((line) => line.length > 0);
    return lines.length > 0 ? lines.join('\n') : null;
  }

  if (endpointId === 'memory-process-embeddings') {
    const processed = readNumber(root, 'processed');
    const succeeded = readNumber(root, 'succeeded');
    const failed = readNumber(root, 'failed');
    const lines = [
      processed !== null ? `Processed: ${processed}` : '',
      succeeded !== null ? `Succeeded: ${succeeded}` : '',
      failed !== null ? `Failed: ${failed}` : ''
    ].filter((line) => line.length > 0);
    return lines.length > 0 ? lines.join('\n') : null;
  }

  if (endpointId === 'memory-dead-letter') {
    const items = asRecordArray(root.items);
    if (items.length === 0) return 'Dead-letter queue is empty';
    return `Dead-letter items: ${items.length}`;
  }

  if (endpointId === 'memory-requeue') {
    const memoryId = readNumber(root, 'memory_id');
    const requeued = typeof root.requeued === 'boolean' ? root.requeued : null;
    if (memoryId !== null && requeued !== null) return `Memory #${memoryId}: ${requeued ? 'requeued' : 'not requeued'}`;
    if (memoryId !== null) return `Memory #${memoryId}`;
    return null;
  }

  if (endpointId === 'ws-events') {
    const type = readString(root, 'type');
    if (!type) return null;
    if (type === 'snapshot') {
      const items = asRecordArray(root.items);
      return `Events snapshot: ${items.length}`;
    }
    if (type === 'event_appended') {
      const item = asRecord(root.item);
      const eventType = item ? readString(item, 'event_type') : '';
      const description = item ? readString(item, 'description') : '';
      if (description) return description;
      if (eventType) return `Event: ${eventType}`;
    }
    return `WS event: ${type}`;
  }

  if (endpointId === 'ws-relationships') {
    const type = readString(root, 'type');
    if (!type) return null;
    if (type === 'snapshot') {
      const graph = asRecord(root.graph);
      const nodes = graph ? asRecordArray(graph.nodes) : [];
      const edges = graph ? asRecordArray(graph.edges) : [];
      return `Relationships snapshot\nNodes: ${nodes.length}\nEdges: ${edges.length}`;
    }
    if (type === 'edge_updated') {
      const edge = asRecord(root.edge);
      const source = edge ? readString(edge, 'agent_a') : '';
      const target = edge ? readString(edge, 'agent_b') : '';
      const affinity = edge ? readNumber(edge, 'affinity_score') : null;
      if (source && target && affinity !== null) return `${source} -> ${target}\nAffinity: ${affinity.toFixed(2)}`;
      return 'Relationship edge updated';
    }
    return `WS event: ${type}`;
  }

  if (endpointId === 'health' || endpointId === 'livez') {
    const status = readString(root, 'status');
    return status ? `Status: ${status}` : null;
  }

  return null;
};

const OperationCard = ({
  endpoint,
  timeScale,
  onTimeScaleChange,
  accessToken,
  operatorUserId,
  agentDirectory,
  onRun
}: OperationCardProps) => {
  const bodyDefaults = useMemo(() => parseDefaultFields(endpoint), [endpoint]);
  const visibleParams = useMemo(
    () => (endpoint.params ?? []).filter((param) => !SYSTEM_MANAGED_BODY_KEYS.has(param.key)),
    [endpoint.params]
  );

  const [paramValues, setParamValues] = useState<Record<string, string>>(() =>
    visibleParams.reduce<Record<string, string>>((acc, param) => {
      acc[param.key] = '';
      return acc;
    }, {})
  );
  const [bodyFields, setBodyFields] = useState<Record<string, string>>(bodyDefaults.values);
  const [isRunning, setIsRunning] = useState(false);
  const [lastSummary, setLastSummary] = useState<string | null>(null);
  const [lastError, setLastError] = useState<string | null>(null);

  useEffect(() => {
    setBodyFields(bodyDefaults.values);
  }, [bodyDefaults]);

  const bodyKeys = Object.keys(bodyFields);
  const resolvedHiddenBodyFields = useMemo(() => {
    const next = { ...bodyDefaults.hiddenValues };
    if (operatorUserId) {
      if ('admin_user_id' in next) next.admin_user_id = operatorUserId;
      if ('user_id' in next) next.user_id = operatorUserId;
    }
    return next;
  }, [bodyDefaults.hiddenValues, operatorUserId]);
  const hasAutoBodyFields = Object.keys(resolvedHiddenBodyFields).length > 0;

  const runAction = async () => {
    setIsRunning(true);
    const started = performance.now();

    const pathParams = (endpoint.params ?? [])
      .filter((param) => !SYSTEM_MANAGED_BODY_KEYS.has(param.key))
      .filter((param) => param.kind === 'path')
      .reduce<Record<string, string>>((acc, param) => {
        acc[param.key] = paramValues[param.key] ?? '';
        return acc;
      }, {});

    const queryParams = visibleParams
      .filter((param) => param.kind === 'query')
      .reduce<QueryParams>((acc, param) => {
        const value = paramValues[param.key] ?? '';
        if (!value.trim().length) return acc;
        acc[param.key] = value;
        return acc;
      }, {});

    const resolvedPath = resolvePathTemplate(endpoint.path, pathParams);
    const invalidPathParam = visibleParams.find((param) => {
      if (!isAgentParam(endpoint, param)) return false;
      const value = (paramValues[param.key] ?? '').trim();
      if (!value.length) return false;
      return !isUuid(value);
    });
    if (invalidPathParam) {
      setLastSummary(null);
      setLastError(`Поле "${invalidPathParam.label}" должно быть UUID.`);
      onRun(`${endpoint.title} failed: invalid UUID in ${invalidPathParam.key}`);
      setIsRunning(false);
      return;
    }

    try {
      if (endpoint.kind === 'ws') {
        const wsQuery: QueryParams = { ...queryParams };
        if (accessToken) wsQuery.access_token = accessToken;

        const wsPayload = await readSingleWebSocketEvent({
          path: resolvedPath,
          query: wsQuery
        });
        setLastSummary(summarizeImportantResponse(endpoint.id, wsPayload));
        setLastError(null);
        onRun(`${endpoint.title} connected via backend WS (${Math.round(performance.now() - started)} ms)`);
      } else {
        const bodyPayload =
          endpoint.method === 'POST' && (bodyKeys.length > 0 || hasAutoBodyFields)
            ? buildRequestBody(bodyFields, resolvedHiddenBodyFields, bodyDefaults.template)
            : undefined;
        const invalidBodyPath = bodyPayload ? findInvalidAgentIdPath(bodyPayload) : null;
        if (invalidBodyPath) {
          setLastSummary(null);
          setLastError(`Поле "${invalidBodyPath}" должно быть UUID.`);
          onRun(`${endpoint.title} failed: invalid UUID in ${invalidBodyPath}`);
          setIsRunning(false);
          return;
        }

        const payload = await requestJson<unknown>({
          path: resolvedPath,
          method: endpoint.method,
          query: queryParams,
          body: bodyPayload,
          accessToken
        });

        if (endpoint.id === 'time-scale-set' || endpoint.id === 'time-scale-get') {
          const maybeTimeScale = pickTimeScale(payload);
          if (maybeTimeScale !== null) {
            onTimeScaleChange(Math.min(10, Math.max(0.1, Number(maybeTimeScale.toFixed(2)))));
          }
        }

        setLastSummary(summarizeImportantResponse(endpoint.id, payload));
        setLastError(null);
        onRun(`${endpoint.title} applied via backend (${Math.round(performance.now() - started)} ms)`);
      }
    } catch (error) {
      if (isBackendUnavailableError(error)) {
        if (endpoint.id === 'time-scale-set') {
          const raw = parseMaybeNumber(bodyFields.time_scale) ?? timeScale;
          onTimeScaleChange(Math.min(10, Math.max(0.1, raw)));
        }
        setLastSummary(null);
        setLastError('Backend unavailable');
        onRun(`${endpoint.title} fallback mock: backend unavailable`);
      } else {
        setLastSummary(null);
        setLastError(formatErrorMessage(error));
        onRun(`${endpoint.title} failed: ${formatErrorMessage(error)}`);
      }
    } finally {
      setIsRunning(false);
    }
  };

  return (
    <Card className="border-white/10 bg-slate-950/55">
      <CardHeader className="space-y-2 p-4">
        <div className="flex items-center justify-between gap-2">
          <CardTitle className="text-sm">{endpoint.title}</CardTitle>
          {endpoint.wsEvents ? <Badge variant="outline">Stream</Badge> : null}
        </div>
        <p className="text-xs text-slate-300/80">{endpoint.summary}</p>
      </CardHeader>
      <CardContent className="space-y-3 p-4 pt-0">
        {visibleParams.length > 0 ? (
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
            {visibleParams.map((param) => (
              <div key={param.key} className="space-y-1">
                <Label>{param.label}</Label>
                {isAgentParam(endpoint, param) ? (
                  <AgentInput
                    value={paramValues[param.key] ?? ''}
                    hint={resolveParamHint(endpoint, param)}
                    onChange={(next) =>
                      setParamValues((prev) => ({
                        ...prev,
                        [param.key]: next
                      }))
                    }
                    className="h-8 text-xs"
                    directory={agentDirectory}
                  />
                ) : (
                  <Input
                    value={paramValues[param.key] ?? ''}
                    title={resolveParamHint(endpoint, param)}
                    aria-label={resolveParamHint(endpoint, param)}
                    onChange={(event) =>
                      setParamValues((prev) => ({
                        ...prev,
                        [param.key]: event.target.value
                      }))
                    }
                    className="h-8 text-xs"
                  />
                )}
              </div>
            ))}
          </div>
        ) : null}

        {endpoint.id === 'time-scale-set' ? (
          <div className="space-y-1">
            <Label>Time Scale</Label>
            <Slider
              min={0.1}
              max={10}
              step={0.1}
              value={[parseMaybeNumber(bodyFields.time_scale) ?? timeScale]}
              onValueChange={(values) => {
                const next = Number((values[0] ?? timeScale).toFixed(2));
                onTimeScaleChange(next);
                setBodyFields((prev) => ({ ...prev, time_scale: String(next) }));
              }}
            />
          </div>
        ) : null}

        {bodyKeys.length > 0 && endpoint.id !== 'time-scale-set' ? (
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
            {bodyKeys.map((key) => (
              <div key={key} className="space-y-1">
                <Label title={resolveBodyFieldHint(key)}>{niceKey(key)}</Label>
                {isAgentBodyKey(key) ? (
                  <AgentInput
                    value={bodyFields[key] ?? ''}
                    hint={resolveBodyFieldHint(key)}
                    onChange={(next) =>
                      setBodyFields((prev) => ({
                        ...prev,
                        [key]: next
                      }))
                    }
                    className="h-8 text-xs"
                    directory={agentDirectory}
                  />
                ) : (
                  <Input
                    value={bodyFields[key] ?? ''}
                    title={resolveBodyFieldHint(key)}
                    aria-label={resolveBodyFieldHint(key)}
                    onChange={(event) =>
                      setBodyFields((prev) => ({
                        ...prev,
                        [key]: event.target.value
                      }))
                    }
                    className={cn('h-8 text-xs', key.toLowerCase().includes('content') ? 'sm:col-span-2' : '')}
                  />
                )}
              </div>
            ))}
          </div>
        ) : null}

        {hasAutoBodyFields ? (
          <p className="text-[11px] text-cyan-300/85">Служебный user/admin id подставляется автоматически из текущей сессии.</p>
        ) : null}

        <div className="flex items-center gap-2">
          <Button size="sm" onClick={runAction} disabled={isRunning}>
            {isRunning ? 'Applying...' : 'Apply'}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              setLastSummary(null);
              setLastError(null);
            }}
          >
            Clear
          </Button>
        </div>

        {lastError ? (
          <div className="rounded-md border border-rose-500/40 bg-rose-950/40 px-3 py-2 text-xs text-rose-200">
            {lastError}
          </div>
        ) : null}

        {lastSummary ? (
          <div className="space-y-1">
            <Label>Result</Label>
            <div className="rounded-md border border-cyan-400/20 bg-slate-900/70 px-4 py-3 text-base font-medium leading-6 text-slate-100 whitespace-pre-line">
              {lastSummary}
            </div>
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
};

export const CategoryOperationsModal = ({
  category,
  categoryLabel,
  endpoints,
  open,
  timeScale,
  onClose,
  accessToken,
  operatorUserId,
  agentDirectory,
  onTimeScaleChange,
  onRun
}: CategoryOperationsModalProps) => {
  const [search, setSearch] = useState('');

  const filtered = useMemo(() => {
    const normalized = search.trim().toLowerCase();
    if (!normalized) return endpoints;
    return endpoints.filter(
      (endpoint) =>
        endpoint.title.toLowerCase().includes(normalized) ||
        endpoint.summary.toLowerCase().includes(normalized)
    );
  }, [endpoints, search]);

  if (!open || !category) return null;

  const resolvedDirectory = agentDirectory?.length ? agentDirectory : AGENT_DIRECTORY;

  return (
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center overflow-y-auto bg-black/70 p-4 backdrop-blur-sm"
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
      role="dialog"
      aria-modal="true"
      aria-label={`${categoryLabel ?? category} operations`}
    >
      <div
        className="panel-sheen my-auto flex max-h-[88vh] w-full max-w-5xl flex-col overflow-hidden rounded-2xl border border-white/15 bg-slate-950/95"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex items-center justify-between gap-4 p-5">
          <div>
            <h3 className="text-lg font-semibold text-white">{categoryLabel ?? category}</h3>
            <p className="text-sm text-slate-300/85">{endpoints.length} operations</p>
          </div>
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
        </div>
        <Separator />
        <div className="p-5 pb-3">
          <Label title="Поиск по названию и описанию операции.">Find operation</Label>
          <Input
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            className="mt-1"
            title="Введите часть названия или описания операции."
          />
        </div>
        <div className="dashboard-scroll min-h-0 flex-1 overflow-y-auto overscroll-contain p-5 pt-2">
          <div className="grid gap-3 md:grid-cols-2">
            {filtered.map((endpoint) => (
              <OperationCard
                key={endpoint.id}
                endpoint={endpoint}
                timeScale={timeScale}
                accessToken={accessToken}
                operatorUserId={operatorUserId}
                agentDirectory={resolvedDirectory}
                onTimeScaleChange={onTimeScaleChange}
                onRun={onRun}
              />
            ))}
          </div>
        </div>
      </div>
    </div>
  );
};
