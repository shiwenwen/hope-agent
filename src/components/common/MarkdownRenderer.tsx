import { useState, useEffect, useRef } from "react";
import { Streamdown, type AnimateOptions } from "streamdown";
import { code } from "@streamdown/code";
import { cjk } from "@streamdown/cjk";
import { math } from "@streamdown/math";
import { mermaid } from "@streamdown/mermaid";
import "katex/dist/katex.min.css";
import "streamdown/styles.css";

const plugins = { code, cjk, math, mermaid };

/** Word-level blurIn: each completed word gets a blur-to-clear entrance */
const streamingAnimation: AnimateOptions = {
  animation: "blurIn",
  sep: "word",
  duration: 500,
  easing: "cubic-bezier(0.22, 1, 0.36, 1)",
};

/** Start catching up when backlog exceeds this */
const CATCHUP_THRESHOLD = 60;
/** Max chars per frame when catching up, prevents jarring jumps */
const MAX_STEP = 8;

interface MarkdownRendererProps {
  content: string;
  isStreaming?: boolean;
}

export default function MarkdownRenderer({
  content,
  isStreaming = false,
}: MarkdownRendererProps) {
  const [displayLen, setDisplayLen] = useState(() =>
    isStreaming ? 0 : content.length,
  );

  const cursorRef = useRef(isStreaming ? 0 : content.length);
  const targetRef = useRef(content.length);
  const streamingRef = useRef(isStreaming);
  const rafRef = useRef<number | null>(null);

  // Height animation refs
  const containerRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);

  // eslint-disable-next-line react-hooks/refs -- intentional "latest value" refs read only in rAF callback
  targetRef.current = content.length;
  // eslint-disable-next-line react-hooks/refs
  streamingRef.current = isStreaming;

  // Non-streaming (history): show full content immediately
  useEffect(() => {
    if (!isStreaming && rafRef.current === null) {
      cursorRef.current = content.length;
      setDisplayLen(content.length);
    }
  }, [isStreaming, content.length]);

  // rAF loop: +1 char per frame, continues draining after stream ends (no jump)
  useEffect(() => {
    if (!isStreaming) return;
    if (rafRef.current !== null) return;

    const tick = () => {
      const cursor = cursorRef.current;
      const target = targetRef.current;

      if (cursor >= target && !streamingRef.current) {
        rafRef.current = null;
        return;
      }

      if (cursor < target) {
        const backlog = target - cursor;
        const step =
          backlog > CATCHUP_THRESHOLD
            ? Math.min(Math.ceil(backlog * 0.1), MAX_STEP)
            : 1;
        const next = Math.min(cursor + step, target);
        cursorRef.current = next;
        setDisplayLen(next);
      }

      rafRef.current = requestAnimationFrame(tick);
    };

    rafRef.current = requestAnimationFrame(tick);
  }, [isStreaming]);

  // Smooth height transition: mount ResizeObserver once when streaming starts,
  // let it detect height changes on its own to avoid breaking CSS transitions
  useEffect(() => {
    const container = containerRef.current;
    const contentEl = contentRef.current;
    if (!container || !contentEl || !isStreaming) {
      if (containerRef.current) containerRef.current.style.height = "";
      return;
    }

    container.style.height = `${contentEl.offsetHeight}px`;

    const observer = new ResizeObserver(() => {
      const h = contentEl.offsetHeight;
      if (container.style.height !== `${h}px`) {
        container.style.height = `${h}px`;
      }
    });
    observer.observe(contentEl);

    return () => {
      observer.disconnect();
      container.style.height = "";
    };
  }, [isStreaming]);

  useEffect(() => {
    return () => {
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };
  }, []);

  if (!content) return null;

  const revealing = displayLen < content.length;
  const displayContent = revealing ? content.slice(0, displayLen) : content;
  const isActive = isStreaming || revealing;

  return (
    <div
      ref={containerRef}
      className={isActive ? "streaming-height" : undefined}
    >
      <div ref={contentRef}>
        <Streamdown
          animated={isActive ? streamingAnimation : true}
          plugins={plugins}
          isAnimating={isActive}
          parseIncompleteMarkdown={isActive}
        >
          {displayContent}
        </Streamdown>
      </div>
    </div>
  );
}
