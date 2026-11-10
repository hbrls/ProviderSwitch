import { useState, useCallback } from "react";
import {
  streamCheckProvider,
  type StreamCheckResult,
} from "@/lib/api/connectivity-check";
import type { AppId } from "@/lib/api";

/**
 * 供应商连通性检查。
 *
 * 只探测 base_url 是否可达（任何 HTTP 响应都算可达），不发真实大模型请求。
 * 刻意 **不** 重置故障转移熔断器——可达 ≠ 配置正确，一个端口通但鉴权废的供应商
 * 不应被误判为"健康"而切回线上。熔断器只由真实转发流量驱动（见 proxy/forwarder.rs）。
 */
export function useStreamCheck(appId: AppId) {
  const [checkingIds, setCheckingIds] = useState<Set<string>>(new Set());

  const checkProvider = useCallback(
    async (providerId: string): Promise<StreamCheckResult | null> => {
      setCheckingIds((prev) => new Set(prev).add(providerId));

      try {
        return await streamCheckProvider(appId, providerId);
      } catch {
        return null;
      } finally {
        setCheckingIds((prev) => {
          const next = new Set(prev);
          next.delete(providerId);
          return next;
        });
      }
    },
    [appId],
  );

  const isChecking = useCallback(
    (providerId: string) => checkingIds.has(providerId),
    [checkingIds],
  );

  return { checkProvider, isChecking };
}
