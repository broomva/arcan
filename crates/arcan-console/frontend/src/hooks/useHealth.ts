import { useState, useEffect, useCallback } from "react";
import { arcanClient } from "../api/client";

export interface HealthState {
  status: "healthy" | "unhealthy" | "unknown";
  lastCheck: Date | null;
}

/**
 * Polls the daemon /health endpoint at a regular interval.
 */
export function useHealth(intervalMs = 10_000): HealthState {
  const [health, setHealth] = useState<HealthState>({
    status: "unknown",
    lastCheck: null,
  });

  const check = useCallback(async () => {
    try {
      const resp = await arcanClient.health();
      setHealth({
        status: resp.status === "ok" ? "healthy" : "unhealthy",
        lastCheck: new Date(),
      });
    } catch {
      setHealth({
        status: "unhealthy",
        lastCheck: new Date(),
      });
    }
  }, []);

  useEffect(() => {
    check();
    const timer = setInterval(check, intervalMs);
    return () => clearInterval(timer);
  }, [check, intervalMs]);

  return health;
}
