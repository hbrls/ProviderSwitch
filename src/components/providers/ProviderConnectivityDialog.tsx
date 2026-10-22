import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, X } from "lucide-react";
import type { AppId } from "@/lib/api";
import type { StreamCheckResult } from "@/lib/api/connectivity-check";
import { testProviderChat } from "@/lib/api/connectivity-check";
import { Button } from "@/components/ui/button";
import type { Provider } from "@/types";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

interface ProviderConnectivityDialogProps {
  appId: AppId;
  provider: Provider | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  checkProvider: (providerId: string) => Promise<StreamCheckResult | null>;
}

export function ProviderConnectivityDialog({
  appId,
  provider,
  open,
  onOpenChange,
  checkProvider,
}: ProviderConnectivityDialogProps) {
  const { t } = useTranslation();
  const [chatResult, setChatResult] = useState<string[]>([]);
  const [svgPreviewUrl, setSvgPreviewUrl] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState("logs");
  const [isChatTesting, setIsChatTesting] = useState(false);
  const [connectivityResult, setConnectivityResult] = useState<string | null>(
    null,
  );
  const [isConnectivityTesting, setIsConnectivityTesting] = useState(false);
  const chatRequestRef = useRef(0);
  const resultRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const element = resultRef.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [chatResult]);

  useEffect(() => {
    chatRequestRef.current += 1;
    setChatResult([]);
    setSvgPreviewUrl(null);
    setActiveTab("logs");
    setIsChatTesting(false);
    setConnectivityResult(null);
    setIsConnectivityTesting(false);
    return () => {
      chatRequestRef.current += 1;
    };
  }, [open, appId, provider?.id]);

  useEffect(() => {
    return () => {
      if (svgPreviewUrl) URL.revokeObjectURL(svgPreviewUrl);
    };
  }, [svgPreviewUrl]);

  const testChat = async () => {
    if (!provider || isChatTesting) return;
    const requestId = ++chatRequestRef.current;
    setIsChatTesting(true);
    setChatResult([]);
    setSvgPreviewUrl(null);
    let fullContent = "";
    let pendingContent = "";
    const flushContent = () => {
      if (!pendingContent) return;
      const remaining = pendingContent;
      pendingContent = "";
      setChatResult((previous) => [...previous, remaining]);
    };
    try {
      const result = await testProviderChat(appId, provider.id, (event) => {
        if (requestId !== chatRequestRef.current) return;
        const data = event
          .split(/\r?\n/)
          .filter((line) => line.startsWith("data:"))
          .map((line) => line.slice(5).trimStart())
          .join("\n");
        if (data.trim() === "[DONE]") {
          flushContent();
          return;
        }
        if (!data) {
          setChatResult((previous) => [...previous, event.trimEnd()]);
          return;
        }
        let chunk;
        try {
          chunk = JSON.parse(data);
        } catch {
          setChatResult((previous) => [...previous, data]);
          return;
        }
        if (chunk.error) {
          flushContent();
          setChatResult((previous) => [
            ...previous,
            JSON.stringify(chunk.error),
          ]);
          return;
        }
        const content = chunk.choices?.[0]?.delta?.content;
        if (typeof content !== "string") return;
        fullContent += content;
        pendingContent += content;
        const lastNewline = pendingContent.lastIndexOf("\n");
        if (lastNewline < 0) return;
        const completeLines = pendingContent.slice(0, lastNewline + 1);
        pendingContent = pendingContent.slice(lastNewline + 1);
        setChatResult((previous) => [...previous, completeLines]);
      });
      if (requestId !== chatRequestRef.current) return;
      flushContent();
      const svgText = fullContent.match(/<svg\b[\s\S]*?<\/svg\s*>/i)?.[0];
      if (svgText) {
        setSvgPreviewUrl(
          URL.createObjectURL(new Blob([svgText], { type: "image/svg+xml" })),
        );
      }
      setChatResult((previous) => [
        ...previous,
        `完成 · ${result.responseTimeMs}ms`,
      ]);
    } catch (error) {
      if (requestId === chatRequestRef.current) {
        flushContent();
        setChatResult((previous) => [...previous, `错误：${String(error)}`]);
      }
    } finally {
      if (requestId === chatRequestRef.current) setIsChatTesting(false);
    }
  };

  const requestRef = useRef<{
    key: string;
    promise: Promise<StreamCheckResult | null>;
  } | null>(null);

  useEffect(() => {
    if (!open || !provider) return;

    const key = `${appId}:${provider.id}`;
    if (requestRef.current?.key !== key) {
      setIsConnectivityTesting(true);
      const promise = checkProvider(provider.id);
      requestRef.current = {
        key,
        promise,
      };
      void promise
        .then((result) => {
          if (requestRef.current?.key !== key) return;
          setConnectivityResult(
            result
              ? result.status === "operational"
                ? `连通正常 (${result.responseTimeMs ?? "-"}ms)`
                : result.status === "degraded"
                  ? `连通但较慢 (${result.responseTimeMs ?? "-"}ms)`
                  : `无法连通: ${result.message}`
              : "检查出错",
          );
        })
        .finally(() => {
          if (requestRef.current?.key === key) setIsConnectivityTesting(false);
        });
    }
  }, [appId, checkProvider, open, provider, t]);

  useEffect(() => {
    if (!open) requestRef.current = null;
  }, [open]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-xl">
        <DialogClose
          className="absolute right-4 top-4 rounded-control p-1 text-fg-3 transition-colors hover:bg-subtle hover:text-fg-1 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
          aria-label={t("common.close", { defaultValue: "关闭" })}
        >
          <X className="h-4 w-4" />
        </DialogClose>
        <DialogHeader>
          <DialogTitle className="text-title font-semibold leading-6 text-success-text">
            {provider?.name || t("provider.connectivityCheck")} -{" "}
            {isConnectivityTesting ? "检查中..." : connectivityResult}
          </DialogTitle>
        </DialogHeader>
        <div className="px-6 pb-6 text-body text-fg-1">
          <Button onClick={testChat} disabled={!provider || isChatTesting}>
            {isChatTesting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            鹈鹕测试
          </Button>
          <Tabs value={activeTab} onValueChange={setActiveTab} className="mt-4">
            <TabsList>
              <TabsTrigger value="logs">输出日志</TabsTrigger>
              <TabsTrigger value="preview">SVG 预览</TabsTrigger>
            </TabsList>
            <TabsContent value="logs" className="mt-3">
              <div
                ref={resultRef}
                aria-live="polite"
                className="h-96 overflow-auto rounded border border-border p-2 text-xs"
              >
                {chatResult.map((event, index) => (
                  <pre
                    key={index}
                    className="mb-4 whitespace-pre-wrap break-words last:mb-0"
                  >
                    {event}
                  </pre>
                ))}
              </div>
            </TabsContent>
            <TabsContent value="preview" className="mt-3">
              <div className="flex h-96 items-center justify-center overflow-auto rounded border border-border bg-white p-3">
                {svgPreviewUrl ? (
                  <img
                    src={svgPreviewUrl}
                    alt="生成的 SVG 预览"
                    className="max-h-full max-w-full object-contain"
                  />
                ) : (
                  <span className="text-fg-3">
                    {isChatTesting ? "生成完成后显示预览" : "暂无 SVG 预览"}
                  </span>
                )}
              </div>
            </TabsContent>
          </Tabs>
        </div>
      </DialogContent>
    </Dialog>
  );
}
