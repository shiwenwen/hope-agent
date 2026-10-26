// @vitest-environment jsdom
import { afterEach, beforeEach, expect, test, vi } from "vitest"
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { ModelDiscovery } from "./ModelDiscovery"
import { CustomWizard } from "./CustomWizard"
import { useState } from "react"
import type { ProviderConfig, ModelConfig } from "./types"

const mock = vi.hoisted(() => ({ call: vi.fn() }))
vi.mock("@/lib/transport-provider", () => ({ getTransport: () => mock }))
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, values?: { id?: string; count?: number }) =>
      values?.id ? `${key} ${values.id}` : values?.count != null ? `${key} ${values.count}` : key,
  }),
}))

const config: ProviderConfig = {
  id: "provider-1",
  name: "Custom",
  baseUrl: "https://synthetic.example/v1",
  apiType: "openai-chat",
  apiKey: "masked...key",
  authProfiles: [],
  enabled: true,
  userAgent: "synthetic",
  thinkingStyle: "none",
  models: [
    {
      id: "existing",
      name: "Saved name",
      inputTypes: ["text", "image"],
      contextWindow: 32000,
      maxTokens: 4000,
      reasoning: true,
      costInput: 1,
      costOutput: 2,
    },
  ],
}
const discovery = {
  models: [
    { id: "existing", name: "Different remote name" },
    { id: "new", name: "New model" },
    { id: "other", name: "Other model" },
  ],
  truncated: false,
}

beforeEach(() => {
  mock.call.mockReset()
})
afterEach(cleanup)

test("requires explicit selection, valid budgets and capability confirmation without overwriting saved models", async () => {
  const before = structuredClone(config)
  const add = vi.fn()
  mock.call.mockResolvedValue(discovery)
  render(<ModelDiscovery config={config} onAdd={add} />)
  expect(mock.call).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole("button", { name: "provider.discovery.action" }))
  await screen.findByText("New model")
  expect(screen.queryByText("Different remote name")).toBeNull()
  expect(screen.getByRole("button", { name: "provider.discovery.add 0" })).toBeDisabled()
  fireEvent.click(screen.getByRole("switch", { name: "provider.discovery.select new" }))
  const context = screen.getByRole("spinbutton", { name: "provider.discovery.context new" })
  const output = screen.getByRole("spinbutton", { name: "provider.discovery.output new" })
  expect(context).toHaveValue(null)
  expect(output).toHaveValue(null)
  fireEvent.change(context, { target: { value: "4000" } })
  fireEvent.change(output, { target: { value: "5000" } })
  fireEvent.click(screen.getByRole("switch", { name: "provider.discovery.confirm" }))
  expect(screen.getByRole("button", { name: "provider.discovery.add 1" })).toBeDisabled()
  fireEvent.change(output, { target: { value: "1000" } })
  // Editing a parameter invalidates an earlier confirmation.
  expect(screen.getByRole("switch", { name: "provider.discovery.confirm" })).not.toBeChecked()
  fireEvent.click(screen.getByRole("switch", { name: "provider.discovery.confirm" }))
  fireEvent.click(screen.getByRole("button", { name: "provider.discovery.add 1" }))
  expect(add).toHaveBeenCalledWith([
    {
      id: "new",
      name: "New model",
      inputTypes: ["text"],
      contextWindow: 4000,
      maxTokens: 1000,
      reasoning: false,
      costInput: null,
      costOutput: null,
    },
  ])
  expect(config).toEqual(before)
  expect(mock.call).toHaveBeenCalledExactlyOnceWith("discover_provider_models", { config })
})

test("a failed discovery is retryable and does not render a sensitive error echo", async () => {
  const add = vi.fn()
  mock.call
    .mockRejectedValueOnce(new Error("synthetic-secret-in-error"))
    .mockResolvedValueOnce({ models: [], truncated: true })
  render(<ModelDiscovery config={config} onAdd={add} />)
  fireEvent.click(screen.getByRole("button", { name: "provider.discovery.action" }))
  expect(await screen.findByRole("alert")).toHaveTextContent("provider.discovery.error")
  expect(screen.queryByText(/synthetic-secret/)).toBeNull()
  fireEvent.click(screen.getByRole("button", { name: "common.retry" }))
  await screen.findByText("provider.discovery.empty")
  expect(screen.getByText("provider.discovery.truncated")).toBeTruthy()
  expect(add).not.toHaveBeenCalled()
})

test("cancel discards an in-flight response", async () => {
  let resolve!: (value: typeof discovery) => void
  mock.call.mockReturnValue(
    new Promise((done) => {
      resolve = done
    }),
  )
  const add = vi.fn()
  render(<ModelDiscovery config={config} onAdd={add} />)
  fireEvent.click(screen.getByRole("button", { name: "provider.discovery.action" }))
  fireEvent.click(screen.getByRole("button", { name: "common.cancel" }))
  await act(async () => resolve(discovery))
  expect(screen.queryByRole("dialog")).toBeNull()
  expect(add).not.toHaveBeenCalled()
})

test("changing endpoint or credentials invalidates old requests and candidates", async () => {
  let resolve!: (value: typeof discovery) => void
  mock.call.mockReturnValueOnce(
    new Promise((done) => {
      resolve = done
    }),
  )
  const add = vi.fn()
  const view = render(<ModelDiscovery config={config} onAdd={add} />)
  fireEvent.click(screen.getByRole("button", { name: "provider.discovery.action" }))
  const next = {
    ...config,
    baseUrl: "https://other.synthetic.example/v1",
    apiKey: "new-synthetic-key",
  }
  view.rerender(<ModelDiscovery config={next} onAdd={add} />)
  await act(async () => resolve(discovery))
  expect(screen.queryByRole("dialog")).toBeNull()
  mock.call.mockResolvedValue({ models: [{ id: "fresh", name: "Fresh" }], truncated: false })
  fireEvent.click(screen.getByRole("button", { name: "provider.discovery.action" }))
  await screen.findByText("Fresh")
  expect(screen.queryByText("New model")).toBeNull()
  expect(mock.call).toHaveBeenLastCalledWith("discover_provider_models", { config: next })
  expect(add).not.toHaveBeenCalled()
})

test("existing model additions while the modal is open cannot be imported again", async () => {
  mock.call.mockResolvedValue(discovery)
  const add = vi.fn()
  const view = render(<ModelDiscovery config={config} onAdd={add} />)
  fireEvent.click(screen.getByRole("button", { name: "provider.discovery.action" }))
  fireEvent.click(await screen.findByRole("switch", { name: "provider.discovery.select new" }))
  view.rerender(
    <ModelDiscovery
      config={{ ...config, models: [...config.models, { ...config.models[0], id: " new " }] }}
      onAdd={add}
    />,
  )
  await waitFor(() =>
    expect(screen.queryByRole("switch", { name: "provider.discovery.select new" })).toBeNull(),
  )
  expect(screen.getByRole("button", { name: "provider.discovery.add 0" })).toBeDisabled()
  expect(add).not.toHaveBeenCalled()
})

test("unsupported OAuth and empty base URL never dispatch discovery", () => {
  const view = render(<ModelDiscovery config={{ ...config, apiType: "codex" }} onAdd={vi.fn()} />)
  expect(screen.getByRole("button", { name: "provider.discovery.action" })).toBeDisabled()
  view.rerender(<ModelDiscovery config={{ ...config, baseUrl: "" }} onAdd={vi.fn()} />)
  expect(screen.getByRole("button", { name: "provider.discovery.action" })).toBeDisabled()
  expect(mock.call).not.toHaveBeenCalled()
})

test("the custom-provider wizard appends confirmed models to its draft", async () => {
  const save = vi.fn()
  mock.call.mockResolvedValue(discovery)
  function Wizard() {
    const [models, setModels] = useState<ModelConfig[]>(config.models)
    return (
      <CustomWizard
        customStep={2}
        setCustomStep={vi.fn()}
        allowPrivateNetwork={false}
        setAllowPrivateNetwork={vi.fn()}
        apiType="openai-chat"
        setApiType={vi.fn()}
        providerName="Custom"
        setProviderName={vi.fn()}
        baseUrl={config.baseUrl}
        setBaseUrl={vi.fn()}
        apiKey=""
        setApiKey={vi.fn()}
        models={models}
        setModels={setModels}
        thinkingStyle="none"
        setThinkingStyle={vi.fn()}
        currency="USD"
        setCurrency={vi.fn()}
        showApiKey={false}
        setShowApiKey={vi.fn()}
        testResult={null}
        setTestResult={vi.fn()}
        testLoading={false}
        setTestLoading={vi.fn()}
        saving={false}
        error=""
        onBack={vi.fn()}
        onSave={() => save(models)}
      />
    )
  }
  render(<Wizard />)
  fireEvent.click(screen.getByRole("button", { name: "provider.discovery.action" }))
  fireEvent.click(await screen.findByRole("switch", { name: "provider.discovery.select new" }))
  fireEvent.change(screen.getByRole("spinbutton", { name: "provider.discovery.context new" }), {
    target: { value: "16000" },
  })
  fireEvent.change(screen.getByRole("spinbutton", { name: "provider.discovery.output new" }), {
    target: { value: "2000" },
  })
  fireEvent.click(screen.getByRole("switch", { name: "provider.discovery.confirm" }))
  fireEvent.click(screen.getByRole("button", { name: "provider.discovery.add 1" }))
  expect(save).not.toHaveBeenCalled()
  expect(mock.call).toHaveBeenCalledExactlyOnceWith("discover_provider_models", {
    config: expect.objectContaining({
      apiKey: "",
      baseUrl: config.baseUrl,
      models: config.models,
      currency: "USD",
      allowPrivateNetwork: false,
    }),
  })
  expect(screen.getByDisplayValue("New model")).toBeTruthy()
  expect(screen.getByDisplayValue("Saved name")).toBeTruthy()
})

test.each([false, true])(
  "wizard private-network consent defaults off and is sent only as selected: %s",
  async (allowed) => {
    mock.call.mockImplementation(async (command: string) =>
      command === "test_provider" ? "Connected" : discovery,
    )
    function Wizard() {
      const [step, setStep] = useState(1)
      const [privateNetwork, setPrivateNetwork] = useState(false)
      return (
        <CustomWizard
          customStep={step}
          setCustomStep={setStep}
          allowPrivateNetwork={privateNetwork}
          setAllowPrivateNetwork={setPrivateNetwork}
          apiType="openai-chat"
          setApiType={vi.fn()}
          providerName="Local"
          setProviderName={vi.fn()}
          baseUrl="http://192.168.50.5:11434/v1"
          setBaseUrl={vi.fn()}
          apiKey="synthetic-local-key"
          setApiKey={vi.fn()}
          models={[]}
          setModels={vi.fn()}
          thinkingStyle="none"
          setThinkingStyle={vi.fn()}
          currency="USD"
          setCurrency={vi.fn()}
          showApiKey={false}
          setShowApiKey={vi.fn()}
          testResult={null}
          setTestResult={vi.fn()}
          testLoading={false}
          setTestLoading={vi.fn()}
          saving={false}
          error=""
          onBack={vi.fn()}
          onSave={vi.fn()}
        />
      )
    }
    render(<Wizard />)
    const consent = screen.getByRole("switch", { name: "provider.allowPrivateNetwork" })
    expect(consent).not.toBeChecked()
    if (allowed) fireEvent.click(consent)
    fireEvent.click(screen.getByRole("button", { name: "provider.testConnection" }))
    await waitFor(() =>
      expect(mock.call).toHaveBeenCalledWith("test_provider", {
        config: expect.objectContaining({ allowPrivateNetwork: allowed }),
      }),
    )
    fireEvent.click(screen.getByRole("button", { name: "common.nextStep" }))
    fireEvent.click(screen.getByRole("button", { name: "provider.discovery.action" }))
    await screen.findByText("New model")
    expect(mock.call).toHaveBeenCalledWith("discover_provider_models", {
      config: expect.objectContaining({
        allowPrivateNetwork: allowed,
        baseUrl: "http://192.168.50.5:11434/v1",
      }),
    })
  },
)
