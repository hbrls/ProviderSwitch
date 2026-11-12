import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

beforeEach(() => {
  vi.resetModules();
  localStorage.clear();
});

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
});

describe("system language subtag boundaries", () => {
  it.each([
    ["zh", "zh"],
    ["zh-CN", "zh"],
    ["zh-TW", "zh"],
    ["zh-TW-u-nu-hanidec", "zh"],
    ["zh-HK", "zh"],
    ["zh-MO", "zh"],
    ["zh-Hant-TW", "zh"],
    ["zh-Hans-CN", "zh"],
    ["ja", "en"],
    ["ja-JP", "en"],
    ["tr", "en"],
    ["tr-TR", "en"],
    ["ru", "en"],
    ["ru-RU", "en"],
    ["rue", "en"],
    ["rup-RO", "en"],
    ["en", "en"],
    ["en-US", "en"],
    ["jam", "en"],
    ["trv-TW", "en"],
    ["zh-hantx", "zh"],
    ["zh-hkx", "zh"],
  ])("maps %s to %s on first launch", async (language, expected) => {
    vi.stubGlobal("navigator", { language });
    const { default: i18n } = await import("@/i18n");
    expect(i18n.language).toBe(expected);
  });
});
