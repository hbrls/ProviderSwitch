const KILO_MODEL_ICON_RULES = [
  { pattern: /^glm(?:[-_.]|$)/i, icon: "zai" },
  { pattern: /^k3$/i, icon: "kimi" },
  { pattern: /^deepseek[-_.]/i, icon: "deepseek" },
] as const;

export function resolveKiloModelIcon(modelId: string): string | undefined {
  const normalizedModelId = modelId.trim().split("/").pop() ?? "";
  return KILO_MODEL_ICON_RULES.find(({ pattern }) =>
    pattern.test(normalizedModelId),
  )?.icon;
}
