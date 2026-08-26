import { StrictMode } from "react"
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { FOLLOW_THRESHOLD_PX, distanceFromBottom, isNearBottom, useAutoFollow } from "@/hooks/use-auto-follow"

interface HarnessProps {
  resetKey: string | null
}

class ControlledResizeObserver implements ResizeObserver {
  static instances: ControlledResizeObserver[] = []

  readonly observed: Element[] = []
  disconnected = false

  constructor(private readonly callback: ResizeObserverCallback) {
    ControlledResizeObserver.instances.push(this)
  }

  disconnect(): void {
    this.disconnected = true
  }

  observe(target: Element): void {
    this.observed.push(target)
  }

  unobserve(): void {}

  trigger(): void {
    this.callback([], this)
  }
}

let animationFrames = new Map<number, FrameRequestCallback>()
let nextAnimationFrame = 1
let originalScrollTo: PropertyDescriptor | undefined

function AutoFollowHarness({ resetKey }: HarnessProps): React.ReactElement {
  const { viewportRef, contentRef, following, pause, scrollToLatest } = useAutoFollow({ resetKey })
  return (
    <div>
      <div ref={viewportRef} data-testid="viewport">
        <div ref={contentRef} data-testid="content">content</div>
      </div>
      <output>{following ? "following" : "paused"}</output>
      <button type="button" onClick={pause}>Pause</button>
      <button type="button" onClick={scrollToLatest}>Back to latest</button>
    </div>
  )
}

function setViewportMetrics(viewport: HTMLElement, { clientHeight, scrollHeight, scrollTop }: {
  clientHeight: number
  scrollHeight: number
  scrollTop: number
}): void {
  Object.defineProperties(viewport, {
    clientHeight: { configurable: true, value: clientHeight },
    scrollHeight: { configurable: true, value: scrollHeight },
    scrollTop: { configurable: true, value: scrollTop, writable: true },
  })
}

function flushAnimationFrame(): void {
  const callbacks = [...animationFrames.values()]
  animationFrames.clear()
  act(() => callbacks.forEach((callback) => callback(0)))
}

function settleInitialFollow(): void {
  flushAnimationFrame()
  flushAnimationFrame()
  flushAnimationFrame()
}

function activeObserver(): ControlledResizeObserver {
  for (let index = ControlledResizeObserver.instances.length - 1; index >= 0; index -= 1) {
    const observer = ControlledResizeObserver.instances[index]
    if (observer !== undefined && !observer.disconnected) return observer
  }
  throw new Error("expected an active ResizeObserver")
}

beforeEach(() => {
  ControlledResizeObserver.instances = []
  animationFrames = new Map()
  nextAnimationFrame = 1
  vi.stubGlobal("ResizeObserver", ControlledResizeObserver)
  vi.stubGlobal("requestAnimationFrame", vi.fn((callback: FrameRequestCallback) => {
    const id = nextAnimationFrame
    nextAnimationFrame += 1
    animationFrames.set(id, callback)
    return id
  }))
  vi.stubGlobal("cancelAnimationFrame", vi.fn((id: number) => animationFrames.delete(id)))
  vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: false })))
  originalScrollTo = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollTo")
  Object.defineProperty(HTMLElement.prototype, "scrollTo", {
    configurable: true,
    value: vi.fn(function scrollTo(this: HTMLElement, options: ScrollToOptions): void {
      const requestedTop = typeof options === "object" ? options.top ?? 0 : 0
      this.scrollTop = Math.max(0, Math.min(requestedTop, this.scrollHeight - this.clientHeight))
      this.dispatchEvent(new Event("scroll"))
    }),
  })
})

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.unstubAllGlobals()
  if (originalScrollTo !== undefined) Object.defineProperty(HTMLElement.prototype, "scrollTo", originalScrollTo)
})

describe("auto-follow metrics", () => {
  it.each([
    [0, true],
    [50, true],
    [96, true],
    [97, false],
    [500, false],
  ])("classifies a %dpx bottom distance", (distance, expected) => {
    const viewport = { clientHeight: 200, scrollHeight: 1_000, scrollTop: 800 - distance }
    expect(distanceFromBottom(viewport)).toBe(distance)
    expect(isNearBottom(viewport, FOLLOW_THRESHOLD_PX)).toBe(expected)
  })
})

describe("useAutoFollow", () => {
  it("follows initial and streaming content growth once per animation frame", () => {
    render(<AutoFollowHarness resetKey="run-a" />)
    const viewport = screen.getByTestId("viewport")
    setViewportMetrics(viewport, { clientHeight: 200, scrollHeight: 600, scrollTop: 400 })
    settleInitialFollow()
    const scrollTo = vi.mocked(viewport.scrollTo)
    scrollTo.mockClear()

    setViewportMetrics(viewport, { clientHeight: 200, scrollHeight: 900, scrollTop: 400 })
    act(() => {
      activeObserver().trigger()
      activeObserver().trigger()
      activeObserver().trigger()
    })
    expect(scrollTo).not.toHaveBeenCalled()
    flushAnimationFrame()

    expect(scrollTo).toHaveBeenCalledTimes(1)
    expect(scrollTo).toHaveBeenCalledWith({ top: 900, behavior: "auto" })
    expect(screen.getByText("following")).toBeInTheDocument()
  })

  it("pauses on a user scroll away and ignores later content growth", () => {
    render(<AutoFollowHarness resetKey="run-a" />)
    const viewport = screen.getByTestId("viewport")
    setViewportMetrics(viewport, { clientHeight: 200, scrollHeight: 800, scrollTop: 600 })
    settleInitialFollow()
    const scrollTo = vi.mocked(viewport.scrollTo)
    scrollTo.mockClear()

    viewport.scrollTop = 100
    fireEvent.scroll(viewport)
    expect(screen.getByText("paused")).toBeInTheDocument()

    setViewportMetrics(viewport, { clientHeight: 200, scrollHeight: 1_000, scrollTop: 100 })
    act(() => activeObserver().trigger())
    flushAnimationFrame()
    expect(scrollTo).not.toHaveBeenCalled()
  })

  it("resumes when the user manually returns within the bottom threshold", () => {
    render(<AutoFollowHarness resetKey="run-a" />)
    const viewport = screen.getByTestId("viewport")
    setViewportMetrics(viewport, { clientHeight: 200, scrollHeight: 1_000, scrollTop: 100 })
    settleInitialFollow()
    viewport.scrollTop = 100
    fireEvent.scroll(viewport)
    expect(screen.getByText("paused")).toBeInTheDocument()

    viewport.scrollTop = 704
    fireEvent.scroll(viewport)
    expect(screen.getByText("following")).toBeInTheDocument()
  })

  it("uses smooth motion for an explicit return and pauses when that motion is interrupted", () => {
    render(<AutoFollowHarness resetKey="run-a" />)
    const viewport = screen.getByTestId("viewport")
    setViewportMetrics(viewport, { clientHeight: 200, scrollHeight: 1_000, scrollTop: 100 })
    settleInitialFollow()
    viewport.scrollTop = 100
    fireEvent.scroll(viewport)
    expect(screen.getByText("paused")).toBeInTheDocument()

    fireEvent.click(screen.getByRole("button", { name: "Back to latest" }))
    expect(viewport.scrollTo).toHaveBeenLastCalledWith({ top: 1_000, behavior: "smooth" })
    expect(screen.getByText("following")).toBeInTheDocument()

    viewport.scrollTop = 200
    fireEvent.scroll(viewport)
    expect(screen.getByText("following")).toBeInTheDocument()
    fireEvent(viewport, new Event("scrollend"))
    expect(screen.getByText("paused")).toBeInTheDocument()
  })

  it("lets a user scroll override continuous machine auto-scroll targets", () => {
    render(<AutoFollowHarness resetKey="run-a" />)
    const viewport = screen.getByTestId("viewport")
    setViewportMetrics(viewport, { clientHeight: 200, scrollHeight: 800, scrollTop: 600 })
    settleInitialFollow()
    const scrollTo = vi.mocked(viewport.scrollTo)
    scrollTo.mockClear()
    scrollTo.mockImplementationOnce(function pendingAutoScroll(this: HTMLElement): void {})

    setViewportMetrics(viewport, { clientHeight: 200, scrollHeight: 1_000, scrollTop: 600 })
    act(() => activeObserver().trigger())
    flushAnimationFrame()
    expect(scrollTo).toHaveBeenCalledOnce()

    viewport.scrollTop = 200
    fireEvent.scroll(viewport)
    expect(screen.getByText("paused")).toBeInTheDocument()

    scrollTo.mockClear()
    act(() => activeObserver().trigger())
    flushAnimationFrame()
    expect(scrollTo).not.toHaveBeenCalled()
  })

  it("keeps an explicit pause authoritative over an older smooth completion", () => {
    vi.useFakeTimers()
    render(<AutoFollowHarness resetKey="run-a" />)
    const viewport = screen.getByTestId("viewport")
    setViewportMetrics(viewport, { clientHeight: 200, scrollHeight: 1_000, scrollTop: 100 })
    settleInitialFollow()
    viewport.scrollTop = 100
    fireEvent.scroll(viewport)
    fireEvent.click(screen.getByRole("button", { name: "Back to latest" }))
    fireEvent.click(screen.getByRole("button", { name: "Pause" }))

    act(() => vi.advanceTimersByTime(2_000))
    fireEvent(viewport, new Event("scrollend"))
    expect(screen.getByText("paused")).toBeInTheDocument()
  })

  it("defers streaming catch-up until a smooth return finishes", () => {
    render(<AutoFollowHarness resetKey="run-a" />)
    const viewport = screen.getByTestId("viewport")
    setViewportMetrics(viewport, { clientHeight: 200, scrollHeight: 1_000, scrollTop: 100 })
    settleInitialFollow()
    viewport.scrollTop = 100
    fireEvent.scroll(viewport)
    fireEvent.click(screen.getByRole("button", { name: "Back to latest" }))
    const scrollTo = vi.mocked(viewport.scrollTo)
    scrollTo.mockClear()

    setViewportMetrics(viewport, { clientHeight: 200, scrollHeight: 1_200, scrollTop: 800 })
    act(() => activeObserver().trigger())
    flushAnimationFrame()
    expect(scrollTo).not.toHaveBeenCalled()

    fireEvent(viewport, new Event("scrollend"))
    flushAnimationFrame()
    expect(scrollTo).toHaveBeenCalledOnce()
    expect(scrollTo).toHaveBeenCalledWith({ top: 1_200, behavior: "auto" })
  })

  it("uses immediate motion for an explicit return when reduced motion is requested", () => {
    vi.mocked(matchMedia).mockReturnValue({ matches: true } as MediaQueryList)
    render(<AutoFollowHarness resetKey="run-a" />)
    const viewport = screen.getByTestId("viewport")
    setViewportMetrics(viewport, { clientHeight: 200, scrollHeight: 1_000, scrollTop: 100 })
    settleInitialFollow()
    viewport.scrollTop = 100
    fireEvent.scroll(viewport)
    fireEvent.click(screen.getByRole("button", { name: "Back to latest" }))
    expect(viewport.scrollTo).toHaveBeenLastCalledWith({ top: 1_000, behavior: "auto" })
  })

  it("resets follow state and detaches the old observer on a Run switch", () => {
    const view = render(<AutoFollowHarness resetKey="run-a" />)
    const viewport = screen.getByTestId("viewport")
    setViewportMetrics(viewport, { clientHeight: 200, scrollHeight: 1_000, scrollTop: 100 })
    settleInitialFollow()
    viewport.scrollTop = 100
    fireEvent.scroll(viewport)
    expect(screen.getByText("paused")).toBeInTheDocument()
    const oldObserver = activeObserver()

    view.rerender(<AutoFollowHarness resetKey="run-b" />)
    expect(screen.getByText("following")).toBeInTheDocument()
    expect(oldObserver.disconnected).toBe(true)
    expect(activeObserver()).not.toBe(oldObserver)
  })

  it("invalidates an old smooth timer and pending frame on a Run switch", () => {
    vi.useFakeTimers()
    const view = render(<AutoFollowHarness resetKey="run-a" />)
    const viewport = screen.getByTestId("viewport")
    setViewportMetrics(viewport, { clientHeight: 200, scrollHeight: 1_000, scrollTop: 100 })
    settleInitialFollow()
    viewport.scrollTop = 100
    fireEvent.scroll(viewport)
    fireEvent.click(screen.getByRole("button", { name: "Back to latest" }))
    const oldObserver = activeObserver()

    view.rerender(<AutoFollowHarness resetKey="run-b" />)
    const scrollTo = vi.mocked(viewport.scrollTo)
    act(() => vi.runOnlyPendingTimers())
    scrollTo.mockClear()
    act(() => {
      oldObserver.trigger()
      vi.advanceTimersByTime(2_000)
    })
    fireEvent(viewport, new Event("scrollend"))

    expect(screen.getByText("following")).toBeInTheDocument()
    expect(scrollTo).not.toHaveBeenCalled()
  })

  it("leaves one active observer and one scheduled follow after a StrictMode probe", () => {
    render(<StrictMode><AutoFollowHarness resetKey="run-a" /></StrictMode>)
    const viewport = screen.getByTestId("viewport")
    setViewportMetrics(viewport, { clientHeight: 200, scrollHeight: 600, scrollTop: 400 })
    expect(ControlledResizeObserver.instances.filter((observer) => !observer.disconnected)).toHaveLength(1)
    settleInitialFollow()
    vi.mocked(viewport.scrollTo).mockClear()

    act(() => activeObserver().trigger())
    flushAnimationFrame()
    expect(viewport.scrollTo).toHaveBeenCalledTimes(1)
  })
})
