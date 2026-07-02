# Knowledge Agent Access

> 返回 [文档索引](../README.md) | 关联：[知识空间架构](../architecture/knowledge-base.md) · [CLI](../architecture/cli.md) · [API 参考](../architecture/api-reference.md)

Phase 7 把知识空间暴露给外部 agent，但默认保持只读。推荐优先用 MCP；HTTP 适合脚本、CI 或已经有 Hope Agent server 的远程部署。

## MCP stdio

默认只读：

```json
{
  "mcpServers": {
    "hope-agent-knowledge": {
      "command": "hope-agent",
      "args": ["knowledge-mcp"]
    }
  }
}
```

允许创建 Review Diff proposal：

```json
{
  "mcpServers": {
    "hope-agent-knowledge": {
      "command": "hope-agent",
      "args": ["knowledge-mcp", "--allow-proposals"]
    }
  }
}
```

默认工具：

- `knowledge_search`
- `knowledge_read`
- `knowledge_expand`
- `knowledge_sources`

`--allow-proposals` 额外暴露 `knowledge_compile_propose`。它只创建 Compile Review proposals，不会直接写入 `.md`。

## HTTP

Owner token 使用 `server.apiKey`，拥有完整管理权限。只读 token 使用 `server.knowledgeAgentReadToken` 或环境变量 `HA_KNOWLEDGE_AGENT_READ_TOKEN`，只允许访问：

- `POST /api/knowledge/agent/search`
- `POST /api/knowledge/agent/read`
- `POST /api/knowledge/agent/expand`
- `POST /api/knowledge/agent/sources`

示例：

```bash
curl -sS \
  -H "Authorization: Bearer $HA_KNOWLEDGE_AGENT_READ_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"query":"project roadmap","kbId":"<kb-id>","limit":5}' \
  http://127.0.0.1:8420/api/knowledge/agent/search
```

所有 HTTP Agent endpoints 都接受裸 input 或 `{ "input": ... }` wrapper：

```bash
curl -sS \
  -H "Authorization: Bearer $HA_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"input":{"kbId":"<kb-id>","path":"Inbox/Example.md"}}' \
  http://127.0.0.1:8420/api/knowledge/agent/read
```

Raw sources 默认不会混进 note 搜索。只有 `includeSources=true` 或调用 `knowledge_sources` / `/sources` endpoint 时才会返回 `kind:"source"`。
