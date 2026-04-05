import type { ReactNode } from "react";
import { Sidebar } from "./Sidebar";
import type { SSEStatus } from "../../api/sse";

interface AppShellProps {
  currentSession: string;
  onSelectSession: (id: string) => void;
  connectionStatus: SSEStatus;
  children: ReactNode;
}

export function AppShell({
  currentSession,
  onSelectSession,
  connectionStatus,
  children,
}: AppShellProps) {
  return (
    <div className="flex h-screen overflow-hidden">
      <Sidebar
        currentSession={currentSession}
        onSelectSession={onSelectSession}
        connectionStatus={connectionStatus}
      />
      <main className="flex-1 flex flex-col overflow-hidden">
        {children}
      </main>
    </div>
  );
}
