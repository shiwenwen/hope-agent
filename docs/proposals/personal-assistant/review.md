# Hope 主动助理：整体审阅与修订

日期：2026-11-15。审阅对象为[产品完整设计](README.md)、[运行契约](runtime-design.md)及本目录调研更新。基线 main：`0a59f58db94ba44ffb867989e2a55355e5f0384f`。

审阅模型：用户指定的 Astra，实际调度标识 `gpt-6-astra`，high reasoning。独立只读审阅代理：`/root/astra_design_review`。审阅含产品、运行与既有代码接缝；未修改产品代码、运行编译测试或做真实模型/IM 验收。

## 第一轮结论

产品结构已经摆脱以 Goal 为中心；轻量事项、独立发现、后台执行和注意力判断的划分成立。第一稿发现四项 P1、两项确定性 P2，均已修订并由同一 Astra 代理进行第二轮复核。

| 问题 | 触发与既有代码依据 | 本次修订 | 验收 |
| --- | --- | --- | --- |
| P1 来源撤权未覆盖模型派生历史 | 研究结果进入普通历史/压缩后，现有 context 恢复可能重新提供旧内容；见 [context.rs](../../../crates/ha-core/src/chat_engine/context.rs) | 来源依赖贯穿工具结果、模型复述、压缩、持久上下文与产物；首版整个上下文保守失效，取消在途并隔离迟到结果 | A28、A37 |
| P1 长期待回答不能依赖 oneshot | 工具问答依赖内存等待；owner 持久回答目前仅支持 record_domain_evidence；见 [questions.rs](../../../crates/ha-core/src/ask_user/questions.rs)、[工具入口](../../../crates/ha-core/src/tools/ask_user_question.rs) | 新增持久 assistant_item_answer 目标；问题/waiting/更新原子创建，答案/终态/后续原子提交，等待释放运行资源 | A15、A26、A38、A39 |
| P1 非 Goal 执行缺累计预算 | 单次尝试限制不能约束协调器反复派新轮次/核实；见 [JobManager 配置](../../../crates/ha-config-schema/src/config.rs) | 新增事项和主人两级累计轮次/工具/活跃时长/并发守卫，核实/重试/子工作继承；耗尽等待，主人追加 | A29、A40 |
| P1 完成注入和旧通知出口双消费 | 原 JobManager 会 inject_and_run_parent，旧桌面 hook 直接消费完成；见 [injection.rs](../../../crates/ha-core/src/async_jobs/injection.rs)、[useDesktopAlerts.ts](../../../src/hooks/useDesktopAlerts.ts) | 各异步边界持久 continuation_owner；工作内部继续原父注入，根报告交协调器；助理所属旧提醒统一收口 | A17、A36、A41、A42 |
| P2 Secondary 策略保存与 Primary 实际生效有空窗 | 现有配置快照进程内缓存，文件通知可能延迟；见 [进程模型](../../architecture/system/process-model.md) | 共享 DB 先关闭认领闸门，Primary 标准配置写入并确认 revision 后发布执行快照；未应用不报已生效 | A30、A43 |
| P2 按新时区重算可能补出额度 | UTC 11-14 23:00 两次，11-15 12:00 上海切洛杉矶可落到不同当地日 | 同时检查当地日和滚动 24 小时上限，保留 UTC 尝试；回拨保守保留，展示下次可尝试时间 | A22、A44 |

主审另修正架构图：原工作 IM 与专属 IM 分别指向共用协议层，图中不再画出二者之间的路由箭头；增加一条无 Goal 的跨端完整时间线。

## 第二轮复核结论

**设计复核通过，可按 A/B/C 进入实现。** Astra 逐项确认六项原问题已在设计层闭合，未发现新的可行动 P1/P2；主文、运行契约与审阅记录无影响实施的冲突。轻量事项仍是必要的持久委托记录，Goal 只作为可选执行器；新增宿主预算、答复目标和续接归属不要求用户填写重型管理对象。

实施必须保留两条约束：累计预算、持久问答、撤权传播和唯一续接归属随对应功能一并落地；A/B/C 可分 PR，但共同通过后才验收为首个完整主动助理版本。试点可以校准频率和额度，不得以体验调参放宽授权、幂等和统一通知边界。

该结论只对当前文档设计有效，不代表代码已实现、A01–A44 已通过，或真实模型效果/IM 可用性已验收。

## 仍需试点校准

每日提醒数量、合并窗口、默认关注期限、研究频率、执行额度，以及建议价值与漏报之间的取舍均为设计初值。需要观察关键决定是否被额度长期压住；模型不得因此自行越过硬限制。

## 验证记录

- 最终设计的文档本地链接路径检查：10 个相关文件，缺失路径 0。
- Mermaid 11.14.0 + jsdom 对最终三张图执行语法解析，通过。语法解析不等同于渲染或真实运行验证。
- 最终设计 git diff --check 通过；验收矩阵 A01–A44 无缺号或重复。
- 本轮未运行编译、全套测试、真实模型或平台测试。上述 A01–A44 为待实施的验收要求，不是已通过的测试。

## 复核快照

以下 SHA-256 对应 Astra 第二轮读取的设计/调研文件；本审阅记录不计入自身快照。分支为 codex/hope-proactive-assistant-design，文档保留在当前工作树，未推送或发布。

| 文件 | SHA-256 |
| --- | --- |
| README.md | `6950a1714df19e8da61223392d3ee461b455a5723791114cde642cfbeb02561e` |
| runtime-design.md | `2596e70c4bc5c8624cfd64a506ce0318e8111d9576cb37eec492a497dbf047ad` |
| products.md | `0bb899de9067bbebc25f7d648dd649723d06edbdd724b50d78df70d4e8c1fd72` |
| open-architectures.md | `434af0e4cfc81b1081a7576d6ee46ef49248ad75856f841311d843fecb3c394d` |
