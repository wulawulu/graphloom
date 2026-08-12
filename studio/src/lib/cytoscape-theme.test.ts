import { describe, expect, it } from "vitest"

import { readCytoscapeTheme } from "@/lib/cytoscape-theme"

const safeTheme = {
  "--cy-background": "#080a0e",
  "--cy-foreground": "#ebeff5",
  "--cy-border": "#282b33",
  "--cy-graph-person": "#00bdce",
  "--cy-graph-organization": "#bc90ee",
  "--cy-graph-geo": "#5bbd74",
  "--cy-graph-event": "#eba941",
  "--cy-graph-default": "#7d8a9f",
  "--cy-graph-seed": "#f9bd01",
}

function styles(overrides: Record<string, string> = {}): CSSStyleDeclaration {
  const element = document.createElement("div")
  for (const [name, value] of Object.entries({ ...safeTheme, ...overrides })) {
    element.style.setProperty(name, value)
  }
  return element.style
}

describe("Cytoscape renderer theme", () => {
  it("reads only explicit renderer-safe hex colors", () => {
    expect(Object.values(readCytoscapeTheme(styles()))).toEqual(Object.values(safeTheme))
    expect(Object.values(readCytoscapeTheme(styles())).some((value) => value.includes("oklch"))).toBe(false)
  })

  it.each(["", "oklch(0.72 0.14 205)", "lab(50% 0 0)", "lch(50% 20 30)"])(
    "rejects unsupported renderer color %j",
    (value) => {
      expect(() => readCytoscapeTheme(styles({ "--cy-graph-person": value }))).toThrow(
        "Invalid Cytoscape renderer theme token: --cy-graph-person",
      )
    },
  )
})
