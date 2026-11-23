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
            proxy: "",
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
        proxy: "",
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
            proxy: "",
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
        proxy: "",
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
            proxy: "",
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

  it("shows an empty proxy field by default", () => {
    render(
      <KiloProviderForm
        submitLabel="Save"
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    const proxyInput = screen.getByLabelText("HTTP/HTTPS Proxy");
    expect(proxyInput).toHaveValue("");
    expect(proxyInput).not.toHaveAttribute("aria-invalid", "true");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("echoes a stored proxy string and round-trips whitespace as empty", () => {
    render(
      <KiloProviderForm
        submitLabel="Save"
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        initialData={{
          name: "Provider",
          settingsConfig: {
            models: { "glm-5.3": { name: "GLM-5.3" } },
            thinking: { type: "enabled" },
            reasoning_effort: "high",
            options: {
              baseURL: "https://example.com/v1",
              apiKey: "secret",
              proxy: "http://127.0.0.1:7890",
            },
          },
        }}
      />,
    );

    expect(screen.getByLabelText("HTTP/HTTPS Proxy")).toHaveValue(
      "http://127.0.0.1:7890",
    );

    fireEvent.change(screen.getByLabelText("HTTP/HTTPS Proxy"), {
      target: { value: "   " },
    });

    const preview = JSON.parse(
      (screen.getByLabelText("Kilo Configuration JSON") as HTMLTextAreaElement)
        .value,
    );
    expect(preview.options.proxy).toBe("");
  });

  it("trims and persists non-empty proxy URLs", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(
      <KiloProviderForm
        submitLabel="Save"
        onSubmit={onSubmit}
        onCancel={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByLabelText("Provider Name"), {
      target: { value: "Provider" },
    });
    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "https://example.com/v1" },
    });
    fireEvent.change(screen.getByLabelText("API Key"), {
      target: { value: "secret" },
    });
    fireEvent.change(screen.getByLabelText("Model ID"), {
      target: { value: "glm-5.3" },
    });
    fireEvent.change(screen.getByLabelText("Model Name"), {
      target: { value: "GLM-5.3" },
    });
    fireEvent.change(screen.getByLabelText("HTTP/HTTPS Proxy"), {
      target: { value: "  http://127.0.0.1:7890  " },
    });

    fireEvent.submit(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalled());
    const submittedConfig = JSON.parse(
      onSubmit.mock.calls[0][0].settingsConfig,
    );
    expect(submittedConfig.options.proxy).toBe("http://127.0.0.1:7890");
  });

  it("accepts IPv6 and default port proxy URLs", () => {
    render(
      <KiloProviderForm
        submitLabel="Save"
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByLabelText("HTTP/HTTPS Proxy"), {
      target: { value: "https://[::1]:8080" },
    });
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("HTTP/HTTPS Proxy"), {
      target: { value: "http://example.com" },
    });
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it.each([
    ["127.0.0.1:7890", /valid http\(s\) URL/i],
    ["socks5://127.0.0.1:1080", /scheme must be http or https/i],
    ["http://user:pass@example.com", /must not contain a username/i],
    ["http://example.com/?x=1", /must not contain a query string/i],
    ["http://example.com/#frag", /must not contain a fragment/i],
    ["http://example.com/api", /must use the root path/i],
  ])("shows an inline error for %s and blocks submit", async (value, pattern) => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(
      <KiloProviderForm
        submitLabel="Save"
        onSubmit={onSubmit}
        onCancel={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByLabelText("Provider Name"), {
      target: { value: "Provider" },
    });
    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "https://example.com/v1" },
    });
    fireEvent.change(screen.getByLabelText("API Key"), {
      target: { value: "secret" },
    });
    fireEvent.change(screen.getByLabelText("Model ID"), {
      target: { value: "glm-5.3" },
    });
    fireEvent.change(screen.getByLabelText("HTTP/HTTPS Proxy"), {
      target: { value },
    });

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(pattern);
    expect(screen.getByLabelText("HTTP/HTTPS Proxy")).toHaveAttribute(
      "aria-invalid",
      "true",
    );

    fireEvent.submit(screen.getByRole("button", { name: "Save" }));
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("prevents extraOptions from overriding the proxy field", () => {
    render(
      <KiloProviderForm
        submitLabel="Save"
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /^Add$/ }));

    const keys = screen.getAllByLabelText("Option Key");
    const values = screen.getAllByLabelText("Option Value");
    fireEvent.change(keys[keys.length - 1], { target: { value: "proxy" } });
    fireEvent.change(values[values.length - 1], {
      target: { value: "http://attacker.example.com:80" },
    });

    const preview = JSON.parse(
      (screen.getByLabelText("Kilo Configuration JSON") as HTMLTextAreaElement)
        .value,
    );
    expect(preview.options.proxy).toBe("");
    // value field should be empty since proxy is reserved
    expect(values[values.length - 1]).toHaveValue("http://attacker.example.com:80");
  });

  it("clears the proxy input when switching back to a preset", () => {
    render(
      <KiloProviderForm
        submitLabel="Save"
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByLabelText("HTTP/HTTPS Proxy"), {
      target: { value: "http://127.0.0.1:7890" },
    });
    expect(screen.getByLabelText("HTTP/HTTPS Proxy")).toHaveValue(
      "http://127.0.0.1:7890",
    );

    fireEvent.click(screen.getByRole("button", { name: /火山/ }));

    expect(screen.getByLabelText("HTTP/HTTPS Proxy")).toHaveValue("");

    fireEvent.click(
      screen.getByRole("button", { name: /providerPreset\.custom/ }),
    );

    expect(screen.getByLabelText("HTTP/HTTPS Proxy")).toHaveValue("");
  });
});
