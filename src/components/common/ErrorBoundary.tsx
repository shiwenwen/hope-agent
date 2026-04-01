import React from "react"
import { useTranslation } from "react-i18next"

interface ErrorBoundaryState {
  hasError: boolean
  error: Error | null
}

interface ErrorBoundaryProps {
  children: React.ReactNode
}

class ErrorBoundaryInner extends React.Component<
  ErrorBoundaryProps & { fallbackRender: (error: Error | null, reset: () => void) => React.ReactNode },
  ErrorBoundaryState
> {
  state: ErrorBoundaryState = { hasError: false, error: null }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error }
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("[ErrorBoundary]", error, info.componentStack)
  }

  handleReset = () => {
    this.setState({ hasError: false, error: null })
  }

  render() {
    if (this.state.hasError) {
      return this.props.fallbackRender(this.state.error, this.handleReset)
    }
    return this.props.children
  }
}

function ErrorFallback({ error, onReset }: { error: Error | null; onReset: () => void }) {
  const { t } = useTranslation()
  return (
    <div className="flex flex-col items-center justify-center h-screen gap-4 p-8 text-center">
      <div className="text-4xl">:(</div>
      <h2 className="text-lg font-semibold text-foreground">
        {t("error.appCrash", "Something went wrong")}
      </h2>
      <p className="text-sm text-muted-foreground max-w-md">
        {error?.message || t("error.unknown", "An unexpected error occurred")}
      </p>
      <button
        onClick={onReset}
        className="px-4 py-2 rounded-md bg-primary text-primary-foreground text-sm hover:opacity-90 transition-opacity"
      >
        {t("error.retry", "Try Again")}
      </button>
    </div>
  )
}

export default function ErrorBoundary({ children }: ErrorBoundaryProps) {
  return (
    <ErrorBoundaryInner
      fallbackRender={(error, reset) => <ErrorFallback error={error} onReset={reset} />}
    >
      {children}
    </ErrorBoundaryInner>
  )
}
