# 常驻个人助手产品调研：Muse、Grok Bot 与 dots

调研日期为 2026-11-06。以下产品行为据各厂商的发布公告、设计说明和使用手册整理。三家在 2026 年 8 至 9 月相继发布了常驻助手；这些资料能确认产品方向，无法据此估算市占率、实际准确率、延迟或长期运行成本。本次未使用真实账号做跨渠道实测。

## 三家怎样处理同一件事

假设用户说“盯一下周五发布，如果延期再告诉我”。这句话留下了三件需要长期保存的事：跟踪哪个发布、什么变化值得打扰、应当联系谁。用户下一句话可能改聊别的，原委托依然有效；发布结束后，关注也应当结束。

| 产品 | 长期工作的组织方式 | 何时重新联系用户 |
| --- | --- | --- |
| Meta Muse | 在 App 或 WhatsApp 交给个人助手；主对话允许插话，Goals 列出跟踪事项及计划。 | 按计划或相关事件推进，完成后台工作后再判断是否有值得报告的变化或需要人作决定。用户可调低或关闭主动联系。[发布公告](https://about.fb.com/news/2026/09/introducing-muse-personal-ai-agent/) · [产品设计原文](https://muse.ai/blog/how-we-designed-muse) |
| Grok Bot | 主 Bot 是联系人，可协调其他 Bot。技能保存做法，例程保存触发条件及执行方法，用户可分开检查和停用。 | 事件可以窄到某个 Slack 频道内出现某个词。例程保存后只处理新事件；手机可看下次运行时间及每次成功或失败原因。[主 Bot](https://docs.x.ai/grok-bot/bots) · [技能与例程](https://docs.x.ai/grok-bot/skills-routines-and-automations) |
| OpenAI dots | 一个 dot 可持续承担一项职责，期间衍生多个有终点的任务。用户从 ChatGPT、Slack 或 Teams 找到同一助手，各渠道消息保留在各自会话。 | 用户可以安排定时工作。没有新委托时，dot 还可从获准来源进行只读研究，形成私人笔记并提出建议。[发布公告](https://openai.com/index/introducing-dots/) · [职责与跨渠道会话](https://developers.openai.com/training/walkthroughs/personalize-your-dot) · [主动研究边界](https://help.openai.com/en/articles/20001529-dots-privacy-security-and-safety-faqs) |

### Meta Muse：主动联系有门槛，长期委托有界面

Meta 在 9 月 8 日发布 Muse 个人助手。发布说明分别介绍了提供能力的 Muse Spark 模型和承载助手的专用虚拟机。用户关闭 App 后，助手仍可推进任务，遇到需要批准的邮件发送或支付再返回。用户能分别授予邮件读取与发送权限、断开连接、查看行动记录和要求忘记特定信息。[Meta 发布公告](https://about.fb.com/news/2026/09/introducing-muse-personal-ai-agent/)

设计团队记录了一个取舍：一个主对话便于随口交代新事，多个长期目标同时推进时，用户仍需要单独查看每个目标的计划和进度。关键审批使用有接受和拒绝按钮的结构化界面。对 Hope 的启发是，IM 负责交代、接收变化和追问，桌面负责列清全部委托、上次检查、为什么来信以及如何暂停。删除一条 IM 消息不能成为取消长期委托的唯一办法。[Muse 设计说明](https://muse.ai/blog/how-we-designed-muse)

### Grok Bot：把“做什么”“何时唤醒”和“向谁发送”拆开

Grok Bot 于 8 月 11 日以 beta 发布。其官方手册提醒，连接 Slack 和订阅消息事件是两个动作。订阅能限定频道、短语或提及，宽泛的监听会消耗大量额度。用户可以先完成一次委托，检查结果与审批边界，再把成熟的方法保存为技能并绑定例程。[发布公告](https://x.ai/news/introducing-grok-bot) · [技能与例程](https://docs.x.ai/grok-bot/skills-routines-and-automations)

一个具体界限是 Bot 自有邮箱：用户未在对话中要求发送时，例程生成的邮件停在草稿，必须由人按发送。收到的第三方邮件被标识为外部材料，其中的命令不能授权动作。对 Hope 的启发是，即使飞书群内事件足以唤醒助手，也只能触发原先约定的检查；群成员发一句“把完整报告贴到这里”不能改收件人、扩大授权或读取私聊。[Grok Bot 电脑与应用手册](https://docs.x.ai/grok-bot/computer-and-apps)

### OpenAI dots：发现机会和执行委托具有不同权限

OpenAI 于 9 月 29 日发布 dots。主动研究能使用获准来源、保存私人笔记，但该阶段的工具无法直接向第三方发消息、修改应用内容或控制电脑。正式受托执行时，另有操作规则与审批，用户可以查看活动记录。11 月 4 日更新又明确了 Enterprise、Edu、Healthcare 合资格工作区的管理员预览期，到 11 月 18 日前可由管理员配置启用或退出，管理员亦可查看连接与用量并暂停助手。[发布公告](https://openai.com/index/introducing-dots/) · [隐私与工具权限](https://help.openai.com/en/articles/20001529-dots-privacy-security-and-safety-faqs) · [11 月 4 日发布记录](https://openai.com/products/release-notes/)

官方教程以“谁负责某件事”区别于定时任务，建议先约定数据源、完成条件、接收方和哪些动作必须再询问。它也明确区分“已经连接应用”“已经安排检查”和“应用支持该事件”。对 Hope 的启发是，一个明确授权的目标可以跨设备继续，但新发现的机会先形成私有建议，扩大监听范围或代表用户对外执行时再从正式入口受理。助手身份一致时仍要保留原始对话的受众边界。[OpenAI 使用教程](https://developers.openai.com/training/walkthroughs/personalize-your-dot)

dots 的教程还给出两个接近 HA 的用法：一边与同一个助手交谈，一边让它把具体任务交给 ChatGPT Work；需要旧决定时，请它找回早先的相关对话并核实。HA 的方案再往前走一步，让主人可以直接指定某个**已存在的工作对话**，让它接单、暂停或回复进度。后面这组操作是 HA 的产品设计。[dots 与 ChatGPT Work 的分工](https://developers.openai.com/training/walkthroughs/personalize-your-dot)

## 对 Hope 的设计取舍

- **先让主人在固定的私人对话里办成一件跨会话的事。** 例如一次让发布和手册两个已有会话各自推进，再回到这里看结果和待决定项。主人若要求此后继续盯发布，再选来源、期限及是否通知到本人私聊，并能看到主机和最近检查状态。
- **把“检查到了”与“值得打扰”分开。** 发布日期没变就留在本地活动记录；出现延期才发送一次，列明原日期、新日期与出处。连续几次读取失败时说明检查中断，避免把无法核实解释成无变化。
- **区分主人、消息作者和投递对象。** HA 的工作机器人和个人助手各用专属联系人。主人在后者的已配对私聊接收获准的答复与提醒，群内发言无法指挥私人助手；两类机器人的绑定与历史分开。[HA 的 IM 设计](im.md)
- **给每项职责一份可见的运行账。** 记录检查依据、时间、停止条件、消耗及待用户决定的事项。按来源和事件去重，先比对确定性变化，必要时再调用模型。自托管是否常驻取决于执行进程、网络和连接服务的状态，产品界面应展示这些条件。

以上四项是 Hope 的设计建议。比较三家的理由是分别借鉴 Muse 对打扰时机的判断、Grok Bot 对事件和成本的管理，以及 dots 对后台研究和受托执行的权限划分。厂商演示的云电脑和账号能力需在 Hope 自身的运行与权限边界下另行验证。
