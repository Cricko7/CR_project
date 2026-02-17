import { AuthProvider, useAuth } from './auth/AuthProvider';
import { AuthPanel } from './components/auth';
import { Dashboard } from './components/Dashboard';

const AppContent = () => {
  const { loading, session } = useAuth();

  if (loading) {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <div className="h-10 w-10 animate-spin rounded-full border-2 border-cyan-300 border-t-transparent" />
      </div>
    );
  }

  if (!session) return <AuthPanel />;
  return <Dashboard />;
};

function App() {
  return (
    <AuthProvider>
      <AppContent />
    </AuthProvider>
  );
}

export default App;
