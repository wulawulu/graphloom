import { cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it } from "vitest"

import { LanguageSelector } from "@/components/layout/language-selector"
import { en } from "@/i18n/en"
import {
  activeStudioLocale,
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
  it("provides Simplified Chinese for every non-plural English UI key", () => {
    const missing = Object.keys(en).filter((key) => !key.endsWith("_other") && !(key in zhCN))
    expect(missing).toEqual([])
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
  ] as const)("resolves stored %s and browser %s to %s", (stored, languages, expected) => {
    expect(resolveStudioLocale(stored, languages)).toBe(expected)
  })

  it("uses a persisted locale during reload-style resolution", () => {
    localStorage.setItem(STUDIO_LOCALE_STORAGE_KEY, "zh-CN")
    expect(resolveStudioLocale(localStorage.getItem(STUDIO_LOCALE_STORAGE_KEY), ["en-US"])).toBe("zh-CN")
  })
})

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
