import { useEffect, useRef, useState, useEffectEvent } from "react"
import type { PlanModeState } from "./usePlanMode"

export interface CommentPopoverState {
  position: { top: number; left: number }
  selectedText: string
}

export function usePlanComment({
  planState,
  onRequestChanges,
}: {
  planState: PlanModeState
  onRequestChanges?: (feedback: string) => void
}) {
  const [commentPopover, setCommentPopover] = useState<CommentPopoverState | null>(null)
  const contentRef = useRef<HTMLDivElement>(null)

  // Whether inline commenting is enabled
  const canComment = (planState === "review" || planState === "planning") && !!onRequestChanges

  // Highlight selected text with <mark> wrapper
  const highlightSelection = (range: Range) => {
    try {
      const mark = document.createElement("mark")
      mark.className = "bg-blue-200/50 dark:bg-blue-500/30 rounded-sm plan-comment-highlight"
      range.surroundContents(mark)
    } catch {
      // surroundContents fails for cross-element selections — wrap individual text nodes
      const treeWalker = document.createTreeWalker(
        range.commonAncestorContainer,
        NodeFilter.SHOW_TEXT,
      )
      const textNodes: Text[] = []
      while (treeWalker.nextNode()) {
        const node = treeWalker.currentNode as Text
        if (range.intersectsNode(node)) textNodes.push(node)
      }
      for (const node of textNodes) {
        const mark = document.createElement("mark")
        mark.className = "bg-blue-200/50 dark:bg-blue-500/30 rounded-sm plan-comment-highlight"
        node.parentNode?.insertBefore(mark, node)
        mark.appendChild(node)
      }
    }
  }

  // Remove all highlight <mark> wrappers, restoring original DOM
  const clearHighlight = () => {
    if (!contentRef.current) return
    const marks = contentRef.current.querySelectorAll("mark.plan-comment-highlight")
    marks.forEach((mark) => {
      const parent = mark.parentNode
      if (parent) {
        while (mark.firstChild) parent.insertBefore(mark.firstChild, mark)
        parent.removeChild(mark)
      }
    })
  }
  const clearHighlightEffectEvent = useEffectEvent(clearHighlight)

  // Handle text selection for inline commenting
  const handleMouseUp = () => {
    if (!contentRef.current) return
    // Only allow commenting in review/planning states
    if (planState !== "review" && planState !== "planning") return

    const selection = window.getSelection()
    if (!selection || selection.isCollapsed || !selection.toString().trim()) {
      return
    }

    const selectedText = selection.toString().trim()
    if (!selectedText) return

    // Check if selection is within the content area
    const range = selection.getRangeAt(0)
    if (!contentRef.current.contains(range.commonAncestorContainer)) return

    // Calculate position relative to the content container
    const rect = range.getBoundingClientRect()
    const containerRect = contentRef.current.getBoundingClientRect()

    // Position the popover below the selection, clamped within container bounds
    const top = rect.bottom - containerRect.top + contentRef.current.scrollTop + 4
    let left = rect.left - containerRect.left
    // Clamp to prevent overflow (popover is 280px wide)
    left = Math.max(0, Math.min(left, contentRef.current.clientWidth - 280))

    // Clear any previous highlight, then apply new one
    clearHighlight()
    highlightSelection(range.cloneRange())
    selection.removeAllRanges()

    setCommentPopover({ position: { top, left }, selectedText })
  }

  // Close comment popover when clicking outside or selection changes
  useEffect(() => {
    const handleMouseDown = () => {
      // Don't close if clicking inside the popover (handled by stopPropagation there)
      if (commentPopover) {
        clearHighlightEffectEvent()
        setCommentPopover(null)
      }
    }
    // Use mousedown on document to dismiss
    document.addEventListener("mousedown", handleMouseDown)
    return () => document.removeEventListener("mousedown", handleMouseDown)
  }, [commentPopover])

  // Cleanup highlights when commenting is disabled
  useEffect(() => {
    const canCommentNow = (planState === "review" || planState === "planning") && !!onRequestChanges
    if (!canCommentNow) clearHighlightEffectEvent()
  }, [planState, onRequestChanges])

  // Submit comment: format as quoted selection + comment and send to model
  const handleCommentSubmit = (comment: string) => {
    if (!commentPopover || !onRequestChanges) return
    const feedback = [
      `<plan-inline-comment>`,
      `The user selected the following section from the current plan and requests a revision:`,
      ``,
      `<selected-text>`,
      commentPopover.selectedText,
      `</selected-text>`,
      ``,
      `<revision-request>`,
      comment,
      `</revision-request>`,
      ``,
      `Please revise the plan to address this feedback. Modify the quoted section while keeping the rest of the plan intact, then resubmit the updated plan using the submit_plan tool.`,
      `</plan-inline-comment>`,
    ].join("\n")
    onRequestChanges(feedback)
    clearHighlight()
    setCommentPopover(null)
    window.getSelection()?.removeAllRanges()
  }

  const closeCommentPopover = () => {
    clearHighlight()
    setCommentPopover(null)
    window.getSelection()?.removeAllRanges()
  }

  return {
    commentPopover,
    contentRef,
    canComment,
    handleMouseUp,
    handleCommentSubmit,
    closeCommentPopover,
  }
}
