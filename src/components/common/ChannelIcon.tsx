import { MessageCircle } from "lucide-react"
import { cn } from "@/lib/utils"
import {
  SiTelegram, SiTelegramHex,
  SiDiscord, SiDiscordHex,
  SiWhatsapp, SiWhatsappHex,
  SiWechat, SiWechatHex,
} from "@icons-pack/react-simple-icons"

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
