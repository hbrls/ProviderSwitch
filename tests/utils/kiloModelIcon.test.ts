import { describe, expect, it } from "vitest";

import { resolveKiloModelIcon } from "@/utils/kiloModelIcon";

describe("resolveKiloModelIcon", () => {
  it("maps glm-* model ids to the zai icon", () => {
    expect(resolveKiloModelIcon("glm-5.3")).toBe("zai");
    expect(resolveKiloModelIcon("GLM-5.3")).toBe("zai");
    expect(resolveKiloModelIcon("glm_5.3")).toBe("zai");
  });

  it("maps exactly k3 to the kimi icon", () => {
    expect(resolveKiloModelIcon("k3")).toBe("kimi");
    expect(resolveKiloModelIcon("K3")).toBe("kimi");
    expect(resolveKiloModelIcon("k3-mini")).toBeUndefined();
    expect(resolveKiloModelIcon("kimi-k3")).toBeUndefined();
  });

  it("maps deepseek-* model ids to the deepseek icon", () => {
    expect(resolveKiloModelIcon("deepseek-v4-pro")).toBe("deepseek");
    expect(resolveKiloModelIcon("DeepSeek-Chat")).toBe("deepseek");
    expect(resolveKiloModelIcon("deepseek")).toBeUndefined();
  });

  it("returns undefined for unknown models and ignores path prefixes", () => {
    expect(resolveKiloModelIcon("unknown-model")).toBeUndefined();
    expect(resolveKiloModelIcon("vendor/k3")).toBe("kimi");
    expect(resolveKiloModelIcon("")).toBeUndefined();
  });
});
