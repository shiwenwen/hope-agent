import { CheckCircle2, XCircle, Clock, Link2, Shield } from "lucide-react"

interface TestStep {
  endpoint: string
  method: string
  auth?: string
  status?: number
  latencyMs?: number
  error?: string
}

interface TestResultData {
  success: boolean
  message: string
  url?: string
  status?: number
  latencyMs?: number
  auth?: string
  detail?: string
  steps?: TestStep[]
}

export interface TestResult {
  ok: boolean
  data: TestResultData
}

/** Parse test_provider response (JSON string) into structured TestResult */
// eslint-disable-next-line react-refresh/only-export-components
export function parseTestResult(raw: string, isError: boolean): TestResult {
  try {
    const data = JSON.parse(raw) as TestResultData
    return { ok: data.success ?? !isError, data }
  } catch {
    return {
      ok: !isError,
      data: { success: !isError, message: raw },
    }
  }
}

export default function TestResultDisplay({ result }: { result: TestResult }) {
  const { ok, data } = result

  return (
    <div
      className={`text-xs rounded-lg border overflow-hidden ${
        ok ? "bg-green-500/10 border-green-500/20" : "bg-red-500/10 border-red-500/20"
      }`}
    >
      {/* Header */}
      <div
        className={`flex items-center gap-2 px-3 py-2 ${ok ? "text-green-400" : "text-red-400"}`}
      >
        {ok ? (
          <CheckCircle2 className="h-3.5 w-3.5 shrink-0" />
        ) : (
          <XCircle className="h-3.5 w-3.5 shrink-0" />
        )}
        <span className="font-medium">{data.message}</span>
        {data.latencyMs != null && data.latencyMs > 0 && (
          <span className="ml-auto flex items-center gap-1 text-muted-foreground">
            <Clock className="h-3 w-3" />
            {data.latencyMs}ms
          </span>
        )}
      </div>

      {/* Details */}
      {(data.url || data.auth || data.steps?.length) && (
        <div className="border-t border-border/30 px-3 py-1.5 space-y-1 text-muted-foreground">
          {data.url && (
            <div className="flex items-center gap-1.5 truncate">
              <Link2 className="h-3 w-3 shrink-0" />
              <span className="truncate font-mono">{data.url}</span>
              {data.status != null && data.status > 0 && (
                <span
                  className={`ml-auto shrink-0 px-1.5 py-0.5 rounded text-[10px] font-medium ${
                    data.status < 300
                      ? "bg-green-500/20 text-green-400"
                      : data.status < 400
                        ? "bg-yellow-500/20 text-yellow-400"
                        : "bg-red-500/20 text-red-400"
                  }`}
                >
                  {data.status}
                </span>
              )}
            </div>
          )}
          {data.auth && (
            <div className="flex items-center gap-1.5">
              <Shield className="h-3 w-3 shrink-0" />
              <span>认证方式: {data.auth}</span>
            </div>
          )}

          {/* Test steps */}
          {data.steps && data.steps.length > 1 && (
            <div className="pt-1 space-y-0.5">
              <span className="text-[10px] font-medium">测试步骤:</span>
              {data.steps.map((step, i) => (
                <div key={i} className="flex items-center gap-1.5 text-[10px] pl-2">
                  <span
                    className={`shrink-0 w-1.5 h-1.5 rounded-full ${
                      step.error
                        ? "bg-red-400"
                        : step.status && step.status < 400
                          ? "bg-green-400"
                          : "bg-yellow-400"
                    }`}
                  />
                  <span className="font-mono truncate">
                    {step.method} {step.endpoint.replace(/^https?:\/\/[^/]+/, "")}
                  </span>
                  {step.status && <span className="ml-auto shrink-0">{step.status}</span>}
                  {step.latencyMs != null && (
                    <span className="shrink-0 text-muted-foreground/60">{step.latencyMs}ms</span>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Error detail (truncated) */}
      {data.detail && (
        <div className="border-t border-border/30 px-3 py-1.5">
          <pre className="text-[10px] text-muted-foreground/70 whitespace-pre-wrap break-all max-h-20 overflow-y-auto">
            {data.detail.slice(0, 500)}
          </pre>
        </div>
      )}
    </div>
  )
}
