const KILO_MODEL_ICON_RULES = [
  { pattern: /^glm(?:[-_.]|$)/i, icon: "zai" },
] as const;

export function resolveKiloModelIcon(modelId: string): string | undefined {
  const normalizedModelId = modelId.trim().split("/").pop() ?? "";
  return KILO_MODEL_ICON_RULES.find(({ pattern }) =>
    pattern.test(normalizedModelId),
  )?.icon;
}
