/**
 * Kilo 预设供应商配置模板
 *
 * 独立于 Claude/Codex/Gemini 的 ProviderPreset 体系：
 * - settingsConfig 严格使用 Kilo 表单当前持久化的 JSON 结构
 * - 只是表单初始化模板，不引入新的 Kilo 配置协议
 */
import type { ProviderCategory } from "../types";
import type { PresetTheme } from "./claudeProviderPresets";
import type { PresetFamilyFields } from "./presetFamilies";

export interface KiloPresetSettingsConfig {
  models: Record<string, { name: string }>;
  thinking: {
    type: string;
  };
  reasoning_effort: string;
  options: {
    baseURL: string;
    apiKey: string;
    proxy?: string;
    headers?: Record<string, string>;
    [key: string]: unknown;
  };
}

export interface KiloProviderPreset extends PresetFamilyFields {
  name: string;
  nameKey?: string; // i18n key for localized display name
  websiteUrl?: string;
  apiKeyUrl?: string;
  settingsConfig: KiloPresetSettingsConfig;
  category?: ProviderCategory;
  isPartner?: boolean; // 标识是否为商业合作伙伴
  primePartner?: boolean; // 置顶合作伙伴（顶级）：徽章显示为心形
  partnerPromotionKey?: string; // 合作伙伴促销信息的 i18n key
  // 视觉主题配置（仅选择器展示用）
  theme?: PresetTheme;
  icon?: string; // 图标名称
  iconColor?: string; // 图标颜色
}

export const kiloProviderPresets: KiloProviderPreset[] = [
  {
    name: "火山 Coding Plan",
    websiteUrl:
      "https://console.volcengine.com/ark/region:cn-beijing/subscription/coding-plan",
    apiKeyUrl:
      "https://console.volcengine.com/ark/region:cn-beijing/subscription/coding-plan",
    settingsConfig: {
      models: {
        "glm-5.3": {
          name: "GLM-5.3",
        },
      },
      thinking: {
        type: "enabled",
      },
      reasoning_effort: "high",
      options: {
        baseURL: "https://ark.cn-beijing.volces.com/api/coding/v3",
        // API Key 必须由用户在表单中填写，不写入源码
        apiKey: "",
      },
    },
    category: "cn_official",
    icon: "huoshan",
    iconColor: "#3370FF",
  },
  {
    name: "百炼 Token Plan",
    websiteUrl:
      "https://bailian.console.aliyun.com/cn-beijing/subscription/token-plan/personal",
    apiKeyUrl:
      "https://bailian.console.aliyun.com/cn-beijing/subscription/token-plan/personal",
    settingsConfig: {
      models: {
        "glm-5.3": {
          name: "GLM-5.3",
        },
      },
      thinking: {
        type: "enabled",
      },
      reasoning_effort: "high",
      options: {
        baseURL:
          "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
        // API Key 必须由用户在表单中填写，不写入源码
        apiKey: "",
      },
    },
    category: "cn_official",
    icon: "bailian",
    iconColor: "#624AFF",
  },
  {
    name: "Kimi",
    websiteUrl: "https://api.kimi.com/coding/v1",
    apiKeyUrl: "https://api.kimi.com/coding/v1",
    settingsConfig: {
      models: {
        k3: {
          name: "K3",
        },
      },
      thinking: {
        type: "enabled",
      },
      reasoning_effort: "high",
      options: {
        baseURL: "https://api.kimi.com/coding/v1",
        // API Key 必须由用户在表单中填写，不写入源码
        apiKey: "",
      },
    },
    category: "cn_official",
    icon: "kimi",
    iconColor: "#6366F1",
  },
  {
    name: "DeepSeek",
    websiteUrl: "https://platform.deepseek.com/usage",
    apiKeyUrl: "https://platform.deepseek.com/usage",
    settingsConfig: {
      models: {
        "deepseek-v4-pro": {
          name: "V4-Pro",
        },
      },
      thinking: {
        type: "enabled",
      },
      reasoning_effort: "high",
      options: {
        baseURL: "https://api.deepseek.com",
        // API Key 必须由用户在表单中填写，不写入源码
        apiKey: "",
      },
    },
    category: "cn_official",
    icon: "deepseek",
    iconColor: "#1E88E5",
  },
];
