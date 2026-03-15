import { Streamdown } from "streamdown";
import { code } from "@streamdown/code";
import { cjk } from "@streamdown/cjk";
import { math } from "@streamdown/math";
import { mermaid } from "@streamdown/mermaid";
import "katex/dist/katex.min.css";
import "streamdown/styles.css";

const plugins = { code, cjk, math, mermaid };

interface MarkdownRendererProps {
  content: string;
  isStreaming?: boolean;
}

export default function MarkdownRenderer({
  content,
  isStreaming = false,
}: MarkdownRendererProps) {
  if (!content) return null;

  return (
    <Streamdown animated plugins={plugins} isAnimating={isStreaming}>
      {content}
    </Streamdown>
  );
}
