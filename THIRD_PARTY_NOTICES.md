# Third-Party Notices

Hope Agent ships with third-party open-source components. This file documents those bundled components and their original licenses.

---

## Bundled Icons (`vscode-icons`)

The colorful, format-specific file icons rendered by [`FileTypeIcon`](./src/components/icons/FileTypeIcon.tsx) (workspace panel, message attachments, project file browser) are from the **VSCode Icons** project, consumed via the `@iconify-json/vscode-icons` package and inlined at build time by `unplugin-icons` (only the icons actually imported are bundled).

Source: <https://github.com/vscode-icons/vscode-icons> · Author: Roberto Huertas

```
MIT License

Copyright (c) 2016 Roberto Huertas

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
