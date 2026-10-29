import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { KiloProviderForm } from "@/components/providers/forms/KiloProviderForm";

describe("KiloProviderForm", () => {
  beforeEach(() => {
    Element.prototype.scrollIntoView = vi.fn();
  });

  it("shows one read-only JSON preview without legacy fields", () => {
    render(
      <KiloProviderForm
        submitLabel="Save"
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        initialData={{
          name: "GLM provider",
          settingsConfig: {
            name: "legacy config name",
            npm: "@ai-sdk/openai-compatible",
            models: {
              "glm-5.3": { name: "GLM-5.3" },
            },
            thinking: { type: "enabled" },
            reasoning_effort: "high",
            options: {
              baseURL: "https://example.com/v1",
              apiKey: "secret",
              timeout: "600000",
            },
          },
        }}
      />,
    );

    const preview = screen.getByLabelText("Kilo Configuration JSON");
    expect(preview).toHaveAttribute("readonly");
    expect(preview).toHaveValue(
      JSON.stringify(
        {
          models: {
            "glm-5.3": { name: "GLM-5.3" },
          },
          thinking: { type: "enabled" },
          reasoning_effort: "high",
          options: {
            baseURL: "https://example.com/v1",
            apiKey: "secret",
            timeout: "600000",
          },
        },
        null,
        2,
      ),
    );
    expect(screen.getByLabelText("API Key")).toHaveAttribute("type", "text");
    expect(screen.queryByLabelText("NPM Package")).not.toBeInTheDocument();
  });

  it("updates the preview from the same fields that are submitted", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(
      <KiloProviderForm
        submitLabel="Save"
        onSubmit={onSubmit}
        onCancel={vi.fn()}
        initialData={{
          name: "Provider",
          settingsConfig: {
            models: { old: { name: "Old" } },
            thinking: { type: "enabled" },
            reasoning_effort: "high",
            options: {
              baseURL: "https://old.example.com",
              apiKey: "old-secret",
            },
          },
        }}
      />,
    );

    fireEvent.change(screen.getByLabelText("API Key"), {
      target: { value: "new-secret" },
    });
    fireEvent.change(screen.getByLabelText("Model ID"), {
      target: { value: "new-model" },
    });
    fireEvent.change(screen.getByLabelText("Model Name"), {
      target: { value: "New Model" },
    });

    const expectedConfig = {
      models: {
        "new-model": { name: "New Model" },
      },
      thinking: { type: "enabled" },
      reasoning_effort: "high",
      options: {
        baseURL: "https://old.example.com",
        apiKey: "new-secret",
      },
    };
    expect(screen.getByLabelText("Kilo Configuration JSON")).toHaveValue(
      JSON.stringify(expectedConfig, null, 2),
    );

    fireEvent.submit(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(onSubmit).toHaveBeenCalledWith(
        expect.objectContaining({
          name: "Provider",
          settingsConfig: JSON.stringify(expectedConfig),
        }),
      ),
    );
  });

  it("filters unfinished header and option rows from the saved config", () => {
    render(
      <KiloProviderForm
        submitLabel="Save"
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        initialData={{
          name: "Provider",
          settingsConfig: {
            models: { model: { name: "Model" } },
            thinking: { type: "enabled" },
            reasoning_effort: "high",
            options: {
              baseURL: "https://example.com",
              apiKey: "secret",
              headers: {
                "X-Ready": "yes",
                "": "ignored",
                "draft-header:123": "ignored",
              },
              timeout: "600000",
            },
          },
        }}
      />,
    );

    const preview = JSON.parse(
      (screen.getByLabelText("Kilo Configuration JSON") as HTMLTextAreaElement)
        .value,
    );
    expect(preview.options.headers).toEqual({ "X-Ready": "yes" });
    expect(preview.options.timeout).toBe("600000");
  });

  it("offers the Volcano preset when creating a new Kilo provider", () => {
    render(
      <KiloProviderForm
        submitLabel="Save"
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: /火山/ })).toBeInTheDocument();

    expect(
      screen.queryByLabelText("Kilo Configuration JSON"),
    ).toBeInTheDocument();
  });

  it("fills the form and JSON preview from the Volcano preset", () => {
    render(
      <KiloProviderForm
        submitLabel="Save"
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /火山/ }));

    expect(screen.getByLabelText("Provider Name")).toHaveValue(
      "火山 Coding Plan",
    );
    expect(screen.getByLabelText("Base URL")).toHaveValue(
      "https://ark.cn-beijing.volces.com/api/coding/v3",
    );
    expect(screen.getByLabelText("Model ID")).toHaveValue("glm-5.3");
    expect(screen.getByLabelText("Model Name")).toHaveValue("GLM-5.3");
    expect(screen.getByLabelText("Thinking Type")).toHaveValue("enabled");
    expect(screen.getByLabelText("Reasoning Effort")).toHaveValue("high");

    const apiKeyInput = screen.getByLabelText("API Key") as HTMLInputElement;
    expect(apiKeyInput).toHaveValue("");
    expect(apiKeyInput).toBeRequired();

    expect(screen.getByLabelText("Kilo Configuration JSON")).toHaveValue(
      JSON.stringify(
        {
          models: {
            "glm-5.3": { name: "GLM-5.3" },
          },
          thinking: { type: "enabled" },
          reasoning_effort: "high",
          options: {
            baseURL: "https://ark.cn-beijing.volces.com/api/coding/v3",
            apiKey: "",
          },
        },
        null,
        2,
      ),
    );
  });

  it("submits the same config as the preview after filling the API key", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(
      <KiloProviderForm
        submitLabel="Save"
        onSubmit={onSubmit}
        onCancel={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /火山/ }));
    fireEvent.change(screen.getByLabelText("API Key"), {
      target: { value: "  volc-secret  " },
    });

    const expectedConfig = {
      models: {
        "glm-5.3": { name: "GLM-5.3" },
      },
      thinking: { type: "enabled" },
      reasoning_effort: "high",
      options: {
        baseURL: "https://ark.cn-beijing.volces.com/api/coding/v3",
        apiKey: "volc-secret",
      },
    };
    expect(screen.getByLabelText("Kilo Configuration JSON")).toHaveValue(
      JSON.stringify(expectedConfig, null, 2),
    );

    fireEvent.submit(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(onSubmit).toHaveBeenCalledWith(
        expect.objectContaining({
          name: "火山 Coding Plan",
          websiteUrl:
            "https://console.volcengine.com/ark/region:cn-beijing/subscription/coding-plan",
          settingsConfig: JSON.stringify(expectedConfig),
          icon: "zai",
          iconColor: "",
        }),
      ),
    );

    const submittedConfig = JSON.parse(
      onSubmit.mock.calls[0][0].settingsConfig,
    );
    expect(Object.keys(submittedConfig)).not.toContain("name");
    expect(Object.keys(submittedConfig)).not.toContain("npm");
  });

  it("resets to a blank form when switching back to custom", () => {
    render(
      <KiloProviderForm
        submitLabel="Save"
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /火山/ }));
    fireEvent.click(
      screen.getByRole("button", { name: /providerPreset\.custom/ }),
    );

    expect(screen.getByLabelText("Provider Name")).toHaveValue("");
    expect(screen.getByLabelText("Base URL")).toHaveValue("");
    expect(screen.getByLabelText("Model ID")).toHaveValue("");
    expect(screen.getByLabelText("Kilo Configuration JSON")).toHaveValue(
      JSON.stringify(
        {
          models: {},
          thinking: { type: "enabled" },
          reasoning_effort: "high",
          options: {
            baseURL: "",
            apiKey: "",
          },
        },
        null,
        2,
      ),
    );
  });

  it("hides the preset selector when editing an existing Kilo provider", () => {
    render(
      <KiloProviderForm
        submitLabel="Save"
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        initialData={{
          name: "Existing provider",
          settingsConfig: {
            models: { "glm-5.3": { name: "GLM-5.3" } },
            thinking: { type: "enabled" },
            reasoning_effort: "high",
            options: {
              baseURL: "https://example.com/v1",
              apiKey: "secret",
            },
          },
        }}
      />,
    );

    expect(
      screen.queryByRole("button", { name: /火山/ }),
    ).not.toBeInTheDocument();
    expect(screen.getByLabelText("Provider Name")).toHaveValue(
      "Existing provider",
    );
    expect(screen.getByLabelText("Base URL")).toHaveValue(
      "https://example.com/v1",
    );
    expect(screen.getByLabelText("Model ID")).toHaveValue("glm-5.3");
  });
});
