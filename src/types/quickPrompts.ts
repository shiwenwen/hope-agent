export interface QuickPromptItem {
  id: string
  title: string
  content: string
  createdAt: string
}

export interface QuickPromptConfig {
  items: QuickPromptItem[]
}

export interface QuickPromptAddResult {
  item: QuickPromptItem
  duplicate: boolean
}
