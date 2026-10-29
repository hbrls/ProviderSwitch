import { describe, expect, it } from "vitest";

import {
  kiloProviderPresets,
  type KiloProviderPreset,
} from "@/config/kiloProviderPresets";

describe("kiloProviderPresets", () => {
  it("contains the Volcano, Bailian, Kimi and DeepSeek presets", () => {
    expect(kiloProviderPresets).toHaveLength(4);
    expect(kiloProviderPresets.map((item) => item.name)).toEqual([
      "火山 Coding Plan",
      "百炼 Token Plan",
      "Kimi",
      "DeepSeek",
    ]);
  });

  const preset = kiloProviderPresets[0] as KiloProviderPreset;

  it("settingsConfig contains all required Kilo sections", () => {
    const config = preset.settingsConfig;
    expect(Object.keys(config)).toEqual(
      expect.arrayContaining([
        "models",
        "thinking",
        "reasoning_effort",
        "options",
      ]),
    );
    expect(Object.keys(config)).not.toContain("name");
    expect(Object.keys(config)).not.toContain("npm");
  });

  it("declares exactly one model", () => {
    const models = preset.settingsConfig.models;
    expect(Object.keys(models)).toHaveLength(1);
    expect(models["glm-5.3"]).toEqual({ name: "GLM-5.3" });
  });

  it("uses the Volcano Coding Plan Kilo base URL", () => {
    expect(preset.settingsConfig.options.baseURL).toBe(
      "https://ark.cn-beijing.volces.com/api/coding/v3",
    );
  });

  it("leaves the API key empty for the user to fill in", () => {
    expect(preset.settingsConfig.options.apiKey).toBe("");
  });

  it("uses the current form defaults for thinking and reasoning effort", () => {
    expect(preset.settingsConfig.thinking.type).toBe("enabled");
    expect(preset.settingsConfig.reasoning_effort).toBe("high");
  });
});
