import { MessageCircle, Slack as LucideSlack } from "lucide-react"
import { cn } from "@/lib/utils"
import {
  SiTelegram, SiTelegramHex,
  SiDiscord, SiDiscordHex,
  SiWhatsapp, SiWhatsappHex,
  SiWechat, SiWechatHex,
  SiQq, SiQqHex,
} from "@icons-pack/react-simple-icons"

const FEISHU_HEX = "#3370FF"

function FeishuIcon({ size, className, color }: { size?: number | string; className?: string; color?: string }) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 24 24"
      fill={color || FEISHU_HEX}
      className={className}
      width={size}
      height={size}
    >
      <path d="M3.42 7.83a9.65 9.65 0 0 1 4.19-5.71A5.97 5.97 0 0 0 6.1 7.39l-2.68.44Zm17.33 2.5a9.54 9.54 0 0 0-3.58-5.81L12 9.15l4.43 3.13a18.6 18.6 0 0 1 4.32-1.95ZM5.3 8.5 2.3 9a.5.5 0 0 0-.24.86l7.5 6.18a.5.5 0 0 0 .38.12l5.47-.6a17.5 17.5 0 0 0-4.57-3.43L5.3 8.5Zm5.53 8.74-1.29.14a34.7 34.7 0 0 0 3.35 4.68.5.5 0 0 0 .82-.12c1-2.14 1.7-4.04 2.2-5.7l-5.08.56Z" />
    </svg>
  )
}

interface ChannelIconEntry {
  icon: React.ComponentType<{ size?: number | string; className?: string; color?: string }>
  color: string
}

const CHANNEL_ICONS: Record<string, ChannelIconEntry> = {
  telegram: { icon: SiTelegram, color: SiTelegramHex },
  discord: { icon: SiDiscord, color: SiDiscordHex },
  whatsapp: { icon: SiWhatsapp, color: SiWhatsappHex },
  wechat: { icon: SiWechat, color: SiWechatHex },
  weixin: { icon: SiWechat, color: SiWechatHex },
  slack: { icon: LucideSlack, color: "#4A154B" },
  feishu: { icon: FeishuIcon, color: FEISHU_HEX },
  qqbot: { icon: SiQq, color: SiQqHex },
}

export default function ChannelIcon({
  channelId,
  className,
}: {
  channelId: string
  className?: string
}) {
  const entry = CHANNEL_ICONS[channelId.toLowerCase()]
  if (entry) {
    const Icon = entry.icon
    return <Icon color={entry.color} className={cn("h-3 w-3", className)} />
  }
  return <MessageCircle className={cn("h-3 w-3", className)} />
}
