## 任务状态说明

- `[ ]` 未开始
- `[~]` 进行中
- `[x]` 已完成

## 任务清单

- [x] 建立前端滚动状态测试入口，覆盖 detached 后不自动滚底、手动恢复跟随。
- [x] 重构 `useVirtualFeed` 的 auto-follow 状态，避免 streaming 开始/结束抢回用户滚动位置。
- [x] 在主聊天 `MessageList` 增加圆形向下箭头恢复跟随入口。
- [x] 在 `QuickChatMessages` 增加紧凑版圆形向下箭头恢复跟随入口。
- [x] 补充发送新消息时强制滚到底部并恢复 auto-follow 的场景与测试。
- [x] 补齐 i18n 文案。
- [x] 执行针对性验证并记录结果。
- [x] 调整 auto-follow 恢复按钮 UI：主聊天与 Quick Chat 使用无横线向下箭头、圆形 icon-only 按钮、hover 小手，移除“跳到最新”可见文案。
