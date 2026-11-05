import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type FormEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { ImeSafeInput } from "@/components/ui/ime-safe-input";
import { RequestHeadersEditor } from "./RequestHeadersEditor";
import type { ProviderFormProps, ProviderFormValues } from "./ProviderForm";
import { resolveKiloModelIcon } from "@/utils/kiloModelIcon";
import { REQUEST_HEADER_DRAFT_PREFIX } from "./helpers/requestHeaders";

type KiloProviderFormProps = Omit<ProviderFormProps, "appId">;

const DEFAULT_THINKING_TYPE = "enabled";
const DEFAULT_REASONING_EFFORT = "high";
const RESERVED_OPTION_KEYS = new Set(["baseURL", "apiKey", "headers"]);

type KiloFormState = {
  providerName: string;
  baseUrl: string;
  apiKey: string;
  headers: Record<string, string>;
  extraOptions: Record<string, string>;
  modelId: string;
  modelName: string;
  thinkingType: string;
  reasoningEffort: string;
};

type KiloSettingsInput = Omit<KiloFormState, "providerName">;

const EMPTY_KILO_FORM: KiloFormState = {
  providerName: "",
  baseUrl: "",
  apiKey: "",
  headers: {},
  extraOptions: {},
  modelId: "",
  modelName: "",
  thinkingType: DEFAULT_THINKING_TYPE,
  reasoningEffort: DEFAULT_REASONING_EFFORT,
};

function stringRecord(value: unknown): Record<string, string> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};

  return Object.fromEntries(
    Object.entries(value).filter(([, item]) => typeof item === "string"),
  ) as Record<string, string>;
}

function readKiloFormState(
  initialData?: KiloProviderFormProps["initialData"],
): KiloFormState {
  const settings = initialData?.settingsConfig;
  if (!settings) {
    return {
      ...EMPTY_KILO_FORM,
      providerName: initialData?.name ?? "",
    };
  }

  const options =
    settings.options &&
    typeof settings.options === "object" &&
    !Array.isArray(settings.options)
      ? (settings.options as Record<string, unknown>)
      : {};
  const modelEntries =
    settings.models &&
    typeof settings.models === "object" &&
    !Array.isArray(settings.models)
      ? Object.entries(settings.models as Record<string, { name?: unknown }>)
      : [];
  const [modelEntry] = modelEntries;
  const [modelId, model] = modelEntry ?? ["", undefined];
  const thinking =
    settings.thinking &&
    typeof settings.thinking === "object" &&
    !Array.isArray(settings.thinking)
      ? (settings.thinking as Record<string, unknown>)
      : {};

  return {
    providerName: initialData?.name ?? "",
    baseUrl: typeof options.baseURL === "string" ? options.baseURL : "",
    apiKey: typeof options.apiKey === "string" ? options.apiKey : "",
    headers: stringRecord(options.headers),
    extraOptions: Object.fromEntries(
      Object.entries(options).filter(
        ([key, value]) =>
          !["baseURL", "apiKey", "headers"].includes(key) &&
          typeof value === "string",
      ),
    ) as Record<string, string>,
    modelId,
    modelName: typeof model?.name === "string" ? model.name : modelId,
    thinkingType:
      typeof thinking.type === "string" && thinking.type.trim()
        ? thinking.type
        : DEFAULT_THINKING_TYPE,
    reasoningEffort:
      typeof settings.reasoning_effort === "string" &&
      settings.reasoning_effort.trim()
        ? settings.reasoning_effort
        : DEFAULT_REASONING_EFFORT,
  };
}

/** Build the exact object persisted as the Kilo provider settingsConfig. */
export function buildKiloSettingsConfig(
  params: KiloSettingsInput,
): Record<string, unknown> {
  const options: Record<string, unknown> = {
    baseURL: params.baseUrl.trim(),
    apiKey: params.apiKey.trim(),
  };

  const headers = Object.fromEntries(
    Object.entries(params.headers).filter(
      ([key, value]) =>
        key.trim() &&
        !key.startsWith(REQUEST_HEADER_DRAFT_PREFIX) &&
        value.trim(),
    ),
  );
  if (Object.keys(headers).length > 0) options.headers = headers;

  for (const [key, value] of Object.entries(params.extraOptions)) {
    const trimmedKey = key.trim();
    if (trimmedKey && !RESERVED_OPTION_KEYS.has(trimmedKey) && value.trim()) {
      options[trimmedKey] = value;
    }
  }

  const modelId = params.modelId.trim();
  const models = modelId
    ? {
        [modelId]: {
          name: params.modelName.trim() || modelId,
        },
      }
    : {};

  return {
    models,
    thinking: {
      type: params.thinkingType.trim(),
    },
    reasoning_effort: params.reasoningEffort.trim(),
    options,
  };
}

export function KiloProviderForm({
  submitLabel,
  onSubmit,
  onCancel,
  onSubmittingChange,
  onSubmitReadyChange,
  initialData,
  showButtons = true,
}: KiloProviderFormProps) {
  const { t } = useTranslation();
  const [formState, setFormState] = useState<KiloFormState>(() =>
    readKiloFormState(initialData),
  );
  const [isSubmitting, setIsSubmitting] = useState(false);

  useEffect(() => {
    setFormState(readKiloFormState(initialData));
  }, [initialData]);

  useEffect(() => {
    onSubmitReadyChange?.(true);
  }, [onSubmitReadyChange]);

  useEffect(() => {
    onSubmittingChange?.(isSubmitting);
  }, [isSubmitting, onSubmittingChange]);

  const updateField = useCallback(
    <K extends keyof KiloFormState>(field: K, value: KiloFormState[K]) => {
      setFormState((current) => ({ ...current, [field]: value }));
    },
    [],
  );

  const kiloConfig = useMemo(
    () => buildKiloSettingsConfig(formState),
    [formState],
  );
  const kiloConfigPreview = useMemo(
    () => JSON.stringify(kiloConfig, null, 2),
    [kiloConfig],
  );

  const extraOptionEntries = Object.entries(formState.extraOptions);

  const handleAddExtraOption = () => {
    updateField("extraOptions", {
      ...formState.extraOptions,
      [`option-${Date.now()}`]: "",
    });
  };

  const handleRemoveExtraOption = (key: string) => {
    const next = { ...formState.extraOptions };
    delete next[key];
    updateField("extraOptions", next);
  };

  const handleExtraOptionKeyChange = (oldKey: string, newKey: string) => {
    const trimmed = newKey.trim();
    if (!trimmed || trimmed === oldKey) return;
    const next: Record<string, string> = {};
    for (const [key, value] of Object.entries(formState.extraOptions)) {
      next[key === oldKey ? trimmed : key] = value;
    }
    updateField("extraOptions", next);
  };

  const handleExtraOptionValueChange = (key: string, value: string) => {
    updateField("extraOptions", { ...formState.extraOptions, [key]: value });
  };

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (
      !formState.providerName.trim() ||
      !formState.modelId.trim() ||
      !formState.thinkingType.trim() ||
      !formState.reasoningEffort.trim()
    ) {
      return;
    }

    const existingIcon = initialData?.icon?.trim() || "";
    const modelIcon = resolveKiloModelIcon(formState.modelId);
    const preservedIcon = existingIcon === "glm" ? "" : existingIcon;
    const payload: ProviderFormValues = {
      name: formState.providerName.trim(),
      websiteUrl: "",
      notes: "",
      settingsConfig: JSON.stringify(kiloConfig),
      icon: modelIcon || preservedIcon,
      iconColor:
        modelIcon || !preservedIcon ? "" : initialData?.iconColor?.trim() || "",
    };

    setIsSubmitting(true);
    try {
      await onSubmit(payload);
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <form
      id="provider-form"
      onSubmit={handleSubmit}
      className="glass rounded-xl border border-white/10 p-6"
    >
      <div className="grid min-w-0 grid-cols-1 gap-6 lg:grid-cols-[2fr_3fr]">
        <section className="order-2 min-w-0 self-start lg:sticky lg:top-4 lg:order-1">
          <Label htmlFor="kilo-config-preview">
            {t("kilo.form.configPreview", {
              defaultValue: "Kilo Configuration JSON",
            })}
          </Label>
          <Textarea
            id="kilo-config-preview"
            value={kiloConfigPreview}
            readOnly
            aria-label={t("kilo.form.configPreview", {
              defaultValue: "Kilo Configuration JSON",
            })}
            className="mt-2 min-h-[420px] resize-none overflow-auto bg-background/60 font-mono text-xs leading-5"
          />
        </section>

        <div className="order-1 min-w-0 space-y-6 border-border-default lg:order-2 lg:border-l lg:pl-6">
          <div className="space-y-2">
            <Label htmlFor="kilo-provider-name">
              {t("kilo.form.providerName", { defaultValue: "Provider Name" })}
            </Label>
            <ImeSafeInput
              id="kilo-provider-name"
              value={formState.providerName}
              onValueChange={(value) => updateField("providerName", value)}
              placeholder="Volc"
            />
          </div>

          <div className="space-y-2">
            <Label htmlFor="kilo-base-url">
              {t("kilo.form.baseUrl", { defaultValue: "Base URL" })}
            </Label>
            <ImeSafeInput
              id="kilo-base-url"
              value={formState.baseUrl}
              onValueChange={(value) => updateField("baseUrl", value)}
              placeholder="https://ark.cn-beijing.volces.com/api/coding/v3"
              required
            />
          </div>

          <div className="space-y-2">
            <Label htmlFor="kilo-api-key">
              {t("kilo.form.apiKey", { defaultValue: "API Key" })}
            </Label>
            <ImeSafeInput
              id="kilo-api-key"
              type="text"
              value={formState.apiKey}
              onValueChange={(value) => updateField("apiKey", value)}
              autoComplete="off"
              required
            />
          </div>

          <RequestHeadersEditor
            headers={formState.headers}
            onHeadersChange={(headers) => updateField("headers", headers)}
          />

          <div className="space-y-3 border-l border-border-default pl-3">
            <Label>{t("kilo.form.models", { defaultValue: "Model" })}</Label>
            <div className="flex items-center gap-2 px-1 text-xs text-muted-foreground">
              <span className="flex-1">
                {t("kilo.form.modelId", { defaultValue: "Model ID" })}
              </span>
              <span className="flex-1">
                {t("kilo.form.modelName", { defaultValue: "Model Name" })}
              </span>
            </div>
            <div className="flex items-center gap-2">
              <ImeSafeInput
                value={formState.modelId}
                onValueChange={(value) => updateField("modelId", value)}
                placeholder="glm-5.3"
                aria-label={t("kilo.form.modelId", {
                  defaultValue: "Model ID",
                })}
                required
                className="min-w-0 flex-1"
              />
              <ImeSafeInput
                value={formState.modelName}
                onValueChange={(value) => updateField("modelName", value)}
                placeholder="GLM-5.3"
                aria-label={t("kilo.form.modelName", {
                  defaultValue: "Model Name",
                })}
                className="min-w-0 flex-1"
              />
            </div>
          </div>

          <div className="space-y-3 border-l border-border-default pl-3">
            <Label>
              {t("kilo.form.requestBody", { defaultValue: "Request Body" })}
            </Label>
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="kilo-thinking-type">
                  {t("kilo.form.thinkingType", {
                    defaultValue: "Thinking Type",
                  })}
                </Label>
                <ImeSafeInput
                  id="kilo-thinking-type"
                  value={formState.thinkingType}
                  onValueChange={(value) => updateField("thinkingType", value)}
                  placeholder={DEFAULT_THINKING_TYPE}
                  required
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="kilo-reasoning-effort">
                  {t("kilo.form.reasoningEffort", {
                    defaultValue: "Reasoning Effort",
                  })}
                </Label>
                <ImeSafeInput
                  id="kilo-reasoning-effort"
                  value={formState.reasoningEffort}
                  onValueChange={(value) =>
                    updateField("reasoningEffort", value)
                  }
                  placeholder={DEFAULT_REASONING_EFFORT}
                  required
                />
              </div>
            </div>
          </div>

          <div className="space-y-2 border-l border-border-default pl-3">
            <div className="flex items-center justify-between">
              <Label>
                {t("kilo.form.extraOptions", { defaultValue: "Options" })}
              </Label>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={handleAddExtraOption}
                className="h-7 gap-1"
              >
                <Plus className="h-3.5 w-3.5" />
                {t("common.add", { defaultValue: "Add" })}
              </Button>
            </div>
            {extraOptionEntries.length === 0 ? (
              <p className="py-1 text-sm text-muted-foreground">
                {t("kilo.form.noExtraOptions", {
                  defaultValue: "No extra options configured",
                })}
              </p>
            ) : (
              <div className="space-y-2">
                {extraOptionEntries.map(([key, value]) => (
                  <div key={key} className="flex items-center gap-2">
                    <ImeSafeInput
                      value={key.startsWith("option-") ? "" : key}
                      onValueChange={(next) =>
                        handleExtraOptionKeyChange(key, next)
                      }
                      placeholder="timeout"
                      aria-label={t("kilo.form.optionKey", {
                        defaultValue: "Option Key",
                      })}
                      className="min-w-0 flex-1"
                    />
                    <ImeSafeInput
                      value={value}
                      onValueChange={(next) =>
                        handleExtraOptionValueChange(key, next)
                      }
                      placeholder="600000"
                      aria-label={t("kilo.form.optionValue", {
                        defaultValue: "Option Value",
                      })}
                      className="min-w-0 flex-1"
                    />
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      onClick={() => handleRemoveExtraOption(key)}
                      aria-label={t("kilo.form.removeOption", {
                        defaultValue: "Remove option",
                      })}
                      className="h-9 w-9 shrink-0 text-muted-foreground hover:text-destructive"
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </div>
                ))}
              </div>
            )}
          </div>

          {showButtons && (
            <div className="flex justify-end gap-2">
              <Button variant="outline" type="button" onClick={onCancel}>
                {t("common.cancel", { defaultValue: "Cancel" })}
              </Button>
              <Button type="submit" disabled={isSubmitting}>
                {submitLabel}
              </Button>
            </div>
          )}
        </div>
      </div>
    </form>
  );
}
