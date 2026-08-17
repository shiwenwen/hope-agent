# UTF-8 连续分页读取评测

只使用 `read` 工具读取工作目录中的 `fixtures/utf8-pages.txt`，禁止使用 `exec`、`grep`、`find` 或其它工具绕过分页。

第一次调用只传 `path`。如果结果给出 `offset` 或 `byte_offset` 续读游标，下一次调用必须使用同一路径和结果中给出的精确游标，不传 `limit`，持续读取，直到结果不再包含续读提示。文件混合中文、emoji 和组合字符，并包含一条超过单页上限的长行；不得按字符数猜游标。

完成后只输出合法 JSON，不要 Markdown 代码块或解释：

```json
{
  "markers": ["按在文件中出现的顺序列出全部 CTX 标记"],
  "eof": true
}
```
