import { cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it } from "vitest"

import { LanguageSelector } from "@/components/layout/language-selector"
import { en } from "@/i18n/en"
import {
  activeStudioLocale,
  i18n,
  resolveStudioLocale,
  setStudioLocale,
  STUDIO_LOCALE_STORAGE_KEY,
} from "@/i18n"
import { zhCN } from "@/i18n/zh-CN"

afterEach(() => {
  cleanup()
  localStorage.clear()
})

describe("Studio locale resolution", () => {
  it("keeps identical nested semantic key trees for English and Simplified Chinese", () => {
    expect(flattenKeys(zhCN)).toEqual(flattenKeys(en))
  })

  it("uses semantic namespaces and resolves nested keys through dot separators", async () => {
    expect(Object.keys(en).sort()).toEqual(["answer", "common", "errors", "explainability", "graph", "languages", "navigation", "query", "runs", "settings"])
    expect(Object.keys(en)).not.toContain("Copy")
    await setStudioLocale("en", false)
    expect(i18n.t("common.copy")).toBe("Copy")
    await setStudioLocale("zh-CN", false)
    expect(i18n.t("common.copy")).toBe("复制")
  })

  it.each([
    [null, ["en-US"], "en"],
    [null, ["zh-CN"], "zh-CN"],
    ["en", ["zh-CN"], "en"],
    ["zh-CN", ["en-US"], "zh-CN"],
    [null, ["fr-FR"], "en"],
    [null, ["zh-Hans-CN"], "zh-CN"],
    [null, ["zh-TW"], "en"],
    [null, ["zh-Hant"], "en"],
    [null, ["zh-TW", "zh", "en-US"], "en"],
    [null, ["zh-Hant-HK", "zh-CN"], "en"],
    ["zh-HK", ["zh-CN"], "en"],
  ] as const)("resolves stored %s and browser %s to %s", (stored, languages, expected) => {
    expect(resolveStudioLocale(stored, languages)).toBe(expected)
  })

  it("uses a persisted locale during reload-style resolution", () => {
    localStorage.setItem(STUDIO_LOCALE_STORAGE_KEY, "zh-CN")
    expect(resolveStudioLocale(localStorage.getItem(STUDIO_LOCALE_STORAGE_KEY), ["en-US"])).toBe("zh-CN")
  })
})

function flattenKeys(value: object, prefix = ""): string[] {
  return Object.entries(value).flatMap(([key, child]) => {
    const path = prefix.length === 0 ? key : `${prefix}.${key}`
    return typeof child === "string" ? [path] : flattenKeys(child as object, path)
  }).sort()
}

describe("LanguageSelector", () => {
  it("switches immediately, persists the choice, and updates html lang", async () => {
    const user = userEvent.setup()
    await setStudioLocale("en", false)
    render(<LanguageSelector />)

    await user.click(screen.getByRole("combobox", { name: "Language" }))
    await user.click(screen.getByRole("option", { name: "简体中文" }))

    await waitFor(() => expect(activeStudioLocale()).toBe("zh-CN"))
    expect(screen.getByRole("combobox", { name: "语言" })).toHaveTextContent("简体中文")
    expect(localStorage.getItem(STUDIO_LOCALE_STORAGE_KEY)).toBe("zh-CN")
    expect(document.documentElement.lang).toBe("zh-CN")
  })
})
