---
name: feishu
description: "Use when the user mentions 飞书 / Feishu / Lark workspace operations: docx (云文档) read/write, bitable (多维表格) records / views / dashboards, drive (云盘) upload/download, wiki (知识库) link resolution, approval (审批) instance create/cancel/query, calendar (日历) event create/list/update + attendees, contact (联系人) user/department lookup, hire (招聘) job/talent/application listing. Trigger on phrases like 'OKR 周报', '把这份文档发到飞书云盘', '给团队拉个评审会议', '查 [姓名] 的联系方式', '撤销那条审批', '/wiki 链接', or any request that mentions a feishu / lark URL / token (doxcn.../bascn.../wikcn.../boxcn.../om_...)."
paths:
  - "**/*飞书*"
  - "**/*feishu*"
  - "**/*lark*"
allowed-tools:
  - feishu_docx_create
  - feishu_docx_get_blocks
  - feishu_docx_append_block
  - feishu_docx_update_block_text
  - feishu_bitable_list_records
  - feishu_bitable_search_records
  - feishu_bitable_create_record
  - feishu_bitable_batch_update_records
  - feishu_bitable_list_views
  - feishu_bitable_get_view
  - feishu_bitable_list_dashboards
  - feishu_drive_list_files
  - feishu_drive_upload_media
  - feishu_drive_download_media
  - feishu_wiki_get_node
  - feishu_approval_create_instance
  - feishu_approval_get_instance
  - feishu_approval_cancel_instance
  - feishu_approval_list_instances
  - feishu_approval_subscribe
  - feishu_calendar_list
  - feishu_calendar_create_event
  - feishu_calendar_list_events
  - feishu_calendar_update_event
  - feishu_calendar_delete_event
  - feishu_calendar_attendees_create
  - feishu_contact_get_user
  - feishu_contact_batch_get_users
  - feishu_contact_get_department
  - feishu_contact_search_users_by_department
  - feishu_hire_list_jobs
  - feishu_hire_get_job
  - feishu_hire_list_talents
  - feishu_hire_get_talent
  - feishu_hire_list_applications
  - read
  - web_search
---

# 飞书 (Lark) 工作流套件

35 个 `feishu_*` tool 覆盖飞书除 IM 之外的核心业务面：docx / bitable / drive / wiki / approval / calendar / contact / hire。所有 tool 共享同一个账号路由——`account` 参数仅在配了 ≥2 个飞书账号时才必须，否则自动选唯一一个。

## 典型工作流剧本

### 1. OKR 周报：bitable → docx → drive

```
1. feishu_bitable_list_records({app_token, table_id, view_id?})
   → 拿到本周的 OKR 进度数据
2. feishu_docx_create({title: "OKR Weekly W#"})
   → 拿到 document_id
3. feishu_docx_append_block({document_id, parent_block_id: document_id,
     block: {block_type: 2, text: {style: {}, elements: [...]}}})
   → 写多段
4. feishu_drive_upload_media({path: "/tmp/screenshot.png",
     folder_token: "...", mime: "image/png"})
   → 把截图传上去（≤20MB，本地路径必须绝对）
```

### 2. 排会议：calendar → attendees

```
1. feishu_calendar_list() → 选目标 calendar_id
2. feishu_calendar_create_event({calendar_id, event: {
     summary: "OKR review", start_time: {timestamp: "1700000000",
     timezone: "Asia/Shanghai"}, end_time: {...}}})
   → 拿到 event_id
3. feishu_calendar_attendees_create({calendar_id, event_id, attendees: [
     {type: "user", user_id: "ou_xxx"},
     {type: "chat", chat_id: "oc_xxx"}]})
```

### 3. 查同事：contact

```
1. feishu_contact_get_user({user_id: "ou_xxx"})  // 或先 search 找 ID
   → 名字 / email / 部门 / 上级
2. feishu_contact_search_users_by_department({department_id: "..."})
   → 整个团队
```

> ⚠️ contact 系列返回员工个人信息（手机号 / 邮箱 / 部门），**不要把原始 JSON 直接 echo 回 IM 群聊**——总结关键字段即可。

### 4. 审批：approval

```
1. feishu_approval_create_instance({approval_code, user_id, form: "[...]"})
   → ⚠️ HIGH RISK：在调之前一定问用户「我准备发起 X 审批，form 字段是 Y/Z，确认吗？」
2. feishu_approval_get_instance({instance_code}) → 看 status / timeline
3. 撤销也是 HIGH：feishu_approval_cancel_instance({approval_code, instance_code, user_id})
```

### 5. wiki 链接：先解析再读

用户给一个形如 `https://xxx.feishu.cn/wiki/wikcnXxx` 的链接时：

```
1. feishu_wiki_get_node({token: "wikcnXxx"})
   → 拿到 obj_token / obj_type
2. obj_type == "docx" → feishu_docx_get_blocks({document_id: obj_token})
   obj_type == "bitable" → feishu_bitable_list_records({app_token: obj_token, table_id: ...})
```

## 飞书 app scope 速查

每个 tool 的 description 已写明所需 scope；汇总如下（在飞书后台「权限管理」开启）：

| 模块 | 主要 scope | 备注 |
|------|-----------|------|
| docx | `docx:document` / `docx:document.readonly` | 写需 `:document` |
| bitable | `bitable:app` / `bitable:app.read` | 写需 `:app` |
| drive | `drive:drive` / `drive:drive.read` | upload / download 都需要 |
| wiki | `wiki:wiki.readonly` / `wiki:wiki` | get_node 一般 readonly 够 |
| approval | `approval:approval` | create / cancel 都需要 |
| calendar | `calendar:calendar` / `calendar:calendar.readonly` | 写需 `:calendar` |
| contact | `contact:user.id:readonly` / `contact:department.id:readonly` | 敏感数据 |
| hire | `hire:job:readonly` / `hire:talent:readonly` / `hire:application:readonly` | tenant 必须开通 hire 模块 |

## 常见错误码翻译（让用户秒懂）

| code | 含义 | 给用户的提示 |
|------|------|------------|
| `99991400` | 请求过快 / 限流 | 「飞书侧限流了，等几秒重试」 |
| `99991663` | 参数错误 / 资源不存在 | 「token 或 ID 不对，确认一下原始链接？」 |
| `99991672` | scope 未授权 | 「飞书 app 缺 X scope，去飞书后台『权限管理』勾选并发版」 |
| `99991677` | 租户未开通 | 「这个能力需要租户管理员先在工作台启用」 |
| `1061004` | hire 模块未开通 | 「招聘模块只在企业版可用，让管理员去工作台启用『飞书招聘』」 |
| `300317` | cardkit sequence 错乱 | 内部错误，已自动降级 |
| `200750` | cardkit 卡片过期 | 重新创建 |

## 风险等级（操作影响范围）

- **HIGH**：`feishu_approval_create_instance` / `feishu_approval_cancel_instance` —— 影响真实审批流，调用前**必须**向用户复述参数 + 等待确认
- **MEDIUM**：其它所有写操作（`docx_create` / `docx_append_block` / `bitable_create_record` / `drive_upload_media` / `calendar_create_event` 等）—— 列出 diff / 预期结果让用户认可一次即可
- **MEDIUM-敏感**：`feishu_contact_*` / `feishu_hire_list_talents` / `feishu_hire_get_talent` —— 返回员工/候选人个人信息；不要回显原始 JSON 到群聊或外部上下文

## 多账号

只在配了 ≥2 个飞书账号时才需要传 `account: "<channel_account_id>"`。GUI 设置入口：Settings → Channels → Feishu。账号信息走 `cached_config()` 拉取，与 IM 渠道是否运行 WebSocket 网关解耦——只想用 docx / bitable 不开 bot 的场景也支持。

## 不在本工具集内（v0.2.0 不含）

- docx 块类型扩展（image / table / code block） — v0.3+
- drive 大文件分片上传 v2（>20MB） — v0.3+
- bitable view 创建 / 自定义 / 公式 / 自动化 — v0.3+
- approval 模板定义 / approve / reject task — v0.3+（写要走飞书后台）
- calendar 会议室 / 资源预订 — v0.3+
- task / email — v0.3+
- 其它 channel 的 reaction / edit / recall 解析 — v0.3+ Phase B.3

碰到上述场景：明确告诉用户「这块在 v0.2.0 暂不覆盖，请去飞书后台手动操作 / 等下版」。
