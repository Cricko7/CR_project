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
  agentDirectory?: AgentDirectoryEntry[];
  onTimeScaleChange: (value: number) => void;
  onRun: (message: string) => void;
}

interface AgentInputProps {
  value: string;
  onChange: (next: string) => void;
  placeholder?: string;
  className?: string;
  directory: AgentDirectoryEntry[];
}

interface AgentDirectoryEntry {
  id: string;
  name: string;
}

interface ParsedBodyDefaults {
  values: Record<string, string>;
  template: Record<string, unknown>;
}

const parseDefaultFields = (endpoint: EndpointDefinition): ParsedBodyDefaults => {
  if (!endpoint.defaultBody) return { values: {}, template: {} };
  try {
    const parsed = JSON.parse(endpoint.defaultBody) as Record<string, unknown>;
    const values = Object.entries(parsed).reduce<Record<string, string>>((acc, [key, value]) => {
      if (typeof value === 'string') {
        acc[key] = value;
        return acc;
      }
      if (typeof value === 'number' || typeof value === 'boolean') {
        acc[key] = String(value);
        return acc;
      }
      acc[key] = JSON.stringify(value);
      return acc;
    }, {});

    return { values, template: parsed };
  } catch {
    return { values: {}, template: {} };
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

const AgentInput = ({ value, onChange, placeholder, className, directory }: AgentInputProps) => {
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
        placeholder={placeholder}
        className={className}
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
                <span className="text-[10px] text-slate-400">{agent.id}</span>
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

const buildRequestBody = (values: Record<string, string>, template: Record<string, unknown>) =>
  Object.entries(values).reduce<Record<string, unknown>>((acc, [key, value]) => {
    acc[key] = coerceBodyValue(value, template[key]);
    return acc;
  }, {});

const pickTimeScale = (payload: unknown) => {
  if (!payload || typeof payload !== 'object') return null;
  const record = payload as Record<string, unknown>;
  return parseMaybeNumber(record.time_scale ?? record.timeScale);
};

const formatErrorMessage = (error: unknown) => (error instanceof Error ? error.message : 'Unexpected error');

const OperationCard = ({
  endpoint,
  timeScale,
  onTimeScaleChange,
  accessToken,
  agentDirectory,
  onRun
}: OperationCardProps) => {
  const bodyDefaults = useMemo(() => parseDefaultFields(endpoint), [endpoint]);

  const [paramValues, setParamValues] = useState<Record<string, string>>(() =>
    (endpoint.params ?? []).reduce<Record<string, string>>((acc, param) => {
      acc[param.key] = param.defaultValue;
      return acc;
    }, {})
  );
  const [bodyFields, setBodyFields] = useState<Record<string, string>>(bodyDefaults.values);
  const [isRunning, setIsRunning] = useState(false);

  useEffect(() => {
    setBodyFields(bodyDefaults.values);
  }, [bodyDefaults]);

  const bodyKeys = Object.keys(bodyFields);

  const runAction = async () => {
    setIsRunning(true);
    const started = performance.now();

    const pathParams = (endpoint.params ?? [])
      .filter((param) => param.kind === 'path')
      .reduce<Record<string, string>>((acc, param) => {
        acc[param.key] = paramValues[param.key] ?? '';
        return acc;
      }, {});

    const queryParams = (endpoint.params ?? [])
      .filter((param) => param.kind === 'query')
      .reduce<QueryParams>((acc, param) => {
        const value = paramValues[param.key] ?? '';
        if (!value.trim().length) return acc;
        acc[param.key] = value;
        return acc;
      }, {});

    const resolvedPath = resolvePathTemplate(endpoint.path, pathParams);

    try {
      if (endpoint.kind === 'ws') {
        const wsQuery: QueryParams = { ...queryParams };
        if (accessToken) wsQuery.access_token = accessToken;

        await readSingleWebSocketEvent({
          path: resolvedPath,
          query: wsQuery
        });
        onRun(`${endpoint.title} connected via backend WS (${Math.round(performance.now() - started)} ms)`);
      } else {
        const payload = await requestJson<unknown>({
          path: resolvedPath,
          method: endpoint.method,
          query: queryParams,
          body:
            endpoint.method === 'POST' && bodyKeys.length > 0
              ? buildRequestBody(bodyFields, bodyDefaults.template)
              : undefined,
          accessToken
        });

        if (endpoint.id === 'time-scale-set' || endpoint.id === 'time-scale-get') {
          const maybeTimeScale = pickTimeScale(payload);
          if (maybeTimeScale !== null) {
            onTimeScaleChange(Math.min(10, Math.max(0.1, Number(maybeTimeScale.toFixed(2)))));
          }
        }

        onRun(`${endpoint.title} applied via backend (${Math.round(performance.now() - started)} ms)`);
      }
    } catch (error) {
      if (isBackendUnavailableError(error)) {
        if (endpoint.id === 'time-scale-set') {
          const raw = Number(bodyFields.time_scale ?? timeScale);
          if (!Number.isNaN(raw)) onTimeScaleChange(Math.min(10, Math.max(0.1, raw)));
        }
        onRun(`${endpoint.title} fallback mock: backend unavailable`);
      } else {
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
        {(endpoint.params ?? []).length > 0 ? (
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
            {endpoint.params?.map((param) => (
              <div key={param.key} className="space-y-1">
                <Label>{param.label}</Label>
                {isAgentParam(endpoint, param) ? (
                  <AgentInput
                    value={paramValues[param.key] ?? ''}
                    onChange={(next) =>
                      setParamValues((prev) => ({
                        ...prev,
                        [param.key]: next
                      }))
                    }
                    placeholder={param.defaultValue}
                    className="h-8 text-xs"
                    directory={agentDirectory}
                  />
                ) : (
                  <Input
                    value={paramValues[param.key] ?? ''}
                    onChange={(event) =>
                      setParamValues((prev) => ({
                        ...prev,
                        [param.key]: event.target.value
                      }))
                    }
                    placeholder={param.defaultValue}
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
              value={[Number(bodyFields.time_scale ?? timeScale)]}
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
                <Label>{niceKey(key)}</Label>
                {isAgentBodyKey(key) ? (
                  <AgentInput
                    value={bodyFields[key] ?? ''}
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

        <Button size="sm" onClick={runAction} disabled={isRunning}>
          {isRunning ? 'Applying...' : 'Apply'}
        </Button>
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
      className="fixed inset-0 z-[100] flex items-center justify-center bg-black/70 p-4 backdrop-blur-sm"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
      aria-label={`${categoryLabel ?? category} operations`}
    >
      <div
        className="panel-sheen flex max-h-[88vh] w-full max-w-5xl flex-col overflow-hidden rounded-2xl border border-white/15 bg-slate-950/95"
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
          <Label>Find operation</Label>
          <Input
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="search by title or intent..."
            className="mt-1"
          />
        </div>
        <div className="dashboard-scroll grid gap-3 overflow-auto p-5 pt-2 md:grid-cols-2">
          {filtered.map((endpoint) => (
            <OperationCard
              key={endpoint.id}
              endpoint={endpoint}
              timeScale={timeScale}
              accessToken={accessToken}
              agentDirectory={resolvedDirectory}
              onTimeScaleChange={onTimeScaleChange}
              onRun={onRun}
            />
          ))}
        </div>
      </div>
    </div>
  );
};
