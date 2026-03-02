import { useState, useCallback, useEffect } from "react";
import { AppShell } from "./components/layout/AppShell";
import { ChatView } from "./components/chat/ChatView";
import { useArcanSession } from "./hooks/useArcanSession";
import { arcanClient } from "./api/client";

export function App() {
  const [sessionId, setSessionId] = useState("");

  // On mount, try to load the most recent session or create one
  useEffect(() => {
    async function init() {
      try {
        const sessions = await arcanClient.listSessions();
        if (sessions.length > 0 && sessions[0]) {
          setSessionId(sessions[0].session_id);
        }
      } catch {
        // Daemon might not be running — that's fine, sidebar will show status
      }
    }
    init();
  }, []);

  const session = useArcanSession(sessionId);

  const handleSelectSession = useCallback((id: string) => {
    setSessionId(id);
  }, []);

  const handleSendMessage = useCallback(
    (content: string) => {
      session.sendMessage(content);
    },
    [session],
  );

  const handleResolveApproval = useCallback(
    (approvalId: string, approved: boolean) => {
      session.resolveApproval(approvalId, approved);
    },
    [session],
  );

  return (
    <AppShell
      currentSession={sessionId}
      onSelectSession={handleSelectSession}
      connectionStatus={session.connectionStatus}
    >
      <ChatView
        session={session}
        onSendMessage={handleSendMessage}
        onResolveApproval={handleResolveApproval}
      />
    </AppShell>
  );
}
