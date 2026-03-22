import { useState, useEffect, useRef } from "react";
import { Streamdown } from "streamdown";
import { code } from "@streamdown/code";
import { cjk } from "@streamdown/cjk";
import { math } from "@streamdown/math";
import { mermaid } from "@streamdown/mermaid";
import "katex/dist/katex.min.css";
import "streamdown/styles.css";

const plugins = { code, cjk, math, mermaid };

/**
 * 打字机速度调参：
 * - BASE_CHARS_PER_FRAME: 积压较小时每帧显示的字符数
 * - CATCHUP_THRESHOLD: 积压超过此值时自动加速追赶
 */
const BASE_CHARS_PER_FRAME = 1;
const CATCHUP_THRESHOLD = 80;

interface MarkdownRendererProps {
  content: string;
  isStreaming?: boolean;
}

export default function MarkdownRenderer({
  content,
  isStreaming = false,
}: MarkdownRendererProps) {
  // 当前显示的字符数（打字机光标位置）
  const [displayLen, setDisplayLen] = useState(isStreaming ? 0 : content.length);
  const targetLenRef = useRef(content.length);
  const rafRef = useRef<number | null>(null);

  // 始终跟踪完整内容长度
  targetLenRef.current = content.length;

  // 非流式状态下立即显示全部内容
  useEffect(() => {
    if (!isStreaming) {
      setDisplayLen(content.length);
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    }
  }, [isStreaming, content.length]);

  // 流式输出时的打字机 rAF 循环
  useEffect(() => {
    if (!isStreaming) return;

    const tick = () => {
      setDisplayLen((prev) => {
        const target = targetLenRef.current;
        if (prev >= target) return prev;
        const backlog = target - prev;
        // 自适应步长：积压过多时加速追赶
        const step =
          backlog > CATCHUP_THRESHOLD
            ? Math.max(BASE_CHARS_PER_FRAME, Math.ceil(backlog / 10))
            : BASE_CHARS_PER_FRAME;
        return Math.min(prev + step, target);
      });
      rafRef.current = requestAnimationFrame(tick);
    };

    rafRef.current = requestAnimationFrame(tick);
    return () => {
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };
  }, [isStreaming]);

  if (!content) return null;

  const displayContent = isStreaming ? content.slice(0, displayLen) : content;

  return (
    <Streamdown animated plugins={plugins} isAnimating={isStreaming}>
      {displayContent}
    </Streamdown>
  );
}
