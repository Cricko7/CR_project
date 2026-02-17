import { useMemo, useState } from 'react';
import { Badge, Button, Card, CardContent, CardDescription, CardHeader, CardTitle, Input, Label, Textarea } from '../ui';
import { cn } from '../../lib/cn';
import type { EndpointDefinition } from './types';

type ExecutionStatus = 'idle' | 'running' | 'ready';

export interface EndpointConsoleCardProps {
  endpoint: EndpointDefinition;
  timeScale: number;
  onTimeScaleChange: (next: number) => void;
}

const methodVariant = (method: 'GET' | 'POST') => (method === 'GET' ? 'secondary' : 'default');

const parseBodyNumber = (body: string, key: string): number | null => {
  try {
    const parsed = JSON.parse(body) as Record<string, unknown>;
    const value = parsed[key];
    if (typeof value === 'number') return value;
    return null;
  } catch {
    return null;
  }
};

const buildMockResponse = (endpoint: EndpointDefinition, body: string, timeScale: number) => {
  if (endpoint.id === 'time-scale-get') {
    return JSON.stringify(
      { time_scale: Number(timeScale.toFixed(2)), updated_at: new Date().toISOString() },
      null,
      2
    );
  }
  if (endpoint.id === 'time-scale-set') {
    const parsed = parseBodyNumber(body, 'time_scale');
    return JSON.stringify(
      { time_scale: parsed ?? Number(timeScale.toFixed(2)), updated_at: new Date().toISOString() },
      null,
      2
    );
  }
  return endpoint.sampleResponse;
};

export const EndpointConsoleCard = ({ endpoint, timeScale, onTimeScaleChange }: EndpointConsoleCardProps) => {
  const [status, setStatus] = useState<ExecutionStatus>('idle');
  const [body, setBody] = useState(endpoint.defaultBody ?? '');
  const [response, setResponse] = useState(endpoint.sampleResponse);
  const [latencyMs, setLatencyMs] = useState<number | null>(null);
  const [activeWsEvent, setActiveWsEvent] = useState(endpoint.wsEvents?.[0] ?? '');

  const [paramValues, setParamValues] = useState<Record<string, string>>(() =>
    (endpoint.params ?? []).reduce<Record<string, string>>((acc, param) => {
      acc[param.key] = param.defaultValue;
      return acc;
    }, {})
  );

  const resolvedPath = useMemo(() => {
    const pathParams = (endpoint.params ?? []).filter((param) => param.kind === 'path');
    return pathParams.reduce((path, param) => {
      const value = paramValues[param.key] ?? param.defaultValue;
      return path.replace(`{${param.key}}`, value || `{${param.key}}`);
    }, endpoint.path);
  }, [endpoint.params, endpoint.path, paramValues]);

  const queryString = useMemo(() => {
    const pairs = (endpoint.params ?? [])
      .filter((param) => param.kind === 'query')
      .map((param) => [param.key, paramValues[param.key] ?? ''] as const)
      .filter(([, value]) => value.trim().length > 0)
      .map(([key, value]) => `${encodeURIComponent(key)}=${encodeURIComponent(value)}`);
    return pairs.length > 0 ? `?${pairs.join('&')}` : '';
  }, [endpoint.params, paramValues]);

  const requestPreview = `${endpoint.method} ${resolvedPath}${queryString}`;

  const runMock = () => {
    const started = performance.now();
    setStatus('running');
    const timeout = 120 + Math.round(Math.random() * 260);

    window.setTimeout(() => {
      const maybeScale = endpoint.id === 'time-scale-set' ? parseBodyNumber(body, 'time_scale') : null;
      if (typeof maybeScale === 'number') {
        onTimeScaleChange(Math.min(10, Math.max(0.1, maybeScale)));
      }

      const baseResponse = buildMockResponse(endpoint, body, timeScale);
      if (endpoint.kind === 'ws' && activeWsEvent) {
        setResponse(
          JSON.stringify(
            {
              type: activeWsEvent,
              stream: endpoint.path,
              connected: true,
              timestamp: new Date().toISOString()
            },
            null,
            2
          )
        );
      } else {
        setResponse(baseResponse);
      }

      setLatencyMs(Math.round(performance.now() - started));
      setStatus('ready');
    }, timeout);
  };

  return (
    <Card className="h-full">
      <CardHeader className="space-y-3">
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant={methodVariant(endpoint.method)}>{endpoint.method}</Badge>
          <Badge variant="outline">{endpoint.kind}</Badge>
          <Badge variant="outline">{endpoint.category}</Badge>
          {status === 'running' ? <Badge variant="secondary">Running...</Badge> : null}
          {latencyMs !== null && status === 'ready' ? <Badge variant="secondary">{latencyMs} ms</Badge> : null}
        </div>
        <CardTitle>{endpoint.title}</CardTitle>
        <CardDescription>{endpoint.summary}</CardDescription>
      </CardHeader>

      <CardContent className="space-y-4">
        <div className="space-y-1">
          <Label>Endpoint</Label>
          <div className="rounded-md border border-white/10 bg-slate-900/70 px-3 py-2 text-xs text-cyan-200">
            {requestPreview}
          </div>
        </div>

        {(endpoint.params ?? []).length > 0 ? (
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            {endpoint.params?.map((param) => (
              <div key={param.key} className="space-y-1">
                <Label>{param.label}</Label>
                <Input
                  value={paramValues[param.key] ?? ''}
                  onChange={(event) =>
                    setParamValues((prev) => ({
                      ...prev,
                      [param.key]: event.target.value
                    }))
                  }
                  placeholder={param.defaultValue}
                />
              </div>
            ))}
          </div>
        ) : null}

        {endpoint.defaultBody ? (
          <div className="space-y-1">
            <Label>Request Body</Label>
            <Textarea value={body} onChange={(event) => setBody(event.target.value)} className="font-mono text-xs" />
          </div>
        ) : null}

        {endpoint.kind === 'ws' && endpoint.wsEvents ? (
          <div className="space-y-1">
            <Label>WS Event Type</Label>
            <select
              className={cn(
                'flex h-10 w-full rounded-md border border-white/15 bg-slate-900/70 px-3 py-2 text-sm text-slate-100',
                'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-300/70'
              )}
              value={activeWsEvent}
              onChange={(event) => setActiveWsEvent(event.target.value)}
            >
              {endpoint.wsEvents.map((eventType) => (
                <option key={eventType} value={eventType} className="bg-slate-950 text-slate-100">
                  {eventType}
                </option>
              ))}
            </select>
          </div>
        ) : null}

        <div className="flex items-center gap-2">
          <Button onClick={runMock} disabled={status === 'running'}>
            Mock Execute
          </Button>
          <Button
            variant="ghost"
            onClick={() => {
              setResponse(endpoint.sampleResponse);
              setStatus('idle');
              setLatencyMs(null);
            }}
          >
            Reset
          </Button>
        </div>

        <div className="space-y-1">
          <Label>Mock Response</Label>
          <pre className="max-h-60 overflow-auto rounded-md border border-white/10 bg-slate-900/80 p-3 text-xs text-slate-200">
            {response}
          </pre>
        </div>
      </CardContent>
    </Card>
  );
};
