import { useTranslation } from "react-i18next"

interface LoadMoreRowProps {
  loadingMore: boolean
  onLoadMore?: () => void | Promise<void>
}

export default function LoadMoreRow({ loadingMore, onLoadMore }: LoadMoreRowProps) {
  const { t } = useTranslation()
  return (
    <div className="flex justify-center py-2">
      {loadingMore ? (
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <div className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-muted-foreground border-t-transparent" />
          {t("chat.loadingMore")}
        </div>
      ) : (
        <button
          onClick={() => onLoadMore?.()}
          className="text-xs text-muted-foreground hover:text-foreground transition-colors"
        >
          {t("chat.loadMore")}
        </button>
      )}
    </div>
  )
}
