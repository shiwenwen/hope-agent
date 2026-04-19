import { useTranslation } from "react-i18next"

export default function AboutPanel() {
  const { t } = useTranslation()

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="flex flex-col items-center text-center py-8 max-w-xl mx-auto">
        {/* App Icon */}
        <div className="w-20 h-20 rounded-2xl bg-gradient-to-br from-primary/20 via-primary/10 to-transparent border border-border/50 flex items-center justify-center mb-5 shadow-lg">
          <span className="text-3xl font-bold text-primary">OC</span>
        </div>

        <h2 className="text-xl font-bold text-foreground mb-1">Hope Agent</h2>
        <p className="text-xs text-muted-foreground mb-4">{t("about.version")} 0.1.0</p>

        <p className="text-sm text-muted-foreground leading-relaxed max-w-sm mb-6">
          {t("about.description")}
        </p>

        <div className="flex items-center gap-4">
          <a
            href="https://github.com"
            target="_blank"
            rel="noreferrer"
            className="text-xs text-muted-foreground hover:text-foreground transition-colors underline underline-offset-2"
          >
            {t("about.github")}
          </a>
        </div>
      </div>

      {/* Tech Stack */}
      <div className="border-t border-border pt-5 mt-2 max-w-xl mx-auto">
        <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-3">
          {t("about.techStack")}
        </h3>
        <div className="grid grid-cols-2 gap-2 text-xs">
          {[
            ["Frontend", "React 19 + TypeScript"],
            ["Backend", "Rust + Tauri 2"],
            ["Styling", "Tailwind CSS v4"],
            ["UI", "shadcn/ui (Radix)"],
          ].map(([label, value]) => (
            <div
              key={label}
              className="flex flex-col gap-0.5 bg-secondary/40 rounded-lg px-3 py-2 border border-border/30"
            >
              <span className="text-muted-foreground">{label}</span>
              <span className="text-foreground font-medium">{value}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}
