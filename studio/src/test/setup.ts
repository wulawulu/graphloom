import "@testing-library/jest-dom/vitest"
import { afterEach, beforeEach } from "vitest"

import { setStudioLocale } from "@/i18n"

class ResizeObserverStub implements ResizeObserver {
  disconnect(): void {}
  observe(): void {}
  unobserve(): void {}
}

globalThis.ResizeObserver = ResizeObserverStub
HTMLElement.prototype.hasPointerCapture = () => false
HTMLElement.prototype.setPointerCapture = () => undefined
HTMLElement.prototype.releasePointerCapture = () => undefined
HTMLElement.prototype.scrollIntoView = () => undefined
HTMLElement.prototype.scrollTo = () => undefined

beforeEach(async () => setStudioLocale("en", false))
afterEach(async () => setStudioLocale("en", false))
