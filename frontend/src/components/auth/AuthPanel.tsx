import { useEffect, useMemo, useState } from 'react';
import { AnimatedBackground, GlassCard } from '../base';
import { Badge, Button, Input, Label } from '../ui';
import { useAuth } from '../../auth/AuthProvider';
import { checkBackendHealth } from '../../lib/backend';

type AuthMode = 'login' | 'register';
type BackendStatus = 'checking' | 'online' | 'offline';

export const AuthPanel = () => {
  const { login, register } = useAuth();
  const [mode, setMode] = useState<AuthMode>('login');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [backendStatus, setBackendStatus] = useState<BackendStatus>('checking');
  const [name, setName] = useState('');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');

  const heading = useMemo(
    () => (mode === 'login' ? 'Sign In to Control Deck' : 'Create Control Deck Account'),
    [mode]
  );

  useEffect(() => {
    let cancelled = false;
    let timer: number | null = null;

    const probe = async () => {
      const online = await checkBackendHealth();
      if (cancelled) return;
      setBackendStatus(online ? 'online' : 'offline');
      timer = window.setTimeout(probe, online ? 7000 : 3000);
    };

    void probe();

    return () => {
      cancelled = true;
      if (timer !== null) window.clearTimeout(timer);
    };
  }, []);

  useEffect(() => {
    if (backendStatus === 'online' && error?.startsWith('Internal error')) {
      setError(null);
    }
  }, [backendStatus, error]);

  const onSubmit = async () => {
    if (backendStatus !== 'online') {
      setError('Internal error. Authentication service is unavailable. Reconnecting...');
      return;
    }

    setError(null);
    setLoading(true);
    try {
      if (mode === 'login') {
        await login({ email, password });
      } else {
        await register({ name, email, password });
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Authentication failed.');
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="relative flex min-h-screen items-center justify-center p-4">
      <AnimatedBackground />
      <GlassCard className="w-full max-w-md p-6 sm:p-7">
        <div className="mb-5">
          <div className="flex items-center gap-2">
            <Badge variant="outline">Auth</Badge>
            <Badge variant={backendStatus === 'online' ? 'secondary' : 'outline'}>
              {backendStatus === 'checking'
                ? 'Checking backend'
                : backendStatus === 'online'
                  ? 'Backend online'
                  : 'Backend reconnecting'}
            </Badge>
          </div>
          <h1 className="mt-2 text-2xl font-black text-white">{heading}</h1>
          <p className="mt-1 text-sm text-slate-300/85">
            Access and refresh tokens are handled automatically in this UI session.
          </p>
          {backendStatus === 'offline' ? (
            <p className="mt-2 text-sm text-amber-200">
              Internal error: backend is unavailable. Reconnecting in background...
            </p>
          ) : null}
        </div>

        <div className="mb-4 grid grid-cols-2 gap-2 rounded-lg border border-white/10 bg-slate-900/60 p-1">
          <Button variant={mode === 'login' ? 'secondary' : 'ghost'} size="sm" onClick={() => setMode('login')}>
            Login
          </Button>
          <Button
            variant={mode === 'register' ? 'secondary' : 'ghost'}
            size="sm"
            onClick={() => setMode('register')}
          >
            Register
          </Button>
        </div>

        <div className="space-y-3">
          {mode === 'register' ? (
            <div className="space-y-1">
              <Label>Name</Label>
              <Input value={name} onChange={(event) => setName(event.target.value)} placeholder="Alex Mercer" />
            </div>
          ) : null}
          <div className="space-y-1">
            <Label>Email</Label>
            <Input value={email} onChange={(event) => setEmail(event.target.value)} placeholder="agent@cyber.life" />
          </div>
          <div className="space-y-1">
            <Label>Password</Label>
            <Input
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder="••••••••"
            />
          </div>
        </div>

        {error ? <p className="mt-3 text-sm text-rose-300">{error}</p> : null}

        <Button
          className="mt-5 w-full"
          onClick={onSubmit}
          disabled={
            loading ||
            backendStatus !== 'online' ||
            !email ||
            !password ||
            (mode === 'register' && !name)
          }
        >
          {loading
            ? 'Processing...'
            : backendStatus !== 'online'
              ? 'Waiting for backend...'
              : mode === 'login'
                ? 'Enter Dashboard'
                : 'Create & Enter'}
        </Button>
      </GlassCard>
    </div>
  );
};
