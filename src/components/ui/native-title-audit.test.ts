import { readdirSync, readFileSync } from "node:fs"
import { join, relative, resolve } from "node:path"
import * as ts from "typescript"
import { describe, expect, it } from "vitest"

const SRC_ROOT = resolve(process.cwd(), "src")
const DOM_FORWARDING_COMPONENTS = new Set(["Button", "SelectTrigger"])
const INTERACTIVE_TOOLTIP_TARGETS = new Set(["button", "Button", "SelectTrigger"])

function tsxFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) return tsxFiles(path)
    if (!entry.name.endsWith(".tsx") || /\.(?:test|spec)\.tsx$/.test(entry.name)) return []
    return [path]
  })
}

function expressionMayRenderText(expression: ts.Expression | undefined): boolean {
  if (!expression) return false
  if (
    ts.isStringLiteralLike(expression) ||
    ts.isNumericLiteral(expression) ||
    ts.isIdentifier(expression) ||
    ts.isPropertyAccessExpression(expression) ||
    ts.isElementAccessExpression(expression) ||
    ts.isCallExpression(expression) ||
    ts.isTemplateExpression(expression) ||
    ts.isNoSubstitutionTemplateLiteral(expression)
  ) {
    return true
  }
  if (ts.isParenthesizedExpression(expression)) {
    return expressionMayRenderText(expression.expression)
  }
  if (ts.isConditionalExpression(expression)) {
    return (
      expressionMayRenderText(expression.whenTrue) ||
      expressionMayRenderText(expression.whenFalse)
    )
  }
  if (ts.isBinaryExpression(expression)) {
    return expression.operatorToken.kind === ts.SyntaxKind.AmpersandAmpersandToken
      ? expressionMayRenderText(expression.right)
      : true
  }
  if (ts.isJsxElement(expression)) return jsxChildrenMayRenderText(expression.children)
  return false
}

function jsxChildrenMayRenderText(children: readonly ts.JsxChild[]): boolean {
  return children.some((child) => {
    if (ts.isJsxText(child)) return child.getText().trim().length > 0
    if (ts.isJsxExpression(child)) return expressionMayRenderText(child.expression)
    if (ts.isJsxElement(child)) return jsxChildrenMayRenderText(child.children)
    return false
  })
}

describe("native title audit", () => {
  it("keeps native title off tooltip-bearing production elements", () => {
    const violations: string[] = []
    const missingAccessibleLabels: string[] = []

    for (const file of tsxFiles(SRC_ROOT)) {
      const source = readFileSync(file, "utf8")
      const sourceFile = ts.createSourceFile(
        file,
        source,
        ts.ScriptTarget.Latest,
        true,
        ts.ScriptKind.TSX,
      )
      const visit = (node: ts.Node) => {
        const opening = ts.isJsxElement(node)
          ? node.openingElement
          : ts.isJsxSelfClosingElement(node)
            ? node
            : null
        if (opening) {
          const tag = opening.tagName.getText(sourceFile)
          const isNativeTooltipTarget =
            (/^[a-z]/.test(tag) && tag !== "iframe") || DOM_FORWARDING_COMPONENTS.has(tag)
          const hasTitle = opening.attributes.properties.some(
            (attribute) =>
              ts.isJsxAttribute(attribute) && attribute.name.getText(sourceFile) === "title",
          )
          const hasDelegatedTooltip = opening.attributes.properties.some(
            (attribute) =>
              ts.isJsxAttribute(attribute) &&
              attribute.name.getText(sourceFile) === "data-ha-title-tip",
          )
          const hasAriaLabel = opening.attributes.properties.some(
            (attribute) =>
              ts.isJsxAttribute(attribute) && attribute.name.getText(sourceFile) === "aria-label",
          )
          const location = () => {
            const { line } = sourceFile.getLineAndCharacterOfPosition(opening.getStart(sourceFile))
            return `${relative(process.cwd(), file)}:${line + 1} <${tag}>`
          }
          if (isNativeTooltipTarget && hasTitle) {
            violations.push(location())
          }
          if (
            INTERACTIVE_TOOLTIP_TARGETS.has(tag) &&
            hasDelegatedTooltip &&
            !hasAriaLabel &&
            tag !== "SelectTrigger" &&
            (!ts.isJsxElement(node) || !jsxChildrenMayRenderText(node.children))
          ) {
            missingAccessibleLabels.push(location())
          }
        }
        ts.forEachChild(node, visit)
      }
      visit(sourceFile)
    }

    expect(violations).toEqual([])
    expect(missingAccessibleLabels).toEqual([])
  })
})
