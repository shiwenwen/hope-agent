/**
 * Tool name constants — must stay in sync with `src-tauri/src/tools/mod.rs`.
 */
export const TOOL_EXEC = "exec" as const
export const TOOL_PROCESS = "process" as const
export const TOOL_READ = "read" as const
export const TOOL_WRITE = "write" as const
export const TOOL_EDIT = "edit" as const
export const TOOL_LS = "ls" as const
export const TOOL_GREP = "grep" as const
export const TOOL_FIND = "find" as const
export const TOOL_APPLY_PATCH = "apply_patch" as const
export const TOOL_WEB_SEARCH = "web_search" as const
export const TOOL_WEB_FETCH = "web_fetch" as const
export const TOOL_SAVE_MEMORY = "save_memory" as const
export const TOOL_RECALL_MEMORY = "recall_memory" as const
export const TOOL_UPDATE_MEMORY = "update_memory" as const
export const TOOL_DELETE_MEMORY = "delete_memory" as const
export const TOOL_MANAGE_CRON = "manage_cron" as const
export const TOOL_BROWSER = "browser" as const
export const TOOL_SEND_NOTIFICATION = "send_notification" as const
export const TOOL_SUBAGENT = "subagent" as const
export const TOOL_TASK_CREATE = "task_create" as const
export const TOOL_TASK_UPDATE = "task_update" as const
export const TOOL_TASK_LIST = "task_list" as const
export const TOOL_MCP_RESOURCE = "mcp_resource" as const
export const TOOL_MCP_PROMPT = "mcp_prompt" as const
export const TOOL_IMAGE_GENERATE = "image_generate" as const
export const TOOL_CANVAS = "canvas" as const
export const TOOL_ACP_SPAWN = "acp_spawn" as const

/**
 * Hardcoded ID of the "main" agent. Mirrors `agent_loader::DEFAULT_AGENT_ID`
 * on the Rust side. The user can change which agent picks up new chats via
 * `AppConfig.default_agent_id`, but the literal "ha-main" agent is always
 * the main one (it gets richer Tier 2/3 toggle defaults).
 */
export const DEFAULT_AGENT_ID = "ha-main" as const

export const isMainAgent = (id: string) => id === DEFAULT_AGENT_ID

/**
 * @deprecated Use the `internal` flag from `list_builtin_tools` API response instead.
 * Kept only as a fallback — the backend ToolDefinition.internal field is the source of truth.
 */
export const INTERNAL_TOOLS = new Set([
  TOOL_SAVE_MEMORY,
  TOOL_RECALL_MEMORY,
  TOOL_UPDATE_MEMORY,
  TOOL_DELETE_MEMORY,
  TOOL_MANAGE_CRON,
  TOOL_SEND_NOTIFICATION,
])

/** Map from tool name to i18n key suffix. */
export const TOOL_I18N_KEY: Record<string, string> = {
  [TOOL_EXEC]: "Exec",
  [TOOL_PROCESS]: "Process",
  [TOOL_READ]: "Read",
  [TOOL_WRITE]: "Write",
  [TOOL_EDIT]: "Edit",
  [TOOL_LS]: "Ls",
  [TOOL_GREP]: "Grep",
  [TOOL_FIND]: "Find",
  [TOOL_APPLY_PATCH]: "ApplyPatch",
  [TOOL_WEB_SEARCH]: "WebSearch",
  [TOOL_WEB_FETCH]: "WebFetch",
  [TOOL_SAVE_MEMORY]: "SaveMemory",
  [TOOL_RECALL_MEMORY]: "RecallMemory",
  [TOOL_UPDATE_MEMORY]: "UpdateMemory",
  [TOOL_DELETE_MEMORY]: "DeleteMemory",
  [TOOL_MANAGE_CRON]: "ManageCron",
  [TOOL_BROWSER]: "Browser",
  [TOOL_SEND_NOTIFICATION]: "SendNotification",
  [TOOL_SUBAGENT]: "Subagent",
  [TOOL_TASK_CREATE]: "TaskCreate",
  [TOOL_TASK_UPDATE]: "TaskUpdate",
  [TOOL_TASK_LIST]: "TaskList",
  [TOOL_MCP_RESOURCE]: "McpResource",
  [TOOL_MCP_PROMPT]: "McpPrompt",
  [TOOL_IMAGE_GENERATE]: "ImageGenerate",
  [TOOL_CANVAS]: "Canvas",
  [TOOL_ACP_SPAWN]: "AcpSpawn",
  // Tier 1 Core::FileSystem (project file)
  project_read_file: "ProjectReadFile",
  // Tier 1 Core::Interaction
  ask_user_question: "AskUserQuestion",
  send_attachment: "SendAttachment",
  // Tier 1 Core::SessionAware
  agents_list: "AgentsList",
  sessions_list: "SessionsList",
  session_status: "SessionStatus",
  sessions_history: "SessionsHistory",
  sessions_send: "SessionsSend",
  peek_sessions: "PeekSessions",
  // Tier 2 Standard
  team: "Team",
  pdf: "Pdf",
  image: "Image",
  get_weather: "GetWeather",
  get_settings: "GetSettings",
  update_settings: "UpdateSettings",
  list_settings_backups: "ListSettingsBackups",
  restore_settings_backup: "RestoreSettingsBackup",
  // Memory (additional)
  memory_get: "MemoryGet",
  update_core_memory: "UpdateCoreMemory",
  // Knowledge base (note_* / knowledge_recall / session_to_note)
  note_create: "NoteCreate",
  note_read: "NoteRead",
  note_update: "NoteUpdate",
  note_patch: "NotePatch",
  note_append: "NoteAppend",
  note_delete: "NoteDelete",
  note_search: "NoteSearch",
  note_link: "NoteLink",
  note_backlinks: "NoteBacklinks",
  note_by_tag: "NoteByTag",
  note_tags: "NoteTags",
  note_rename: "NoteRename",
  note_move: "NoteMove",
  note_set_frontmatter: "NoteSetFrontmatter",
  note_assign_block: "NoteAssignBlock",
  note_broken_links: "NoteBrokenLinks",
  note_orphans: "NoteOrphans",
  note_graph: "NoteGraph",
  note_similar: "NoteSimilar",
  note_related: "NoteRelated",
  note_suggest_links: "NoteSuggestLinks",
  note_distill: "NoteDistill",
  note_moc: "NoteMoc",
  session_to_note: "SessionToNote",
  knowledge_recall: "KnowledgeRecall",
}

type ToolDisplayFallback = {
  zh: string
  en: string
  zhDesc: string
  enDesc: string
}

const FEISHU_TOOL_DISPLAY: Record<string, ToolDisplayFallback> = {
  feishu_docx_create: {
    zh: "飞书云文档：新建文档",
    en: "Feishu Docs: Create Document",
    zhDesc: "创建一篇新的飞书云文档，返回 document_id。",
    enDesc: "Create a new Feishu/Lark document and return its document_id.",
  },
  feishu_docx_get_blocks: {
    zh: "飞书云文档：读取内容块",
    en: "Feishu Docs: Read Blocks",
    zhDesc: "分页读取飞书云文档的块结构和正文内容。",
    enDesc: "Read a Feishu/Lark document's block structure and content by page.",
  },
  feishu_docx_append_block: {
    zh: "飞书云文档：追加内容块",
    en: "Feishu Docs: Append Block",
    zhDesc: "在指定父块下追加段落、标题或列表等内容块。",
    enDesc: "Append a paragraph, heading, list item, or other block under a parent block.",
  },
  feishu_docx_update_block_text: {
    zh: "飞书云文档：更新文本块",
    en: "Feishu Docs: Update Text Block",
    zhDesc: "覆盖更新飞书云文档中某个文本类块的内容。",
    enDesc: "Overwrite the content of a text-bearing document block.",
  },
  feishu_bitable_list_records: {
    zh: "飞书多维表格：列出记录",
    en: "Feishu Bitable: List Records",
    zhDesc: "按表格、视图或简单过滤条件分页读取记录。",
    enDesc: "List records by table, view, or a simple filter expression.",
  },
  feishu_bitable_search_records: {
    zh: "飞书多维表格：搜索记录",
    en: "Feishu Bitable: Search Records",
    zhDesc: "用字段投影、复合过滤和排序进行结构化查询。",
    enDesc: "Run structured record queries with field projection, compound filters, and sorting.",
  },
  feishu_bitable_create_record: {
    zh: "飞书多维表格：新增记录",
    en: "Feishu Bitable: Create Record",
    zhDesc: "向指定数据表新增一条记录。",
    enDesc: "Create a single record in a target bitable table.",
  },
  feishu_bitable_batch_update_records: {
    zh: "飞书多维表格：批量更新记录",
    en: "Feishu Bitable: Batch Update Records",
    zhDesc: "批量更新同一数据表中的多条记录。",
    enDesc: "Batch update multiple records in one bitable table.",
  },
  feishu_bitable_list_views: {
    zh: "飞书多维表格：列出视图",
    en: "Feishu Bitable: List Views",
    zhDesc: "列出指定数据表下的所有视图。",
    enDesc: "List all views under a target bitable table.",
  },
  feishu_bitable_get_view: {
    zh: "飞书多维表格：读取视图",
    en: "Feishu Bitable: Get View",
    zhDesc: "读取某个多维表格视图的元信息。",
    enDesc: "Read metadata for a specific bitable view.",
  },
  feishu_bitable_list_dashboards: {
    zh: "飞书多维表格：列出仪表盘",
    en: "Feishu Bitable: List Dashboards",
    zhDesc: "列出多维表格应用中的仪表盘。",
    enDesc: "List dashboards in a bitable app.",
  },
  feishu_drive_list_files: {
    zh: "飞书云盘：列出文件",
    en: "Feishu Drive: List Files",
    zhDesc: "分页列出云盘文件夹内容。",
    enDesc: "List files in a Feishu/Lark Drive folder.",
  },
  feishu_drive_upload_media: {
    zh: "飞书云盘：上传文件",
    en: "Feishu Drive: Upload File",
    zhDesc: "把本地文件上传到飞书云盘或作为文档媒体资源。",
    enDesc: "Upload a local file to Drive or as media for a Feishu artifact.",
  },
  feishu_drive_download_media: {
    zh: "飞书云盘：下载文件",
    en: "Feishu Drive: Download File",
    zhDesc: "按 file_token 下载飞书文件到本地路径。",
    enDesc: "Download a Feishu file by file_token to a local path.",
  },
  feishu_wiki_get_node: {
    zh: "飞书知识库：解析节点",
    en: "Feishu Wiki: Resolve Node",
    zhDesc: "用 wiki token 解析知识库节点及其背后的文档对象。",
    enDesc: "Resolve a wiki token to its node metadata and backing object.",
  },
  feishu_approval_create_instance: {
    zh: "飞书审批：发起审批",
    en: "Feishu Approval: Create Instance",
    zhDesc: "按审批模板发起新的审批实例，高风险操作。",
    enDesc: "Create a new approval instance from an approval template. High risk.",
  },
  feishu_approval_get_instance: {
    zh: "飞书审批：查询审批",
    en: "Feishu Approval: Get Instance",
    zhDesc: "读取审批实例详情、状态和时间线。",
    enDesc: "Read approval instance details, status, and timeline.",
  },
  feishu_approval_cancel_instance: {
    zh: "飞书审批：撤销审批",
    en: "Feishu Approval: Cancel Instance",
    zhDesc: "撤销已发起的审批实例，高风险操作。",
    enDesc: "Cancel an existing approval instance. High risk.",
  },
  feishu_approval_list_instances: {
    zh: "飞书审批：列出审批",
    en: "Feishu Approval: List Instances",
    zhDesc: "按时间范围分页列出审批实例编号。",
    enDesc: "List approval instance codes in a time range.",
  },
  feishu_approval_subscribe: {
    zh: "飞书审批：订阅事件",
    en: "Feishu Approval: Subscribe",
    zhDesc: "为审批模板订阅事件推送。",
    enDesc: "Subscribe to approval events for a template.",
  },
  feishu_calendar_list: {
    zh: "飞书日历：列出日历",
    en: "Feishu Calendar: List Calendars",
    zhDesc: "列出当前账号可访问的飞书日历。",
    enDesc: "List Feishu/Lark calendars visible to the account.",
  },
  feishu_calendar_create_event: {
    zh: "飞书日历：创建日程",
    en: "Feishu Calendar: Create Event",
    zhDesc: "在指定日历中创建新日程。",
    enDesc: "Create a new event in a target calendar.",
  },
  feishu_calendar_list_events: {
    zh: "飞书日历：列出日程",
    en: "Feishu Calendar: List Events",
    zhDesc: "按时间范围分页读取日历日程。",
    enDesc: "List calendar events in a time range.",
  },
  feishu_calendar_update_event: {
    zh: "飞书日历：更新日程",
    en: "Feishu Calendar: Update Event",
    zhDesc: "更新已有日程的标题、时间、地点或说明等字段。",
    enDesc: "Update an event's title, time, location, description, or other fields.",
  },
  feishu_calendar_delete_event: {
    zh: "飞书日历：删除日程",
    en: "Feishu Calendar: Delete Event",
    zhDesc: "删除指定飞书日程。",
    enDesc: "Delete a specific Feishu/Lark calendar event.",
  },
  feishu_calendar_attendees_create: {
    zh: "飞书日历：添加参与人",
    en: "Feishu Calendar: Add Attendees",
    zhDesc: "给已有日程添加用户、群或邮箱参与人。",
    enDesc: "Add user, chat, or email attendees to an existing event.",
  },
  feishu_contact_get_user: {
    zh: "飞书通讯录：查询用户",
    en: "Feishu Contact: Get User",
    zhDesc: "按用户 ID 查询员工资料，可能包含敏感信息。",
    enDesc: "Look up a user by ID. May return sensitive profile data.",
  },
  feishu_contact_batch_get_users: {
    zh: "飞书通讯录：批量查询用户",
    en: "Feishu Contact: Batch Get Users",
    zhDesc: "批量查询多个员工资料，可能包含敏感信息。",
    enDesc: "Look up multiple users by ID. May return sensitive profile data.",
  },
  feishu_contact_get_department: {
    zh: "飞书通讯录：查询部门",
    en: "Feishu Contact: Get Department",
    zhDesc: "按部门 ID 查询部门信息。",
    enDesc: "Read department metadata by department ID.",
  },
  feishu_contact_search_users_by_department: {
    zh: "飞书通讯录：按部门找人",
    en: "Feishu Contact: Search Users by Department",
    zhDesc: "列出指定部门下的用户，可能包含敏感信息。",
    enDesc: "List users under a department. May return sensitive profile data.",
  },
  feishu_hire_list_jobs: {
    zh: "飞书招聘：列出职位",
    en: "Feishu Hire: List Jobs",
    zhDesc: "分页列出招聘职位。",
    enDesc: "List hiring jobs by page.",
  },
  feishu_hire_get_job: {
    zh: "飞书招聘：查询职位",
    en: "Feishu Hire: Get Job",
    zhDesc: "读取指定招聘职位详情。",
    enDesc: "Read details for a specific hiring job.",
  },
  feishu_hire_list_talents: {
    zh: "飞书招聘：列出人才",
    en: "Feishu Hire: List Talents",
    zhDesc: "分页列出候选人才，包含敏感个人信息。",
    enDesc: "List candidate talents by page. Contains sensitive personal data.",
  },
  feishu_hire_get_talent: {
    zh: "飞书招聘：查询人才",
    en: "Feishu Hire: Get Talent",
    zhDesc: "读取候选人才详情，包含敏感个人信息。",
    enDesc: "Read candidate talent details. Contains sensitive personal data.",
  },
  feishu_hire_list_applications: {
    zh: "飞书招聘：列出投递",
    en: "Feishu Hire: List Applications",
    zhDesc: "分页列出候选人的职位投递记录。",
    enDesc: "List candidate applications by page.",
  },
}

const isChineseLocale = (locale?: string) =>
  !!locale && (locale.startsWith("zh") || locale.startsWith("cn"))

const humanizeToolName = (name: string) =>
  name
    .split("_")
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ")

export const toolDisplayNameFallback = (name: string, locale?: string) => {
  const feishu = FEISHU_TOOL_DISPLAY[name]
  if (feishu) return isChineseLocale(locale) ? feishu.zh : feishu.en
  return humanizeToolName(name)
}

export const toolDisplayDescFallback = (
  name: string,
  backendDescription = "",
  locale?: string,
) => {
  const feishu = FEISHU_TOOL_DISPLAY[name]
  if (feishu) return isChineseLocale(locale) ? feishu.zhDesc : feishu.enDesc
  return backendDescription
}
