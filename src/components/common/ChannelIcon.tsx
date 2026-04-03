import { MessageCircle } from "lucide-react"
import { cn } from "@/lib/utils"
import {
  SiTelegram, SiTelegramHex,
  SiDiscord, SiDiscordHex,
  SiWhatsapp, SiWhatsappHex,
  SiWechat, SiWechatHex,
  SiQq, SiQqHex,
  SiSignal, SiSignalHex,
  SiLine, SiLineHex,
  SiGooglechat, SiGooglechatHex,
  SiImessage, SiImessageHex,
} from "@icons-pack/react-simple-icons"

const FEISHU_HEX = "#3370FF"

function FeishuIcon({ size, className, color }: { size?: number | string; className?: string; color?: string }) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 48 48"
      fill={color || FEISHU_HEX}
      className={className}
      width={size}
      height={size}
    >
      <path fillRule="evenodd" clipRule="evenodd" d="M41.072 5.994 3.31 16.519l9.075 9.293 8.414.147 9.683-9.44a4.2 4.2 0 0 1-.116-1.318c0-.794.312-1.423.796-1.868.83-.763 1.828-.877 2.995-.342l7.183-6.997Z" />
      <path fillRule="evenodd" clipRule="evenodd" d="m42.102 6.728-10.524 37.761-9.294-9.075-.147-8.414 9.375-9.519a3.6 3.6 0 0 0 1.664.496c.903-.05 1.486-.595 1.759-.916s.594-.855.567-1.649a3.4 3.4 0 0 0-.521-1.464l7.121-7.22Z" />
    </svg>
  )
}

const IRC_HEX = "#1F8B4C"

function IrcIcon({ size, className, color }: { size?: number | string; className?: string; color?: string }) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 48 48"
      fill="none"
      stroke={color || IRC_HEX}
      strokeWidth={4}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      width={size}
      height={size}
    >
      <rect x="5.5" y="5.5" width="37" height="37" rx="4" />
      <path d="M18.773 5.5v37M29.227 5.5v37M5.5 18.773h37M5.5 29.227h37" />
    </svg>
  )
}

const SLACK_HEX = "#4A154B"

function SlackIcon({ size, className, color }: { size?: number | string; className?: string; color?: string }) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 24 24"
      fill={color || SLACK_HEX}
      className={className}
      width={size}
      height={size}
    >
      <path d="M5.042 15.165a2.528 2.528 0 0 1-2.52 2.523A2.528 2.528 0 0 1 0 15.165a2.527 2.527 0 0 1 2.522-2.52h2.52v2.52zM6.313 15.165a2.527 2.527 0 0 1 2.521-2.52 2.527 2.527 0 0 1 2.521 2.52v6.313A2.528 2.528 0 0 1 8.834 24a2.528 2.528 0 0 1-2.521-2.522v-6.313zM8.834 5.042a2.528 2.528 0 0 1-2.521-2.52A2.528 2.528 0 0 1 8.834 0a2.528 2.528 0 0 1 2.521 2.522v2.52H8.834zM8.834 6.313a2.528 2.528 0 0 1 2.521 2.521 2.528 2.528 0 0 1-2.521 2.521H2.522A2.528 2.528 0 0 1 0 8.834a2.528 2.528 0 0 1 2.522-2.521h6.312zM18.956 8.834a2.528 2.528 0 0 1 2.522-2.521A2.528 2.528 0 0 1 24 8.834a2.528 2.528 0 0 1-2.522 2.521h-2.522V8.834zM17.688 8.834a2.528 2.528 0 0 1-2.523 2.521 2.527 2.527 0 0 1-2.52-2.521V2.522A2.527 2.527 0 0 1 15.165 0a2.528 2.528 0 0 1 2.523 2.522v6.312zM15.165 18.956a2.528 2.528 0 0 1 2.523 2.522A2.528 2.528 0 0 1 15.165 24a2.527 2.527 0 0 1-2.52-2.522v-2.522h2.52zM15.165 17.688a2.527 2.527 0 0 1-2.52-2.523 2.526 2.526 0 0 1 2.52-2.52h6.313A2.527 2.527 0 0 1 24 15.165a2.528 2.528 0 0 1-2.522 2.523h-6.313z" />
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
  slack: { icon: SlackIcon, color: SLACK_HEX },
  feishu: { icon: FeishuIcon, color: FEISHU_HEX },
  qqbot: { icon: SiQq, color: SiQqHex },
  signal: { icon: SiSignal, color: SiSignalHex },
  line: { icon: SiLine, color: SiLineHex },
  googlechat: { icon: SiGooglechat, color: SiGooglechatHex },
  irc: { icon: IrcIcon, color: IRC_HEX },
  imessage: { icon: SiImessage, color: SiImessageHex },
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
